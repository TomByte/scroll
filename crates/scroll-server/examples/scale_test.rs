//! Measures one workspace at the target scale against a running server: publishing it, a device joining, edit
//! latency under load, a 10,000-row transaction and sustained transactions per second.
//!
//! cargo run --release -p scroll-server --example scale_test -- --url http://127.0.0.1:8080 --invite CODE \
//!     [--server-pid PID] [--database-url postgres://...] [--pages 100000] [--big-rows 500000] [--small-rows 500000] \
//!     [--writers 16,64]
//!
//! The invite is a sign-up invite (`scroll-server create-invite`). `--server-pid` adds the server's peak memory;
//! `--database-url` adds the longest workspace-lock hold and table sizes. It refuses a server whose `SERVER_NAME`
//! does not contain "test", and a database whose name does not, unless given `--allow-any-server`.

mod support;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use scroll_api::protocol::{CatchUpPart, ClientMessage, ServerMessage};
use scroll_core::id::{DatabaseId, PageId, PropertyId, WorkspaceId};
use scroll_core::operation::{FieldChange, Operation, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue};
use scroll_server::test_support::tools::{
    MemoryWatch, argument, refuse_unless_test_target, resident_megabytes,
};
use support::{Accounts, Random, Session, connect, number_argument, percentiles, receive, send};
use tokio::task::JoinSet;

/// The most operations the store puts in one publishing transaction (`PUBLISH_CHUNK_OPERATIONS`).
const CHUNK_OPERATIONS: usize = 2_000;
const CHUNKS_IN_FLIGHT: usize = 8;
const BIG_DATABASE_PROPERTIES: usize = 10;
const SMALL_DATABASES: usize = 4;
const SMALL_DATABASE_PROPERTIES: usize = 3;

struct Fixture {
    big_database: DatabaseId,
    big_rows: Vec<PageId>,
    big_properties: Vec<PropertyId>,
}

fn page(title: String, parent: Option<PageId>, row_of: Option<DatabaseId>) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title,
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of,
        is_archived: false,
    }
}

fn properties(count: usize) -> Vec<Property> {
    (0..count)
        .map(|index| Property {
            id: PropertyId::generate(),
            name: format!("Property {index}"),
            property_type: if index % 2 == 0 {
                PropertyType::Text
            } else {
                PropertyType::Number
            },
            options: vec![],
        })
        .collect()
}

fn value_for(property: &Property, row: usize) -> PropertyValue {
    match property.property_type {
        PropertyType::Number => PropertyValue::Number(row as f64),
        _ => PropertyValue::Text(format!("value {row}")),
    }
}

/// Every operation that builds the workspace, in an order validation accepts, made as they are consumed.
fn fixture_operations(
    pages: usize,
    big_rows: usize,
    small_rows: usize,
) -> (Fixture, Box<dyn Iterator<Item = Operation> + Send>) {
    let hosts: Vec<Page> = (0..=SMALL_DATABASES)
        .map(|index| page(format!("Database host {index}"), None, None))
        .collect();
    let big = Database {
        id: DatabaseId::generate(),
        host_page: hosts[0].id,
        properties: properties(BIG_DATABASE_PROPERTIES),
    };
    let smalls: Vec<Database> = hosts[1..]
        .iter()
        .map(|host| Database {
            id: DatabaseId::generate(),
            host_page: host.id,
            properties: properties(SMALL_DATABASE_PROPERTIES),
        })
        .collect();
    let big_rows_ids: Vec<PageId> = (0..big_rows).map(|_| PageId::generate()).collect();
    let fixture = Fixture {
        big_database: big.id,
        big_rows: big_rows_ids.clone(),
        big_properties: big.properties.iter().map(|property| property.id).collect(),
    };
    let plain_pages = (0..pages.saturating_sub(hosts.len()))
        .map(|index| Operation::Create(Record::Page(page(format!("Page {index}"), None, None))));
    let host_pages = hosts
        .clone()
        .into_iter()
        .map(|host| Operation::Create(Record::Page(host)));
    let databases = std::iter::once(big.clone())
        .chain(smalls.clone())
        .map(|database| Operation::Create(Record::Database(database)));
    let big_host = hosts[0].id;
    let big_for_rows = big.clone();
    let big_row_operations = big_rows_ids
        .into_iter()
        .enumerate()
        .flat_map(move |(index, id)| rows_operations(&big_for_rows, big_host, (index, id)));
    let per_small = small_rows / SMALL_DATABASES;
    let small_row_operations =
        smalls
            .into_iter()
            .zip(hosts.into_iter().skip(1))
            .flat_map(move |(database, host)| {
                (0..per_small).flat_map(move |index| {
                    rows_operations(&database, host.id, (index, PageId::generate()))
                })
            });
    let operations = plain_pages
        .chain(host_pages)
        .chain(databases)
        .chain(big_row_operations)
        .chain(small_row_operations);
    (fixture, Box::new(operations))
}

