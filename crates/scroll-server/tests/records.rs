//! Record sync: transactions validated, committed in order, relayed and caught up.

mod common;

use common::TestServer;
use scroll_api::protocol::{ClientMessage, RejectReason, ServerMessage, TransactionRejection};
use scroll_core::id::{DatabaseId, PageId, PropertyId, ViewId, WorkspaceId};
use scroll_core::operation::{FieldChange, Operation, PageField, Record, RecordId, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue, View, ViewType};
use scroll_core::sync::{
    PropertyValueEntry, SequencedTransaction, WorkspaceCatchUp, WorkspaceSnapshot,
};
use scroll_core::validation::ValidationError;

fn new_page(title: &str, parent: Option<PageId>) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: title.to_owned(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    }
}

fn create(record: Record) -> Transaction {
    Transaction::new(vec![Operation::Create(record)])
}

fn create_page(page: &Page) -> Transaction {
    create(Record::Page(page.clone()))
}

fn set_page(id: PageId, field: PageField) -> Transaction {
    Transaction::new(vec![Operation::Set(FieldChange::Page { id, field })])
}

fn snapshot_pages(snapshot: &WorkspaceSnapshot) -> Vec<Page> {
    let mut pages: Vec<Page> = snapshot
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Page(page) => Some(page.clone()),
            _ => None,
        })
        .collect();
    pages.sort_by_key(|page| page.id);
    pages
}

#[tokio::test]
async fn two_clients_converge_on_records() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let mut owner_client = server.connect(&owner).await;
    let mut editor_client = server.connect(&editor).await;
    assert_eq!(
        owner_client.open_workspace(workspace_id, 0).await,
        WorkspaceCatchUp {
            snapshot: None,
            transactions: vec![]
        },
        "with nothing committed, the empty log rather than an empty snapshot"
    );
    editor_client.open_workspace(workspace_id, 0).await;
    let mut page = new_page("Draft", None);

    let created = create_page(&page);
    assert_eq!(owner_client.commit(workspace_id, &created).await, 1);
    assert_eq!(
        editor_client.receive().await,
        ServerMessage::RemoteTransaction {
            workspace_id,
            seq: 1,
            author: owner.user_id,
            transaction: created
        }
    );

    let renamed = set_page(page.id, PageField::Title("Plan".into()));
    assert_eq!(editor_client.commit(workspace_id, &renamed).await, 2);
    assert_eq!(
        owner_client.receive().await,
        ServerMessage::RemoteTransaction {
            workspace_id,
            seq: 2,
            author: editor.user_id,
            transaction: renamed
        }
    );

    page.title = "Plan".into();
    let mut newcomer = server.connect(&owner).await;
    let snapshot = newcomer
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(snapshot.seq, 2);
    assert_eq!(snapshot_pages(&snapshot), vec![page]);
}

#[tokio::test]
async fn catch_up_is_a_snapshot_from_zero_and_the_log_after_a_seen_seq() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut writer = server.connect(&owner).await;
    writer.open_workspace(workspace_id, 0).await;
    let page = new_page("One", None);
    let transactions = [
        create_page(&page),
        set_page(page.id, PageField::Title("Two".into())),
        set_page(page.id, PageField::IsArchived(true)),
    ];
    for transaction in &transactions {
        writer.commit(workspace_id, transaction).await;
    }
    let mut reader = server.connect(&owner).await;

    let from_log = reader.open_workspace(workspace_id, 1).await;
    assert_eq!(from_log.snapshot, None);
    assert_eq!(
        from_log.transactions,
        vec![
            SequencedTransaction {
                seq: 2,
                transaction: transactions[1].clone()
            },
            SequencedTransaction {
                seq: 3,
                transaction: transactions[2].clone()
            },
        ]
    );

    let from_zero = reader.open_workspace(workspace_id, 0).await;
    assert!(from_zero.transactions.is_empty());
    let snapshot = from_zero.snapshot.unwrap();
    assert_eq!(snapshot.seq, 3);
    let expected = Page {
        title: "Two".into(),
        is_archived: true,
        ..page
    };
    assert_eq!(snapshot_pages(&snapshot), vec![expected]);

    let up_to_date = reader.open_workspace(workspace_id, 3).await;
    assert_eq!(up_to_date.snapshot, None);
    assert!(up_to_date.transactions.is_empty());
}

