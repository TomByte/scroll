//! Who receives committed transactions when, and committing many at once.

mod common;

use std::time::Duration;

use common::{SyncClient, TestServer};
use scroll_api::protocol::{ClientMessage, RejectReason, ServerMessage, TransactionRejection};
use scroll_core::id::{DatabaseId, PageId, PropertyId, WorkspaceId};
use scroll_core::operation::{FieldChange, Operation, PageField, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue};
use scroll_core::validation::ValidationError;
use scroll_server::Settings;

/// Long enough that nothing held back arrives during a test unless something else brings it.
const NEVER: Duration = Duration::from_secs(600);
const PROMPTLY: Duration = Duration::from_millis(500);

fn page(title: &str, row_of: Option<DatabaseId>) -> Page {
    Page {
        id: PageId::generate(),
        parent: None,
        title: title.to_owned(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of,
        is_archived: false,
    }
}

struct Table {
    host: PageId,
    database: DatabaseId,
    property: PropertyId,
    row: PageId,
}

async fn create_table(client: &mut SyncClient, workspace_id: WorkspaceId) -> Table {
    let host = page("Tasks", None);
    let property = Property {
        id: PropertyId::generate(),
        name: "Estimate".into(),
        property_type: PropertyType::Number,
        options: vec![],
    };
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![property.clone()],
    };
    let row = page("Row", Some(database.id));
    let table = Table {
        host: host.id,
        database: database.id,
        property: property.id,
        row: row.id,
    };
    let setup = Transaction::new(vec![
        Operation::Create(Record::Page(host)),
        Operation::Create(Record::Database(database)),
        Operation::Create(Record::Page(row)),
    ]);
    client.commit(workspace_id, &setup).await;
    table
}

fn set_estimate(table: &Table, estimate: f64) -> Transaction {
    Transaction::new(vec![Operation::Set(FieldChange::PropertyValue {
        page: table.row,
        property: table.property,
        value: Some(PropertyValue::Number(estimate)),
    })])
}

async fn receive_remote_seq(client: &mut SyncClient) -> u64 {
    match client.receive().await {
        ServerMessage::RemoteTransaction { seq, .. } => seq,
        other => panic!("expected a remote transaction, got {other:?}"),
    }
}

async fn receive_remote_seq_promptly(client: &mut SyncClient) -> u64 {
    tokio::time::timeout(PROMPTLY, receive_remote_seq(client))
        .await
        .expect("the transaction arrives at once")
}

#[tokio::test]
async fn row_changes_go_at_once_only_to_connections_with_the_database_open_and_to_others_in_order_later()
 {
    let server = TestServer::start_with(Settings {
        deferred_delivery: NEVER,
        ..TestServer::test_settings()
    })
    .await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut writer = server.connect(&owner).await;
    writer.open_workspace(workspace_id, 0).await;
    let table = create_table(&mut writer, workspace_id).await;
    let mut looking = server.connect(&owner).await;
    looking.open_workspace(workspace_id, 0).await;
    let open = ClientMessage::OpenDatabase {
        workspace_id,
        database_id: table.database,
    };
    looking.send(&open).await;
    round_trip(&mut looking, workspace_id).await;
    let mut elsewhere = server.connect(&owner).await;
    elsewhere.open_workspace(workspace_id, 0).await;

    let first = writer
        .commit(workspace_id, &set_estimate(&table, 1.0))
        .await;
    assert_eq!(receive_remote_seq_promptly(&mut looking).await, first);
    elsewhere.assert_silent().await;

    elsewhere.send(&open).await;
    assert_eq!(
        receive_remote_seq_promptly(&mut elsewhere).await,
        first,
        "opening delivers what was held back"
    );
    let second = writer
        .commit(workspace_id, &set_estimate(&table, 2.0))
        .await;
    assert_eq!(receive_remote_seq_promptly(&mut elsewhere).await, second);

    let close = ClientMessage::CloseDatabase {
        workspace_id,
        database_id: table.database,
    };
    elsewhere.send(&close).await;
    round_trip(&mut elsewhere, workspace_id).await;
    let third = writer
        .commit(workspace_id, &set_estimate(&table, 3.0))
        .await;
    elsewhere.assert_silent().await;
    let rename = Transaction::new(vec![Operation::Set(FieldChange::Page {
        id: table.host,
        field: PageField::Title("Renamed".into()),
    })]);
    let fourth = writer.commit(workspace_id, &rename).await;
    assert_eq!(
        receive_remote_seq_promptly(&mut elsewhere).await,
        third,
        "a change everyone needs at once brings what was held back first"
    );
    assert_eq!(receive_remote_seq_promptly(&mut elsewhere).await, fourth);
}

