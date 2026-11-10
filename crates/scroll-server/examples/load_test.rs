//! Opens many sync connections to a running server and drives a realistic mix of work: typing in pages, cell
//! edits, presence and switching pages. Reports latency percentiles, throughput and errors.
//!
//! cargo run --release -p scroll-server --example load_test -- --url https://scroll.example.com --invite CODE \
//!     [--connections 300] [--seconds 60] [--users 10] [--interval-ms 1000] [--open-database-percent 10] \
//!     [--server-pid PID]
//!
//! Every client edits cells; `--open-database-percent` of them also have the table open, as someone looking at it
//! would, and so receive its changes at once rather than batched.
//!
//! The invite is a sign-up invite (`scroll-server create-invite`). The tool signs up `--users` accounts (an owner
//! and members it invites) and shares the connections among them, which keeps it within the sign-up rate limit.
//! It refuses a server whose `SERVER_NAME` does not contain "test" unless given `--allow-any-server`.

mod support;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use scroll_api::protocol::{ClientMessage, ServerMessage};
use scroll_core::id::{DatabaseId, PageId, PropertyId, WorkspaceId};
use scroll_core::operation::{FieldChange, Operation, Record, Transaction};
use scroll_core::page_doc::{BlockKind, BlockPlacement, PageDoc};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue};
use scroll_server::test_support::tools::{MemoryWatch, argument, refuse_unless_test_target};
use support::{Accounts, Random, Session, connect, number_argument, percentiles, receive, send};
use tokio::task::JoinSet;
use tokio::time::sleep_until;
use uuid::Uuid;

const PAGES: usize = 50;
const ROWS: usize = 2_000;
const PROPERTIES: usize = 5;

/// What the clients share: the pages to type into and the cells to edit.
struct Workspace {
    id: WorkspaceId,
    database: DatabaseId,
    /// Keystroke-sized updates every client sends in turn. Made once, before the clients start: made by each
    /// client as it starts, they kept the load tool's own threads busy while the first measurements ran.
    typing: Vec<Vec<u8>>,
    pages: Vec<PageId>,
    rows: Vec<PageId>,
    properties: Vec<PropertyId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Typing,
    CellEdit,
    /// The first page a client opens, while every client is connecting at once.
    FirstOpenPage,
    /// Switching to another page later.
    OpenPage,
}

#[derive(Default)]
struct Results {
    latencies: HashMap<Kind, Vec<Duration>>,
    presence_sent: usize,
    messages_received: usize,
    rejections: usize,
    disconnects: usize,
}

fn page(title: String, row_of: Option<DatabaseId>) -> Page {
    Page {
        id: PageId::generate(),
        parent: None,
        title,
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of,
        is_archived: false,
    }
}

async fn create_workspace(accounts: &Accounts, owner: &Session, url: &str) -> Workspace {
    let id = accounts.create_workspace(owner, "Load test").await;
    let pages: Vec<Page> = (0..PAGES)
        .map(|index| page(format!("Page {index}"), None))
        .collect();
    let properties: Vec<Property> = (0..PROPERTIES)
        .map(|index| Property {
            id: PropertyId::generate(),
            name: format!("Column {index}"),
            property_type: PropertyType::Number,
            options: vec![],
        })
        .collect();
    let database = Database {
        id: DatabaseId::generate(),
        host_page: pages[0].id,
        properties: properties.clone(),
    };
    let rows: Vec<Page> = (0..ROWS)
        .map(|index| page(format!("Row {index}"), Some(database.id)))
        .collect();
    let mut operations: Vec<Operation> = pages
        .iter()
        .chain(&rows[..0])
        .map(|page| Operation::Create(Record::Page(page.clone())))
        .collect();
    operations.push(Operation::Create(Record::Database(database.clone())));
    for row in &rows {
        operations.push(Operation::Create(Record::Page(row.clone())));
    }
    let mut socket = connect(url, owner).await;
    send(
        &mut socket,
        &ClientMessage::OpenWorkspace {
            workspace_id: id,
            last_seen_seq: u64::MAX,
        },
    )
    .await;
    for chunk in operations.chunks(2_000) {
        let transaction = Transaction::new(chunk.to_vec());
        let transaction_id = transaction.id;
        send(
            &mut socket,
            &ClientMessage::PushTransaction {
                workspace_id: id,
                transaction,
            },
        )
        .await;
        loop {
            match receive(&mut socket).await {
                Some(ServerMessage::TransactionAck {
                    transaction_id: acked,
                    ..
                }) if acked == transaction_id => break,
                Some(ServerMessage::TransactionRejected { reason, .. }) => {
                    panic!("setup was rejected: {reason:?}")
                }
                Some(_) => {}
                None => panic!("the connection closed during setup"),
            }
        }
    }
    Workspace {
        id,
        database: database.id,
        typing: Vec::new(),
        pages: pages.iter().map(|page| page.id).collect(),
        rows: rows.iter().map(|row| row.id).collect(),
        properties: properties.iter().map(|property| property.id).collect(),
    }
}