#[tokio::test]
async fn an_invalid_transaction_is_rejected_whole_and_leaves_no_trace() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut writer = server.connect(&owner).await;
    let mut watcher = server.connect(&owner).await;
    writer.open_workspace(workspace_id, 0).await;
    watcher.open_workspace(workspace_id, 0).await;
    let parent = new_page("Parent", None);
    let child = new_page("Child", Some(parent.id));
    writer.commit(workspace_id, &create_page(&parent)).await;
    writer.commit(workspace_id, &create_page(&child)).await;
    for _ in 0..2 {
        assert!(matches!(
            watcher.receive().await,
            ServerMessage::RemoteTransaction { .. }
        ));
    }

    let own_ancestor = set_page(parent.id, PageField::Parent(Some(child.id)));
    assert_eq!(
        writer.commit_rejected(workspace_id, &own_ancestor).await,
        TransactionRejection::Invalid(ValidationError::PageWouldBeItsOwnAncestor {
            page: parent.id,
            parent: child.id
        })
    );
    // The first operation is valid, but the second is not, so neither happens.
    let unrelated = new_page("Unrelated", None);
    let half_valid = Transaction::new(vec![
        Operation::Create(Record::Page(unrelated.clone())),
        Operation::Set(FieldChange::Page {
            id: parent.id,
            field: PageField::Parent(Some(parent.id)),
        }),
    ]);
    assert_eq!(
        writer.commit_rejected(workspace_id, &half_valid).await,
        TransactionRejection::Invalid(ValidationError::PageWouldBeItsOwnAncestor {
            page: parent.id,
            parent: parent.id
        })
    );
    // A move earlier in the same transaction counts: the unrelated page goes under the child, so the parent
    // cannot then go under it.
    let made_cycle = Transaction::new(vec![
        Operation::Create(Record::Page(unrelated.clone())),
        Operation::Set(FieldChange::Page {
            id: unrelated.id,
            field: PageField::Parent(Some(child.id)),
        }),
        Operation::Set(FieldChange::Page {
            id: parent.id,
            field: PageField::Parent(Some(unrelated.id)),
        }),
    ]);
    assert_eq!(
        writer.commit_rejected(workspace_id, &made_cycle).await,
        TransactionRejection::Invalid(ValidationError::PageWouldBeItsOwnAncestor {
            page: parent.id,
            parent: unrelated.id
        })
    );
    watcher.assert_silent().await;

    // No sequence number was used, and the state is as before.
    assert_eq!(
        writer.commit(workspace_id, &create_page(&unrelated)).await,
        3
    );
    assert!(matches!(
        watcher.receive().await,
        ServerMessage::RemoteTransaction { seq: 3, .. }
    ));
    let snapshot = watcher
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    let mut expected = vec![parent, child, unrelated];
    expected.sort_by_key(|page| page.id);
    assert_eq!(snapshot_pages(&snapshot), expected);
    let log = watcher.open_workspace(workspace_id, 1).await.transactions;
    assert_eq!(
        log.iter().map(|logged| logged.seq).collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[tokio::test]
async fn a_resent_transaction_gets_its_original_ack_and_is_not_applied_again() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut writer = server.connect(&owner).await;
    let mut watcher = server.connect(&owner).await;
    writer.open_workspace(workspace_id, 0).await;
    watcher.open_workspace(workspace_id, 0).await;
    let created = create_page(&new_page("Once", None));

    assert_eq!(writer.commit(workspace_id, &created).await, 1);
    assert!(matches!(
        watcher.receive().await,
        ServerMessage::RemoteTransaction { seq: 1, .. }
    ));
    assert_eq!(writer.commit(workspace_id, &created).await, 1);
    watcher.assert_silent().await;

    writer.close().await;
    let mut reconnected = server.connect(&owner).await;
    reconnected.open_workspace(workspace_id, 1).await;
    assert_eq!(reconnected.commit(workspace_id, &created).await, 1);
    watcher.assert_silent().await;
    assert!(
        reconnected
            .open_workspace(workspace_id, 0)
            .await
            .snapshot
            .is_some_and(|snapshot| snapshot.seq == 1)
    );
}