/// A row, with a value for every property.
fn rows_operations(
    database: &Database,
    host: PageId,
    (index, id): (usize, PageId),
) -> Vec<Operation> {
    let row = Page {
        id,
        ..page(format!("Row {index}"), Some(host), Some(database.id))
    };
    std::iter::once(Operation::Create(Record::Page(row)))
        .chain(database.properties.iter().map(|property| {
            Operation::Set(FieldChange::PropertyValue {
                page: id,
                property: property.id,
                value: Some(value_for(property, index)),
            })
        }))
        .collect()
}

fn set_value(fixture: &Fixture, row: usize, property: usize, value: f64) -> Operation {
    Operation::Set(FieldChange::PropertyValue {
        page: fixture.big_rows[row],
        property: fixture.big_properties[property],
        value: Some(PropertyValue::Number(value)),
    })
}

/// Commits one transaction and waits for its ack, ignoring other traffic. Returns the round trip.
async fn commit(
    socket: &mut support::Socket,
    workspace_id: WorkspaceId,
    transaction: Transaction,
) -> Duration {
    let id = transaction.id;
    let started = Instant::now();
    send(
        socket,
        &ClientMessage::PushTransaction {
            workspace_id,
            transaction,
        },
    )
    .await;
    loop {
        match receive(socket).await {
            Some(ServerMessage::TransactionAck { transaction_id, .. }) if transaction_id == id => {
                return started.elapsed();
            }
            Some(ServerMessage::TransactionRejected { reason, .. }) => {
                panic!("rejected: {reason:?}")
            }
            Some(_) => {}
            None => panic!("the connection closed"),
        }
    }
}

/// Opens the workspace for live transactions only, skipping any catch-up.
async fn subscribe(url: &str, session: &Session, workspace_id: WorkspaceId) -> support::Socket {
    let mut socket = connect(url, session).await;
    send(
        &mut socket,
        &ClientMessage::OpenWorkspace {
            workspace_id,
            last_seen_seq: u64::MAX,
        },
    )
    .await;
    loop {
        if let Some(ServerMessage::WorkspaceCatchUpPart { is_last: true, .. }) =
            receive(&mut socket).await
        {
            return socket;
        }
    }
}

/// Publishes the fixture in chunks with a window in flight, as the sync engine does.
/// Returns the number of chunks and the range of sequence numbers they were committed at.
async fn publish(
    socket: &mut support::Socket,
    workspace_id: WorkspaceId,
    mut operations: Box<dyn Iterator<Item = Operation> + Send>,
) -> (usize, (u64, u64)) {
    let mut in_flight = VecDeque::new();
    let mut chunks = 0;
    let mut seqs = (u64::MAX, 0);
    loop {
        while in_flight.len() < CHUNKS_IN_FLIGHT {
            let chunk: Vec<Operation> = operations.by_ref().take(CHUNK_OPERATIONS).collect();
            if chunk.is_empty() {
                break;
            }
            let transaction = Transaction::new(chunk);
            in_flight.push_back(transaction.id);
            send(
                socket,
                &ClientMessage::PushTransaction {
                    workspace_id,
                    transaction,
                },
            )
            .await;
            chunks += 1;
        }
        if in_flight.is_empty() {
            return (chunks, seqs);
        }
        match receive(socket).await {
            Some(ServerMessage::TransactionAck {
                transaction_id,
                seq,
                ..
            }) => {
                seqs = (seqs.0.min(seq), seqs.1.max(seq));
                in_flight.retain(|id| *id != transaction_id);
                if chunks % 250 == 0 && in_flight.len() == CHUNKS_IN_FLIGHT - 1 {
                    eprintln!("  {chunks} chunks sent");
                }
            }
            Some(ServerMessage::TransactionRejected { reason, .. }) => {
                panic!("a chunk was rejected: {reason:?}")
            }
            Some(_) => {}
            None => panic!("the connection closed during the publish"),
        }
    }
}