/// Keystroke-sized Loro updates, made up front because a `PageDoc` cannot be held across an await.
fn typing_updates(count: usize) -> Vec<Vec<u8>> {
    let mut doc = PageDoc::new();
    let block = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    (0..count)
        .map(|step| {
            doc.insert_text(block, 0, if step % 5 == 4 { " " } else { "a" })
                .unwrap();
            doc.take_local_update().unwrap().unwrap()
        })
        .collect()
}

/// How one simulated person works.
#[derive(Clone, Copy)]
struct Habits {
    interval: Duration,
    deadline: Instant,
    has_table_open: bool,
}

/// One simulated person: opens the workspace and a page, then works until the deadline.
async fn run_client(
    url: Arc<str>,
    session: Arc<Session>,
    workspace: Arc<Workspace>,
    habits: Habits,
) -> Results {
    let Habits {
        interval,
        deadline,
        has_table_open,
    } = habits;
    let mut results = Results::default();
    let updates = &workspace.typing;
    let mut random = Random::new();
    let mut socket = connect(&url, &session).await;
    send(
        &mut socket,
        &ClientMessage::OpenWorkspace {
            workspace_id: workspace.id,
            last_seen_seq: u64::MAX,
        },
    )
    .await;
    if has_table_open {
        send(
            &mut socket,
            &ClientMessage::OpenDatabase {
                workspace_id: workspace.id,
                database_id: workspace.database,
            },
        )
        .await;
    }
    let mut page_id = workspace.pages[random.below(PAGES)];
    let mut pending: HashMap<Uuid, (Kind, Instant)> = HashMap::new();
    let mut open_started = Some((Kind::FirstOpenPage, Instant::now()));
    send(
        &mut socket,
        &ClientMessage::OpenPage {
            workspace_id: workspace.id,
            page_id,
            last_seen_seq: 0,
        },
    )
    .await;
    let mut next_update = 0;
    let mut next_action = tokio::time::Instant::now() + random.duration_around(interval);
    while Instant::now() < deadline {
        tokio::select! {
            message = receive(&mut socket) => {
                let Some(message) = message else {
                    results.disconnects += 1;
                    return results;
                };
                results.messages_received += 1;
                match message {
                    ServerMessage::PageUpdateAck { update_id, .. } => record(&mut results, pending.remove(&update_id)),
                    ServerMessage::TransactionAck { transaction_id, .. } => record(&mut results, pending.remove(&transaction_id.0)),
                    ServerMessage::PageCatchUp { page_id: caught_up, .. } if caught_up == page_id => {
                        if let Some((kind, started)) = open_started.take() {
                            results.latencies.entry(kind).or_default().push(started.elapsed());
                        }
                    }
                    ServerMessage::PageUpdateRejected { .. }
                    | ServerMessage::TransactionRejected { .. }
                    | ServerMessage::PageRejected { .. }
                    | ServerMessage::Rejected { .. } => results.rejections += 1,
                    _ => {}
                }
            }
            () = sleep_until(next_action) => {
                next_action = tokio::time::Instant::now() + random.duration_around(interval);
                match random.below(100) {
                    0..55 => {
                        let update_id = Uuid::now_v7();
                        let update = updates[next_update % updates.len()].clone();
                        next_update += 1;
                        pending.insert(update_id, (Kind::Typing, Instant::now()));
                        send(&mut socket, &ClientMessage::PushPageUpdate { update_id, page_id, update }).await;
                    }
                    55..75 => {
                        let edit = Operation::Set(FieldChange::PropertyValue {
                            page: workspace.rows[random.below(ROWS)],
                            property: workspace.properties[random.below(PROPERTIES)],
                            value: Some(PropertyValue::Number(random.below(1_000) as f64)),
                        });
                        let transaction = Transaction::new(vec![edit]);
                        pending.insert(transaction.id.0, (Kind::CellEdit, Instant::now()));
                        send(&mut socket, &ClientMessage::PushTransaction { workspace_id: workspace.id, transaction }).await;
                    }
                    75..95 => {
                        let state = format!("cursor {}", random.below(500)).into_bytes();
                        send(&mut socket, &ClientMessage::Presence { page_id, state }).await;
                        results.presence_sent += 1;
                    }
                    _ => {
                        send(&mut socket, &ClientMessage::ClosePage { page_id }).await;
                        page_id = workspace.pages[random.below(PAGES)];
                        open_started = Some((Kind::OpenPage, Instant::now()));
                        send(&mut socket, &ClientMessage::OpenPage { workspace_id: workspace.id, page_id, last_seen_seq: 0 }).await;
                    }
                }
            }
        }
    }
    results
}