#[tokio::test]
async fn viewers_non_members_and_unopened_workspaces_cannot_push() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let viewer = server.new_member(&owner, workspace_id, "Viewer").await;
    let outsider = server.new_user().await;
    let created = create_page(&new_page("Nope", None));

    let mut viewer_client = server.connect(&viewer).await;
    viewer_client.open_workspace(workspace_id, 0).await;
    assert_eq!(
        viewer_client.commit_rejected(workspace_id, &created).await,
        TransactionRejection::Refused(RejectReason::ReadOnly)
    );

    let mut outsider_client = server.connect(&outsider).await;
    outsider_client
        .send(&ClientMessage::OpenWorkspace {
            workspace_id,
            last_seen_seq: 0,
        })
        .await;
    assert_eq!(
        outsider_client.receive().await,
        ServerMessage::WorkspaceRejected {
            workspace_id,
            reason: RejectReason::NotAMember
        }
    );
    assert_eq!(
        outsider_client
            .commit_rejected(workspace_id, &created)
            .await,
        TransactionRejection::Refused(RejectReason::WorkspaceNotOpen)
    );

    let mut owner_client = server.connect(&owner).await;
    assert_eq!(
        owner_client.commit_rejected(workspace_id, &created).await,
        TransactionRejection::Refused(RejectReason::WorkspaceNotOpen)
    );
    let catch_up = owner_client.open_workspace(workspace_id, 0).await;
    assert_eq!(catch_up.transactions, vec![], "nothing was committed");
}

#[tokio::test]
async fn concurrent_transactions_get_gapless_seqs_seen_in_order_by_everyone() {
    const WRITERS: usize = 5;
    const TRANSACTIONS_PER_WRITER: usize = 20;
    const TOTAL: u64 = (WRITERS * TRANSACTIONS_PER_WRITER) as u64;
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut observer = server.connect(&owner).await;
    observer.open_workspace(workspace_id, 0).await;
    let mut writers = Vec::new();
    for _ in 0..WRITERS {
        let editor = server.new_member(&owner, workspace_id, "Editor").await;
        let mut client = server.connect(&editor).await;
        client.open_workspace(workspace_id, 0).await;
        writers.push(client);
    }
    let mut tasks = Vec::new();
    for mut client in writers {
        tasks.push(tokio::spawn(async move {
            let transactions: Vec<Transaction> = (0..TRANSACTIONS_PER_WRITER)
                .map(|_| create_page(&new_page("Concurrent", None)))
                .collect();
            for transaction in &transactions {
                client.send_transaction(workspace_id, transaction).await;
            }
            let mut acked = Vec::new();
            let mut seqs_in_arrival_order = Vec::new();
            while seqs_in_arrival_order.len() < TOTAL as usize {
                let seq = match client.receive().await {
                    ServerMessage::TransactionAck { seq, .. } => {
                        acked.push(seq);
                        seq
                    }
                    ServerMessage::RemoteTransaction { seq, .. } => seq,
                    other => panic!("unexpected {other:?}"),
                };
                seqs_in_arrival_order.push(seq);
            }
            (acked, seqs_in_arrival_order)
        }));
    }

    let mut all_acked = Vec::new();
    for task in tasks {
        let (acked, seqs_in_arrival_order) = task.await.unwrap();
        assert_eq!(acked.len(), TRANSACTIONS_PER_WRITER);
        assert_eq!(
            seqs_in_arrival_order,
            (1..=TOTAL).collect::<Vec<_>>(),
            "each writer sees every seq in order"
        );
        all_acked.extend(acked);
    }
    all_acked.sort_unstable();
    assert_eq!(all_acked, (1..=TOTAL).collect::<Vec<_>>());
    for expected in 1..=TOTAL {
        assert!(matches!(
            observer.receive().await,
            ServerMessage::RemoteTransaction { seq, .. } if seq == expected
        ));
    }
    let snapshot = observer
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(snapshot.seq, TOTAL);
    assert_eq!(snapshot.records.len(), TOTAL as usize);
}

