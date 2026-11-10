use crate::id::{PageId, TagId, WorkspaceId};
use crate::operation::{FieldChange, Operation, PageField, Record, Transaction};
use crate::position::Position;
use crate::record::{Page, PageTag, Tag, TagColor};
use crate::store::test_support;
use crate::store::{StoreError, WorkspaceStore, open_in_memory, open_linked_in_memory};
use crate::sync::WorkspaceSnapshot;

fn new_page(title: &str) -> Page {
    Page {
        id: PageId::generate(),
        parent: None,
        title: title.into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    }
}

/// A server-side workspace with pages, a tag and memberships, exported at `seq`.
fn server_snapshot(seq: u64, page_count: usize) -> WorkspaceSnapshot {
    let mut server = open_in_memory();
    let tag = Tag {
        id: TagId::generate(),
        name: "urgent".into(),
        color: TagColor::Red,
    };
    let mut operations = vec![Operation::Create(Record::Tag(tag.clone()))];
    for index in 0..page_count {
        let page = new_page(&format!("Server page {index}"));
        let id = page.id;
        operations.push(Operation::Create(Record::Page(page)));
        if index % 2 == 0 {
            operations.push(Operation::Create(Record::PageTag(PageTag {
                page: id,
                tag: tag.id,
            })));
        }
    }
    server
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    WorkspaceSnapshot {
        seq,
        ..server.export_workspace_snapshot().unwrap()
    }
}

/// The snapshot cut into parts of at most `size` records, values kept with nothing in particular.
fn parts(snapshot: &WorkspaceSnapshot, size: usize) -> Vec<WorkspaceSnapshot> {
    snapshot
        .records
        .chunks(size)
        .map(|records| WorkspaceSnapshot {
            seq: snapshot.seq,
            records: records.to_vec(),
            property_values: Vec::new(),
        })
        .collect()
}

fn records_of(store: &WorkspaceStore) -> Vec<String> {
    let snapshot = store.export_workspace_snapshot().unwrap();
    let mut records: Vec<String> = snapshot
        .records
        .iter()
        .map(|record| format!("{record:?}"))
        .collect();
    records.sort();
    records
}

fn client_with_local_page() -> (WorkspaceStore, PageId) {
    let mut client = open_linked_in_memory();
    let page = new_page("Local draft");
    let id = page.id;
    client
        .apply_local_transaction(&Transaction::new(vec![Operation::Create(Record::Page(
            page,
        ))]))
        .unwrap();
    (client, id)
}

#[test]
fn a_snapshot_staged_in_parts_ends_like_the_whole_snapshot() {
    let snapshot = server_snapshot(7, 25);
    let (mut whole, _) = client_with_local_page();
    test_support::apply_snapshot(&mut whole, snapshot.clone()).unwrap();

    let (mut staged, local_page) = client_with_local_page();
    staged.begin_workspace_snapshot(7).unwrap();
    for part in parts(&snapshot, 4) {
        staged.stage_workspace_snapshot_part(&part).unwrap();
    }
    staged.finish_workspace_snapshot(7).unwrap();

    let mut expected = records_of(&whole);
    expected.retain(|record| !record.contains("Local draft"));
    let mut actual = records_of(&staged);
    actual.retain(|record| !record.contains("Local draft"));
    assert_eq!(actual, expected);
    // The pending local page survives on top of the server's records.
    assert!(staged.page(local_page).unwrap().is_some());
    assert_eq!(staged.last_seen_workspace_seq().unwrap(), 7);
    assert_eq!(staged.tags().unwrap()[0].page_count, 13);
}