/// Commits a one-page transaction every 200 ms until told to stop, returning each round trip.
fn keep_editing(
    url: String,
    session: Session,
    workspace_id: WorkspaceId,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<Vec<Duration>>,
) {
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let mut socket = subscribe(&url, &session, workspace_id).await;
        let mut latencies = Vec::new();
        loop {
            let created = Operation::Create(Record::Page(page("During".into(), None, None)));
            latencies
                .push(commit(&mut socket, workspace_id, Transaction::new(vec![created])).await);
            tokio::select! {
                _ = &mut stopped => return latencies,
                () = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
        }
    });
    (stop, task)
}

/// Joins as a new device, keeping every part in memory. Returns (first part, total, parts, bytes).
async fn join(
    url: &str,
    session: &Session,
    workspace_id: WorkspaceId,
) -> (Duration, Duration, usize, usize) {
    let mut socket = connect(url, session).await;
    let started = Instant::now();
    send(
        &mut socket,
        &ClientMessage::OpenWorkspace {
            workspace_id,
            last_seen_seq: 0,
        },
    )
    .await;
    let mut first_part = None;
    let mut parts = 0;
    let mut bytes = 0;
    let mut records = Vec::new();
    let mut values = Vec::new();
    loop {
        let Some(ServerMessage::WorkspaceCatchUpPart { is_last, part, .. }) =
            receive(&mut socket).await
        else {
            continue;
        };
        first_part.get_or_insert_with(|| started.elapsed());
        parts += 1;
        bytes += scroll_api::protocol::encode(&part).len();
        if let CatchUpPart::Snapshot(snapshot) = part {
            records.extend(snapshot.records);
            values.extend(snapshot.property_values);
        }
        if is_last {
            eprintln!(
                "  joined with {} records and {} values",
                records.len(),
                values.len()
            );
            return (
                first_part.unwrap_or_default(),
                started.elapsed(),
                parts,
                bytes,
            );
        }
    }
}

/// The longest gap between the starts of consecutive commits while the publish kept the server busy: commits
/// queue for the workspace lock back to back, so no single commit held it longer than this.
async fn longest_lock_and_sizes(
    database_url: &str,
    workspace_id: WorkspaceId,
    publish_seqs: (u64, u64),
) {
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls)
        .await
        .expect("the database is reachable");
    tokio::spawn(connection);
    let gap: f64 = client
        .query_one(
            "SELECT coalesce(max(gap), 0) FROM (
                SELECT extract(epoch FROM created_at - lag(created_at) OVER (ORDER BY seq))::float8 AS gap
                FROM record_transactions WHERE workspace_id = $1 AND seq BETWEEN $2 AND $3) AS gaps",
            &[&workspace_id.0, &(publish_seqs.0 as i64), &(publish_seqs.1 as i64)],
        )
        .await
        .unwrap()
        .get(0);
    println!(
        "longest workspace-lock hold during the publish (upper bound): {:.0} ms",
        gap * 1000.0
    );
    let rows = client
        .query(
            "SELECT relname, pg_size_pretty(pg_relation_size(c.oid)) AS heap,
                    pg_size_pretty(pg_indexes_size(c.oid)) AS indexes, reltuples::bigint AS rows
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public' AND c.relkind = 'r' ORDER BY pg_total_relation_size(c.oid) DESC",
            &[],
        )
        .await
        .unwrap();
    println!("table sizes (heap / indexes / rows):");
    for row in rows {
        let name: String = row.get("relname");
        let heap: String = row.get("heap");
        let indexes: String = row.get("indexes");
        let count: i64 = row.get("rows");
        println!("  {name}: {heap} / {indexes} / {count}");
    }
}