#[tokio::test]
async fn workspaces_are_isolated_and_a_page_id_belongs_to_one_workspace() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let first = server.create_workspace(&owner).await;
    let second = server.create_workspace(&owner).await;
    let mut first_client = server.connect(&owner).await;
    let mut second_client = server.connect(&owner).await;
    first_client.open_workspace(first, 0).await;
    second_client.open_workspace(second, 0).await;
    let first_page = new_page("In the first", None);
    first_client.commit(first, &create_page(&first_page)).await;
    second_client.assert_silent().await;

    // Records of another workspace do not exist from here, and their ids cannot be reused.
    let child_across = create_page(&new_page("Child", Some(first_page.id)));
    assert_eq!(
        second_client.commit_rejected(second, &child_across).await,
        TransactionRejection::Invalid(ValidationError::PageNotFound(first_page.id))
    );
    let same_id = create_page(&Page {
        title: "Same id".into(),
        ..first_page.clone()
    });
    assert_eq!(
        second_client.commit_rejected(second, &same_id).await,
        TransactionRejection::Invalid(ValidationError::RecordAlreadyExists(RecordId::Page(
            first_page.id
        )))
    );
    let rename_across = set_page(first_page.id, PageField::Title("Hijacked".into()));
    assert_eq!(
        second_client.commit_rejected(second, &rename_across).await,
        TransactionRejection::Invalid(ValidationError::PageNotFound(first_page.id))
    );

    // The page record and its content share one binding to a workspace.
    second_client
        .send(&ClientMessage::OpenPage {
            workspace_id: second,
            page_id: first_page.id,
            last_seen_seq: 0,
        })
        .await;
    assert_eq!(
        second_client.receive().await,
        ServerMessage::PageRejected {
            page_id: first_page.id,
            reason: RejectReason::PageInAnotherWorkspace
        }
    );
    let content_first = PageId::generate();
    second_client.open_page(second, content_first, 0).await;
    second_client.push(content_first, b"content").await;
    let record_elsewhere = create_page(&Page {
        id: content_first,
        ..new_page("Elsewhere", None)
    });
    assert_eq!(
        first_client.commit_rejected(first, &record_elsewhere).await,
        TransactionRejection::Invalid(ValidationError::RecordAlreadyExists(RecordId::Page(
            content_first
        )))
    );
    let record_here = create_page(&Page {
        id: content_first,
        ..new_page("Here", None)
    });
    assert_eq!(second_client.commit(second, &record_here).await, 1);

    let first_snapshot = first_client
        .open_workspace(first, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(snapshot_pages(&first_snapshot), vec![first_page]);
}