/// The server handles a connection's messages in order, so once this answer arrives it has handled the ones
/// sent before.
async fn round_trip(client: &mut SyncClient, workspace_id: WorkspaceId) {
    client.open_page(workspace_id, PageId::generate(), 0).await;
}

#[tokio::test]
async fn opening_a_database_needs_the_workspace_open() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut client = server.connect(&owner).await;
    client
        .send(&ClientMessage::OpenDatabase {
            workspace_id,
            database_id: DatabaseId::generate(),
        })
        .await;
    assert_eq!(
        client.receive().await,
        ServerMessage::WorkspaceRejected {
            workspace_id,
            reason: RejectReason::WorkspaceNotOpen
        }
    );
}

/// What one writer saw: the seqs of its acks, and how many of its transactions were rejected as invalid.
#[derive(Default)]
struct Answers {
    acked: Vec<u64>,
    invalid: usize,
}

#[tokio::test]
async fn transactions_committed_together_are_each_all_or_nothing_with_gapless_seqs() {
    const WRITERS: usize = 20;
    const PER_WRITER: usize = 10;
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let elsewhere = server.create_workspace(&owner).await;
    let mut other_client = server.connect(&owner).await;
    other_client.open_workspace(elsewhere, 0).await;
    let taken = page("Elsewhere", None);
    let create_taken = Transaction::new(vec![Operation::Create(Record::Page(taken.clone()))]);
    other_client.commit(elsewhere, &create_taken).await;
    let mut tasks = Vec::new();
    for _ in 0..WRITERS {
        let taken = taken.clone();
        let mut client = server.connect(&owner).await;
        client.open_workspace(workspace_id, 0).await;
        tasks.push(tokio::spawn(async move {
            let mut transactions = Vec::new();
            for index in 0..PER_WRITER {
                let valid = Operation::Create(Record::Page(page("Valid", None)));
                // Creates a page, then fails on a missing one: the page must not survive.
                let invalid = vec![
                    Operation::Create(Record::Page(page("Rolled back", None))),
                    Operation::Set(FieldChange::Page {
                        id: PageId::generate(),
                        field: PageField::Title("Missing".into()),
                    }),
                ];
                // Only found out when the group's new pages are written.
                let taken_id = vec![Operation::Create(Record::Page(Page {
                    title: "Taken".into(),
                    ..taken.clone()
                }))];
                let operations = match index % 4 {
                    0 | 2 => vec![valid],
                    1 => invalid,
                    _ => taken_id,
                };
                transactions.push(Transaction::new(operations));
            }
            for transaction in &transactions {
                client.send_transaction(workspace_id, transaction).await;
            }
            let mut answers = Answers::default();
            while answers.acked.len() + answers.invalid < PER_WRITER {
                match client.receive().await {
                    ServerMessage::TransactionAck { seq, .. } => answers.acked.push(seq),
                    ServerMessage::TransactionRejected {
                        reason:
                            TransactionRejection::Invalid(
                                ValidationError::PageNotFound(_)
                                | ValidationError::RecordAlreadyExists(_),
                            ),
                        ..
                    } => answers.invalid += 1,
                    ServerMessage::RemoteTransaction { .. } => {}
                    other => panic!("unexpected {other:?}"),
                }
            }
            answers
        }));
    }
    let mut acked = Vec::new();
    for task in tasks {
        let answers = task.await.unwrap();
        assert_eq!(answers.invalid, PER_WRITER / 2);
        acked.extend(answers.acked);
    }
    acked.sort_unstable();
    let committed = (WRITERS * PER_WRITER / 2) as u64;
    assert_eq!(acked, (1..=committed).collect::<Vec<_>>());

    let mut reader = server.connect(&owner).await;
    let snapshot = reader
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(snapshot.seq, committed);
    assert!(snapshot.records.iter().all(|record| match record {
        Record::Page(page) => page.title == "Valid",
        _ => false,
    }));
    assert_eq!(snapshot.records.len(), committed as usize);
}

#[tokio::test]
async fn the_same_transaction_pushed_by_two_connections_at_once_commits_once() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let transaction = Transaction::new(vec![Operation::Create(Record::Page(page("Once", None)))]);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let mut client = server.connect(&owner).await;
        client.open_workspace(workspace_id, 0).await;
        let transaction = transaction.clone();
        tasks.push(tokio::spawn(async move {
            client.send_transaction(workspace_id, &transaction).await;
            loop {
                match client.receive().await {
                    ServerMessage::TransactionAck { seq, .. } => return seq,
                    ServerMessage::RemoteTransaction { .. } => {}
                    other => panic!("unexpected {other:?}"),
                }
            }
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), 1);
    }
}