fn record(results: &mut Results, sent: Option<(Kind, Instant)>) {
    if let Some((kind, started)) = sent {
        results
            .latencies
            .entry(kind)
            .or_default()
            .push(started.elapsed());
    }
}

#[tokio::main]
async fn main() {
    let url = argument("--url", "http://127.0.0.1:8080");
    let invite = argument("--invite", "");
    let connections = number_argument("--connections", 300);
    let seconds = number_argument("--seconds", 60);
    let user_count = number_argument("--users", 10).max(1);
    let interval = Duration::from_millis(number_argument("--interval-ms", 1_000) as u64);
    let open_database_percent = number_argument("--open-database-percent", 10);
    let server_pid: Option<u32> = argument("--server-pid", "").parse().ok();
    assert!(
        !invite.is_empty(),
        "--invite CODE is required (scroll-server create-invite)"
    );
    refuse_unless_test_target(&url, "").await;

    let accounts = Accounts::new(&url);
    let owner = accounts.sign_up(&invite).await;
    let mut workspace = create_workspace(&accounts, &owner, &url).await;
    workspace.typing = typing_updates((seconds + 1) * 4);
    let workspace = Arc::new(workspace);
    let mut users = vec![Arc::new(owner)];
    let owner = Arc::clone(&users[0]);
    for _ in 1..user_count {
        users.push(Arc::new(accounts.add_member(&owner, workspace.id).await));
    }

    let server_memory = server_pid.map(MemoryWatch::start);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds as u64);
    let url: Arc<str> = Arc::from(url.as_str());
    let mut clients = JoinSet::new();
    for index in 0..connections {
        let session = Arc::clone(&users[index % users.len()]);
        let habits = Habits {
            interval,
            deadline,
            has_table_open: index * 100 < open_database_percent * connections,
        };
        clients.spawn(run_client(
            Arc::clone(&url),
            session,
            Arc::clone(&workspace),
            habits,
        ));
    }
    let mut total = Results::default();
    while let Some(results) = clients.join_next().await {
        let results = results.expect("a client task finished");
        for (kind, mut latencies) in results.latencies {
            total
                .latencies
                .entry(kind)
                .or_default()
                .append(&mut latencies);
        }
        total.presence_sent += results.presence_sent;
        total.messages_received += results.messages_received;
        total.rejections += results.rejections;
        total.disconnects += results.disconnects;
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "{connections} connections ({open_database_percent}% with the table open), {user_count} users, one action per ~{interval:?} each, for {elapsed:.0} s"
    );
    for kind in [
        Kind::Typing,
        Kind::CellEdit,
        Kind::FirstOpenPage,
        Kind::OpenPage,
    ] {
        let mut latencies = total.latencies.remove(&kind).unwrap_or_default();
        let throughput = latencies.len() as f64 / elapsed;
        println!(
            "{} ({throughput:.0}/s)",
            percentiles(&format!("{kind:?}"), &mut latencies)
        );
    }
    println!(
        "presence sent: {:.0}/s; messages received: {:.0}/s; rejections: {}; disconnects: {}",
        total.presence_sent as f64 / elapsed,
        total.messages_received as f64 / elapsed,
        total.rejections,
        total.disconnects
    );
    if let Some(watch) = server_memory {
        println!("server peak memory: {:.0} MB", watch.peak_megabytes().await);
    }
}