#[tokio::test]
async fn databases_views_and_property_values_are_validated_and_snapshotted() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut client = server.connect(&owner).await;
    client.open_workspace(workspace_id, 0).await;
    let host = new_page("Tasks", None);
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
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: vec![],
        sort: vec![],
        visible_columns: vec![property.id],
        column_widths: vec![],
    };
    let row = Page {
        row_of: Some(database.id),
        ..new_page("Write tests", Some(host.id))
    };
    let set_estimate = |value| {
        Transaction::new(vec![Operation::Set(FieldChange::PropertyValue {
            page: row.id,
            property: property.id,
            value: Some(value),
        })])
    };

    let setup = Transaction::new(vec![
        Operation::Create(Record::Page(host.clone())),
        Operation::Create(Record::Database(database.clone())),
        Operation::Create(Record::View(view.clone())),
        Operation::Create(Record::Page(row.clone())),
    ]);
    assert_eq!(client.commit(workspace_id, &setup).await, 1);
    assert_eq!(
        client
            .commit(workspace_id, &set_estimate(PropertyValue::Number(3.5)))
            .await,
        2
    );

    let wrong_type = set_estimate(PropertyValue::Text("three".into()));
    let second_database_on_host = create(Record::Database(Database {
        id: DatabaseId::generate(),
        ..database.clone()
    }));
    let delete_with_rows =
        Transaction::new(vec![Operation::Delete(RecordId::Database(database.id))]);
    let expected_errors = [
        ValidationError::PropertyTypeMismatch {
            property: property.id,
            expected: PropertyType::Number,
            actual: PropertyType::Text,
        },
        ValidationError::PageAlreadyHostsDatabase {
            page: host.id,
            database: database.id,
        },
        ValidationError::DatabaseHasRows(database.id),
    ];
    for (invalid, expected) in [wrong_type, second_database_on_host, delete_with_rows]
        .iter()
        .zip(expected_errors)
    {
        assert_eq!(
            client.commit_rejected(workspace_id, invalid).await,
            TransactionRejection::Invalid(expected)
        );
    }

    let snapshot = client
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(snapshot.seq, 2);
    assert!(snapshot.records.contains(&Record::Database(database)));
    assert!(snapshot.records.contains(&Record::View(view)));
    assert_eq!(
        snapshot.property_values,
        vec![PropertyValueEntry {
            page: row.id,
            property: property.id,
            value: PropertyValue::Number(3.5)
        }]
    );
    let mut expected_pages = vec![host, row];
    expected_pages.sort_by_key(|page| page.id);
    assert_eq!(snapshot_pages(&snapshot), expected_pages);
}

#[tokio::test]
async fn losing_membership_closes_the_workspace_stream() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id: WorkspaceId = server.create_workspace(&owner).await;
    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let mut owner_client = server.connect(&owner).await;
    let mut editor_client = server.connect(&editor).await;
    owner_client.open_workspace(workspace_id, 0).await;
    editor_client.open_workspace(workspace_id, 0).await;

    server
        .pool
        .get()
        .await
        .unwrap()
        .execute(
            "DELETE FROM memberships WHERE workspace_id = $1 AND user_id = $2",
            &[&workspace_id.0, &editor.user_id.0],
        )
        .await
        .unwrap();

    assert_eq!(
        editor_client.receive().await,
        ServerMessage::WorkspaceRejected {
            workspace_id,
            reason: RejectReason::NotAMember
        }
    );
    owner_client
        .commit(workspace_id, &create_page(&new_page("Private", None)))
        .await;
    editor_client.assert_silent().await;
}

#[tokio::test]
async fn deleting_a_page_record_deletes_its_content() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut client = server.connect(&owner).await;
    client.open_workspace(workspace_id, 0).await;
    let page = new_page("Doomed", None);
    client.commit(workspace_id, &create_page(&page)).await;
    client.open_page(workspace_id, page.id, 0).await;
    let update_id = uuid::Uuid::now_v7();
    client.send_update(update_id, page.id, b"content").await;
    client.receive_ack(update_id).await;

    let delete = Transaction::new(vec![Operation::Delete(RecordId::Page(page.id))]);
    client.commit(workspace_id, &delete).await;

    let catch_up = client.open_page(workspace_id, page.id, 0).await;
    assert_eq!(catch_up.snapshot, None);
    assert!(catch_up.updates.is_empty());
    // A late resend of the deleted content is recognised, not stored again.
    client.send_update(update_id, page.id, b"content").await;
    assert_eq!(client.receive_ack(update_id).await, 1);
    assert!(
        client
            .open_page(workspace_id, page.id, 0)
            .await
            .updates
            .is_empty()
    );
}