#[test]
fn staged_parts_stay_out_of_sight_and_an_interrupted_snapshot_leaves_the_old_state() {
    let directory = std::env::temp_dir().join(format!("scroll-staging-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    let snapshot = server_snapshot(3, 10);
    {
        let mut client = WorkspaceStore::open(&path).unwrap();
        client
            .link_to_server("https://scroll.test", WorkspaceId::generate())
            .unwrap();
        let before = records_of(&client);
        client.begin_workspace_snapshot(3).unwrap();
        client
            .stage_workspace_snapshot_part(&parts(&snapshot, 4)[0])
            .unwrap();
        assert_eq!(records_of(&client), before);
        // Dropped mid-way, as on a crash or a cut connection.
    }
    let mut client = WorkspaceStore::open(&path).unwrap();
    assert!(records_of(&client).is_empty());
    assert_eq!(client.last_seen_workspace_seq().unwrap(), 0);

    // Beginning again discards the leftover part.
    client.begin_workspace_snapshot(3).unwrap();
    let all_parts = parts(&snapshot, 4);
    client.stage_workspace_snapshot_part(&all_parts[1]).unwrap();
    client.finish_workspace_snapshot(3).unwrap();
    let staged_titles: usize = all_parts[1]
        .records
        .iter()
        .filter(|record| matches!(record, Record::Page(_)))
        .count();
    assert_eq!(
        test_support::child_pages(&client, None).unwrap().len(),
        staged_titles
    );
    drop(client);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn parts_and_finishes_must_match_the_snapshot_begun() {
    let mut client = open_linked_in_memory();
    let snapshot = server_snapshot(5, 2);
    assert!(matches!(
        client.stage_workspace_snapshot_part(&snapshot),
        Err(StoreError::NoStagedSnapshot {
            seq: 5,
            staged: None
        })
    ));
    client.begin_workspace_snapshot(4).unwrap();
    assert!(matches!(
        client.stage_workspace_snapshot_part(&snapshot),
        Err(StoreError::NoStagedSnapshot {
            seq: 5,
            staged: Some(4)
        })
    ));
    assert!(matches!(
        client.finish_workspace_snapshot(5),
        Err(StoreError::NoStagedSnapshot { .. })
    ));
    assert!(matches!(
        open_in_memory().begin_workspace_snapshot(1),
        Err(StoreError::NotLinked)
    ));
}

#[test]
fn a_staged_snapshot_older_than_what_was_seen_is_discarded() {
    let (mut client, local_page) = client_with_local_page();
    let newer = Transaction::new(vec![Operation::Set(FieldChange::Page {
        id: local_page,
        field: PageField::Title("Seen".into()),
    })]);
    client.apply_remote_transaction(9, &newer).unwrap();
    let before = records_of(&client);

    client.begin_workspace_snapshot(4).unwrap();
    client
        .stage_workspace_snapshot_part(&server_snapshot(4, 3))
        .unwrap();
    client.finish_workspace_snapshot(4).unwrap();
    assert_eq!(records_of(&client), before);
    assert_eq!(client.last_seen_workspace_seq().unwrap(), 9);
}

#[test]
fn a_finished_snapshot_leaves_nothing_staged() {
    let mut client = open_linked_in_memory();
    let snapshot = server_snapshot(2, 3);
    client.begin_workspace_snapshot(2).unwrap();
    client.stage_workspace_snapshot_part(&snapshot).unwrap();
    client.finish_workspace_snapshot(2).unwrap();
    assert!(matches!(
        client.stage_workspace_snapshot_part(&snapshot),
        Err(StoreError::NoStagedSnapshot { staged: None, .. })
    ));
    let staged_rows: usize = client
        .conn
        .query_row(
            "SELECT (SELECT count(*) FROM staged_record) + (SELECT count(*) FROM staged_value)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(staged_rows, 0);
}

#[test]
fn a_page_deleted_here_and_gone_from_the_snapshot_stays_gone_when_the_delete_is_rejected() {
    let mut client = open_linked_in_memory();
    let page = new_page("Shared");
    let id = page.id;
    client
        .apply_remote_transaction(
            1,
            &Transaction::new(vec![Operation::Create(Record::Page(page))]),
        )
        .unwrap();
    let delete = Transaction::new(vec![Operation::Delete(crate::operation::RecordId::Page(
        id,
    ))]);
    client.apply_local_transaction(&delete).unwrap();

    // Someone else deleted it too; the snapshot no longer has it.
    let snapshot = WorkspaceSnapshot {
        seq: 2,
        records: Vec::new(),
        property_values: Vec::new(),
    };
    client.begin_workspace_snapshot(2).unwrap();
    client.stage_workspace_snapshot_part(&snapshot).unwrap();
    client.finish_workspace_snapshot(2).unwrap();
    client.reject_transaction(delete.id).unwrap();

    assert_eq!(client.page(id).unwrap(), None);
}

#[test]
fn a_finish_cut_short_by_a_crash_resumes_or_restarts_to_the_same_end_state() {
    let directory = std::env::temp_dir().join(format!("scroll-finish-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    let page_count = super::FINISH_BATCH * 2 + 10;
    let snapshot = server_snapshot(4, page_count);
    let mut expected = open_linked_in_memory();
    test_support::apply_snapshot(&mut expected, snapshot.clone()).unwrap();
    {
        let mut client = WorkspaceStore::open(&path).unwrap();
        client
            .link_to_server("https://scroll.test", WorkspaceId::generate())
            .unwrap();
        client.begin_workspace_snapshot(4).unwrap();
        let all_parts = parts(&snapshot, 1_000);
        client.stage_workspace_snapshot_part(&all_parts[0]).unwrap();
        // Part-way through downloading, there is nothing to resume.
        assert_eq!(client.unfinished_snapshot().unwrap(), None);
        for part in &all_parts[1..] {
            client.stage_workspace_snapshot_part(part).unwrap();
        }
        assert_eq!(client.unfinished_snapshot().unwrap(), None);
        // One batch commits, then the process dies.
        assert_eq!(
            client.settle_next_staged_records().unwrap(),
            super::FINISH_BATCH
        );
    }

    let mut client = WorkspaceStore::open(&path).unwrap();
    assert_eq!(client.unfinished_snapshot().unwrap(), Some(4));
    // Part-way: some records are in, and the snapshot is not marked as seen.
    assert_eq!(client.last_seen_workspace_seq().unwrap(), 0);
    let partial = records_of(&client).len();
    assert!(partial > 0 && partial < records_of(&expected).len());

    let mut reports = Vec::new();
    client
        .finish_workspace_snapshot_reporting(4, |progress| reports.push(progress))
        .unwrap();
    assert_eq!(records_of(&client), records_of(&expected));
    assert_eq!(client.last_seen_workspace_seq().unwrap(), 4);
    assert_eq!(client.unfinished_snapshot().unwrap(), None);
    // The resumed finish starts from what the first batch applied and ends at the total.
    let total = snapshot.records.len();
    assert_eq!(
        reports.first().map(|report| report.applied),
        Some(super::FINISH_BATCH)
    );
    assert_eq!(
        reports.last().map(|report| (report.applied, report.total)),
        Some((total, total))
    );
    assert!(
        reports
            .windows(2)
            .all(|pair| pair[0].applied < pair[1].applied)
    );

    // Beginning again instead of resuming reaches the same state.
    client.begin_workspace_snapshot(5).unwrap();
    let again = WorkspaceSnapshot { seq: 5, ..snapshot };
    for part in parts(&again, 1_000) {
        client.stage_workspace_snapshot_part(&part).unwrap();
    }
    client.finish_workspace_snapshot(5).unwrap();
    assert_eq!(records_of(&client), records_of(&expected));
    drop(client);
    std::fs::remove_dir_all(&directory).unwrap();
}