/// Single-change transactions per second from `writers` connections, each waiting for its ack before the next,
/// over 10 seconds.
async fn sustained_rate(
    url: &str,
    session: &Session,
    workspace_id: WorkspaceId,
    (fixture, writers): (&Fixture, usize),
) -> f64 {
    let mut tasks = JoinSet::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    for _ in 0..writers {
        let mut socket = subscribe(url, session, workspace_id).await;
        let rows = fixture.big_rows.clone();
        let property = fixture.big_properties[7];
        tasks.spawn(async move {
            let mut random = Random::new();
            let mut count = 0usize;
            while Instant::now() < deadline {
                let edit = Operation::Set(FieldChange::PropertyValue {
                    page: rows[random.below(rows.len())],
                    property,
                    value: Some(PropertyValue::Number(count as f64)),
                });
                commit(&mut socket, workspace_id, Transaction::new(vec![edit])).await;
                count += 1;
            }
            count
        });
    }
    let total: usize = tasks.join_all().await.into_iter().sum();
    total as f64 / 10.0
}

#[tokio::main]
async fn main() {
    let url = argument("--url", "http://127.0.0.1:8080");
    let invite = argument("--invite", "");
    let server_pid: Option<u32> = argument("--server-pid", "").parse().ok();
    let database_url = argument("--database-url", "");
    let pages = number_argument("--pages", 100_000);
    let big_rows = number_argument("--big-rows", 500_000);
    let small_rows = number_argument("--small-rows", 500_000);
    assert!(
        !invite.is_empty(),
        "--invite CODE is required (scroll-server create-invite)"
    );
    refuse_unless_test_target(&url, &database_url).await;

    let accounts = Accounts::new(&url);
    let owner = accounts.sign_up(&invite).await;
    let workspace_id = accounts.create_workspace(&owner, "Scale").await;
    let editor = accounts.add_member(&owner, workspace_id).await;
    let idle_memory = server_pid.and_then(resident_megabytes);

    // Publish, with another member editing throughout.
    let (fixture, operations) = fixture_operations(pages, big_rows, small_rows);
    let (stop_editing, editing) = keep_editing(
        url.clone(),
        Session {
            user_id: editor.user_id,
            token: editor.token.clone(),
        },
        workspace_id,
    );
    let server_memory = server_pid.map(MemoryWatch::start);
    let mut publisher = subscribe(&url, &owner, workspace_id).await;
    let started = Instant::now();
    let (chunks, publish_seqs) = publish(&mut publisher, workspace_id, operations).await;
    let publish_time = started.elapsed();
    let _ = stop_editing.send(());
    let mut editor_latencies = editing.await.unwrap();
    println!(
        "publish: {chunks} chunks of up to {CHUNK_OPERATIONS} operations in {publish_time:.1?}"
    );
    println!(
        "{}",
        percentiles(
            "  another member's commits meanwhile",
            &mut editor_latencies
        )
    );
    if let Some(watch) = server_memory {
        println!(
            "  server peak memory during the publish: {:.0} MB (idle before: {:.0} MB)",
            watch.peak_megabytes().await,
            idle_memory.unwrap_or(0.0)
        );
    }

    // A second device joins.
    let server_memory = server_pid.map(MemoryWatch::start);
    let client_memory = MemoryWatch::start(std::process::id());
    let (first_part, join_time, parts, bytes) = join(&url, &owner, workspace_id).await;
    println!(
        "join: first data after {first_part:.1?}, complete after {join_time:.1?}, {parts} parts, {:.0} MB",
        bytes as f64 / 1_048_576.0
    );
    println!(
        "  client peak memory (whole snapshot kept): {:.0} MB",
        client_memory.peak_megabytes().await
    );
    if let Some(watch) = server_memory {
        println!(
            "  server peak memory during the join: {:.0} MB",
            watch.peak_megabytes().await
        );
    }

    // A single cell edit while 50 others look at the table and one client bulk-edits it.
    let mut listeners = JoinSet::new();
    for _ in 0..50 {
        let mut socket = subscribe(&url, &editor, workspace_id).await;
        let open = ClientMessage::OpenDatabase {
            workspace_id,
            database_id: fixture.big_database,
        };
        send(&mut socket, &open).await;
        listeners.spawn(async move {
            let mut socket = socket;
            while receive(&mut socket).await.is_some() {}
        });
    }
    let (stop_bulk, mut stopped_bulk) = tokio::sync::oneshot::channel::<()>();
    let bulk_rows = fixture.big_rows.clone();
    let bulk_properties = fixture.big_properties.clone();
    let bulk_session = Session {
        user_id: owner.user_id,
        token: owner.token.clone(),
    };
    let bulk_url = url.clone();
    let bulk = tokio::spawn(async move {
        let mut socket = subscribe(&bulk_url, &bulk_session, workspace_id).await;
        let mut random = Random::new();
        let mut transactions = 0;
        loop {
            let operations = (0..CHUNK_OPERATIONS)
                .map(|_| {
                    Operation::Set(FieldChange::PropertyValue {
                        page: bulk_rows[random.below(bulk_rows.len())],
                        property: bulk_properties[1],
                        value: Some(PropertyValue::Number(random.below(1000) as f64)),
                    })
                })
                .collect();
            commit(&mut socket, workspace_id, Transaction::new(operations)).await;
            transactions += 1;
            if stopped_bulk.try_recv().is_ok() {
                return transactions;
            }
        }
    });
    let mut cell_editor = subscribe(&url, &editor, workspace_id).await;
    let mut random = Random::new();
    for has_table_open in [false, true] {
        if has_table_open {
            let open = ClientMessage::OpenDatabase {
                workspace_id,
                database_id: fixture.big_database,
            };
            send(&mut cell_editor, &open).await;
        }
        let mut cell_latencies = Vec::new();
        for _ in 0..200 {
            let edit = set_value(
                &fixture,
                random.below(fixture.big_rows.len()),
                3,
                random.below(100) as f64,
            );
            let transaction = Transaction::new(vec![edit]);
            cell_latencies.push(commit(&mut cell_editor, workspace_id, transaction).await);
        }
        let label = if has_table_open {
            "cell edit by someone with the table open, 50 listeners on it and a bulk editor"
        } else {
            "cell edit by someone without the table open, 50 listeners on it and a bulk editor"
        };
        println!("{}", percentiles(label, &mut cell_latencies));
    }
    let _ = stop_bulk.send(());
    let bulk_transactions = bulk.await.unwrap();
    println!(
        "  the bulk editor committed {bulk_transactions} transactions of {CHUNK_OPERATIONS} value changes meanwhile"
    );
    listeners.abort_all();

    // One transaction touching 10,000 rows.
    let operations = (0..10_000)
        .map(|row| set_value(&fixture, row, 5, 1.0))
        .collect();
    let wide = commit(&mut cell_editor, workspace_id, Transaction::new(operations)).await;
    println!("a transaction changing 10,000 rows: {wide:.1?}");

    for writers in argument("--writers", "16,64").split(',') {
        let writers = writers
            .parse()
            .expect("--writers is a list of whole numbers");
        let rate = sustained_rate(&url, &editor, workspace_id, (&fixture, writers)).await;
        println!("sustained: {rate:.0} transactions per second from {writers} writers");
    }
    let _ = fixture.big_database;

    if let Some(megabytes) = server_pid.and_then(resident_megabytes) {
        println!("server memory at the end: {megabytes:.0} MB");
    }
    if !database_url.is_empty() {
        longest_lock_and_sizes(&database_url, workspace_id, publish_seqs).await;
    }
}