/// Six transactions of 500 pages, each with a long title: about 3 MB of records.
async fn commit_many_pages(
    client: &mut common::SyncClient,
    workspace_id: WorkspaceId,
) -> Vec<Page> {
    let long_title = "x".repeat(1_000);
    let mut pages = Vec::new();
    for _ in 0..6 {
        let batch: Vec<Page> = (0..500).map(|_| new_page(&long_title, None)).collect();
        let operations = batch
            .iter()
            .map(|page| Operation::Create(Record::Page(page.clone())))
            .collect();
        client
            .commit(workspace_id, &Transaction::new(operations))
            .await;
        pages.extend(batch);
    }
    pages
}

#[tokio::test]
async fn a_large_snapshot_arrives_in_parts_and_a_commit_during_it_arrives_exactly_once() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut writer = server.connect(&owner).await;
    writer.open_workspace(workspace_id, 0).await;
    commit_many_pages(&mut writer, workspace_id).await;
    let mut reader = server.connect(&owner).await;

    reader
        .send(&ClientMessage::OpenWorkspace {
            workspace_id,
            last_seen_seq: 0,
        })
        .await;
    let during = new_page("Committed while the reader joins", None);
    writer.commit(workspace_id, &create_page(&during)).await;

    let (catch_up, part_count) = reader.receive_workspace_catch_up(workspace_id).await;
    assert!(part_count > 2, "{part_count} parts");
    let snapshot = catch_up.snapshot.unwrap();
    let in_snapshot = snapshot_pages(&snapshot)
        .iter()
        .any(|page| page.id == during.id);
    match snapshot.seq {
        6 => {
            assert!(!in_snapshot);
            assert!(matches!(
                reader.receive().await,
                ServerMessage::RemoteTransaction { seq: 7, .. }
            ));
        }
        7 => assert!(in_snapshot),
        other => panic!("unexpected snapshot seq {other}"),
    }
    reader.assert_silent().await;
    assert_eq!(snapshot.records.len() + usize::from(!in_snapshot), 3_001);
}

#[tokio::test]
async fn a_long_log_arrives_in_parts_in_order() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut writer = server.connect(&owner).await;
    writer.open_workspace(workspace_id, 0).await;
    commit_many_pages(&mut writer, workspace_id).await;
    let mut reader = server.connect(&owner).await;

    reader
        .send(&ClientMessage::OpenWorkspace {
            workspace_id,
            last_seen_seq: 1,
        })
        .await;
    let (catch_up, part_count) = reader.receive_workspace_catch_up(workspace_id).await;

    assert!(part_count > 2, "{part_count} parts");
    assert_eq!(catch_up.snapshot, None);
    let seqs: Vec<u64> = catch_up
        .transactions
        .iter()
        .map(|logged| logged.seq)
        .collect();
    assert_eq!(seqs, vec![2, 3, 4, 5, 6]);
}

fn new_tag(name: &str) -> scroll_core::record::Tag {
    scroll_core::record::Tag {
        id: scroll_core::id::TagId::generate(),
        name: name.to_owned(),
        color: scroll_core::record::TagColor::Blue,
    }
}

fn tag_and_apply(tag: &scroll_core::record::Tag, page: PageId) -> Transaction {
    Transaction::new(vec![
        Operation::Create(Record::Tag(tag.clone())),
        Operation::Create(Record::PageTag(scroll_core::record::PageTag {
            page,
            tag: tag.id,
        })),
    ])
}

fn memberships(snapshot: &WorkspaceSnapshot) -> Vec<scroll_core::record::PageTag> {
    let mut memberships: Vec<_> = snapshot
        .records
        .iter()
        .filter_map(|record| match record {
            Record::PageTag(membership) => Some(*membership),
            _ => None,
        })
        .collect();
    memberships.sort_by_key(|membership| membership.tag);
    memberships
}

#[tokio::test]
async fn two_clients_tagging_one_page_at_once_both_keep_their_tags() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let mut first = server.connect(&owner).await;
    let mut second = server.connect(&editor).await;
    first.open_workspace(workspace_id, 0).await;
    second.open_workspace(workspace_id, 0).await;
    let page = new_page("Tagged", None);
    first.commit(workspace_id, &create_page(&page)).await;
    assert!(matches!(
        second.receive().await,
        ServerMessage::RemoteTransaction { .. }
    ));
    let urgent = new_tag("Urgent");
    let draft = new_tag("Draft");

    first
        .send_transaction(workspace_id, &tag_and_apply(&urgent, page.id))
        .await;
    second
        .send_transaction(workspace_id, &tag_and_apply(&draft, page.id))
        .await;
    for client in [&mut first, &mut second] {
        let mut answers = 0;
        while answers < 2 {
            match client.receive().await {
                ServerMessage::TransactionAck { .. } | ServerMessage::RemoteTransaction { .. } => {
                    answers += 1
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    let snapshot = first
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    let mut expected = vec![
        scroll_core::record::PageTag {
            page: page.id,
            tag: urgent.id,
        },
        scroll_core::record::PageTag {
            page: page.id,
            tag: draft.id,
        },
    ];
    expected.sort_by_key(|membership| membership.tag);
    assert_eq!(memberships(&snapshot), expected);
    assert!(snapshot.records.contains(&Record::Tag(urgent)));
    assert!(snapshot.records.contains(&Record::Tag(draft)));
}

#[tokio::test]
async fn a_tag_name_already_taken_rejects_the_transaction_and_its_memberships() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut client = server.connect(&owner).await;
    client.open_workspace(workspace_id, 0).await;
    let page = new_page("Tagged", None);
    client.commit(workspace_id, &create_page(&page)).await;
    let first = new_tag("Urgent");
    client
        .commit(workspace_id, &tag_and_apply(&first, page.id))
        .await;

    let same_name = new_tag("  urgent ");
    assert_eq!(
        client
            .commit_rejected(workspace_id, &tag_and_apply(&same_name, page.id))
            .await,
        TransactionRejection::Invalid(ValidationError::TagNameTaken {
            name: "  urgent ".into(),
            existing: first.id
        })
    );
    let snapshot = client
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(
        memberships(&snapshot),
        vec![scroll_core::record::PageTag {
            page: page.id,
            tag: first.id
        }]
    );
}

#[tokio::test]
async fn deleting_a_tag_or_a_page_removes_its_memberships() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let mut client = server.connect(&owner).await;
    client.open_workspace(workspace_id, 0).await;
    let kept_page = new_page("Kept", None);
    let deleted_page = new_page("Deleted", None);
    let kept_tag = new_tag("Kept");
    let deleted_tag = new_tag("Deleted");
    let mut operations = vec![
        Operation::Create(Record::Page(kept_page.clone())),
        Operation::Create(Record::Page(deleted_page.clone())),
        Operation::Create(Record::Tag(kept_tag.clone())),
        Operation::Create(Record::Tag(deleted_tag.clone())),
    ];
    for page in [kept_page.id, deleted_page.id] {
        for tag in [kept_tag.id, deleted_tag.id] {
            operations.push(Operation::Create(Record::PageTag(
                scroll_core::record::PageTag { page, tag },
            )));
        }
    }
    client
        .commit(workspace_id, &Transaction::new(operations))
        .await;

    let deletions = Transaction::new(vec![
        Operation::Delete(RecordId::Tag(deleted_tag.id)),
        Operation::Delete(RecordId::Page(deleted_page.id)),
    ]);
    client.commit(workspace_id, &deletions).await;

    let snapshot = client
        .open_workspace(workspace_id, 0)
        .await
        .snapshot
        .unwrap();
    assert_eq!(
        memberships(&snapshot),
        vec![scroll_core::record::PageTag {
            page: kept_page.id,
            tag: kept_tag.id
        }]
    );
    assert!(!snapshot.records.contains(&Record::Tag(deleted_tag)));
}
