use super::*;
use crate::id::WorkspaceId;
use crate::page_doc::{Block, BlockKind, BlockPlacement};
use crate::store::test_support;
use uuid::Uuid;

use crate::store::QueuedChange;
use crate::store::{open_in_memory, open_linked_in_memory};

/// A page edited in steps, each step's update returned in order, plus the final blocks.
fn edited_page(step_count: usize) -> (Vec<Vec<u8>>, Vec<std::sync::Arc<Block>>) {
    let mut doc = PageDoc::new();
    let block = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    let mut updates = Vec::new();
    for step in 0..step_count {
        doc.insert_text(block, 0, &format!("{step} ")).unwrap();
        updates.push(doc.take_local_update().unwrap().unwrap());
    }
    (updates, doc.blocks().unwrap())
}

fn rebuild(catch_up: &PageCatchUp, start: &PageDoc) {
    if let Some(snapshot) = &catch_up.snapshot {
        start.import(&snapshot.bytes).unwrap();
    }
    for update in &catch_up.updates {
        start.import(&update.bytes).unwrap();
    }
}

#[test]
fn appended_updates_are_sequenced_and_returned_after_the_last_seen() {
    let mut store = open_in_memory();
    let page_id = PageId::generate();

    store.record_local_page_update(page_id, b"first").unwrap();
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 1);
    store.record_local_page_update(page_id, b"second").unwrap();
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 2);

    let catch_up = store.catch_up_page(page_id, 1).unwrap();
    assert_eq!(catch_up.snapshot, None);
    assert_eq!(
        catch_up.updates,
        vec![SequencedBytes {
            seq: 2,
            bytes: b"second".to_vec()
        }]
    );
}

#[test]
fn compaction_keeps_catch_up_consistent_for_every_reader() {
    let mut store = open_in_memory();
    let page_id = PageId::generate();
    let (updates, final_blocks) = edited_page(4);
    for update in &updates[..3] {
        store.record_local_page_update(page_id, update).unwrap();
    }

    // A reader who saw update 1 before compaction.
    let early_reader = PageDoc::new();
    early_reader.import(&updates[0]).unwrap();

    store.compact_page(page_id).unwrap();
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 3);
    store
        .record_local_page_update(page_id, &updates[3])
        .unwrap();
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 4);

    let from_one = store.catch_up_page(page_id, 1).unwrap();
    assert_eq!(
        from_one.snapshot.as_ref().map(|snapshot| snapshot.seq),
        Some(3)
    );
    assert_eq!(
        from_one
            .updates
            .iter()
            .map(|update| update.seq)
            .collect::<Vec<_>>(),
        vec![4]
    );
    rebuild(&from_one, &early_reader);
    assert_eq!(early_reader.blocks().unwrap(), final_blocks);

    let from_three = store.catch_up_page(page_id, 3).unwrap();
    assert_eq!(from_three.snapshot, None);
    assert_eq!(from_three.updates.len(), 1);

    let fresh_reader = PageDoc::new();
    rebuild(&store.catch_up_page(page_id, 0).unwrap(), &fresh_reader);
    assert_eq!(fresh_reader.blocks().unwrap(), final_blocks);
    assert_eq!(
        store.load_page_doc(page_id).unwrap().blocks().unwrap(),
        final_blocks
    );

    assert!(store.catch_up_page(page_id, 4).unwrap().updates.is_empty());
}

#[test]
fn loading_includes_updates_still_waiting_for_the_server() {
    let mut store = open_in_memory();
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();
    let page_id = PageId::generate();
    let (updates, final_blocks) = edited_page(2);
    store
        .store_remote_page_update(
            page_id,
            &SequencedBytes {
                seq: 1,
                bytes: updates[0].clone(),
            },
        )
        .unwrap();
    store
        .record_local_page_update(page_id, &updates[1])
        .unwrap();
    let Some(QueuedChange::PageUpdate(queued)) =
        test_support::queued_changes(&store).unwrap().pop()
    else {
        panic!("the update should be queued");
    };
    let update_id = queued.update_id;

    assert_eq!(
        store.load_page_doc(page_id).unwrap().blocks().unwrap(),
        final_blocks
    );
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 1);

    store.acknowledge_page_update(update_id, 2).unwrap();
    assert!(test_support::queued_changes(&store).unwrap().is_empty());
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 2);
    assert_eq!(
        store.load_page_doc(page_id).unwrap().blocks().unwrap(),
        final_blocks
    );
    assert_eq!(
        store.catch_up_page(page_id, 1).unwrap().updates[0].bytes,
        updates[1]
    );
}

#[test]
fn a_catch_up_snapshot_replaces_an_older_one_and_redelivery_is_harmless() {
    let mut store = open_linked_in_memory();
    let page_id = PageId::generate();
    let (updates, final_blocks) = edited_page(3);
    store
        .store_remote_page_update(
            page_id,
            &SequencedBytes {
                seq: 1,
                bytes: updates[0].clone(),
            },
        )
        .unwrap();

    let server_doc = PageDoc::new();
    server_doc.import(&updates[0]).unwrap();
    server_doc.import(&updates[1]).unwrap();
    let catch_up = PageCatchUp {
        snapshot: Some(SequencedBytes {
            seq: 2,
            bytes: server_doc.export_snapshot().unwrap(),
        }),
        updates: vec![SequencedBytes {
            seq: 3,
            bytes: updates[2].clone(),
        }],
    };
    store.store_page_catch_up(page_id, &catch_up).unwrap();
    store.store_page_catch_up(page_id, &catch_up).unwrap();
    store
        .store_remote_page_update(
            page_id,
            &SequencedBytes {
                seq: 1,
                bytes: updates[0].clone(),
            },
        )
        .unwrap();

    assert_eq!(store.catch_up_page(page_id, 0).unwrap(), catch_up);
    assert_eq!(
        store.load_page_doc(page_id).unwrap().blocks().unwrap(),
        final_blocks
    );
}

#[test]
fn saving_folds_updates_up_to_the_included_sequence_only() {
    let mut store = open_in_memory();
    let page_id = PageId::generate();
    let (updates, final_blocks) = edited_page(3);
    for update in &updates {
        store.record_local_page_update(page_id, update).unwrap();
    }
    let doc_at_two = PageDoc::new();
    doc_at_two.import(&updates[0]).unwrap();
    doc_at_two.import(&updates[1]).unwrap();

    store.save_page_doc(page_id, &doc_at_two, 2).unwrap();
    let catch_up = store.catch_up_page(page_id, 0).unwrap();
    assert_eq!(catch_up.snapshot.map(|snapshot| snapshot.seq), Some(2));
    assert_eq!(
        catch_up
            .updates
            .iter()
            .map(|update| update.seq)
            .collect::<Vec<_>>(),
        vec![3]
    );
    assert_eq!(
        store.load_page_doc(page_id).unwrap().blocks().unwrap(),
        final_blocks
    );

    let result = store.save_page_doc(page_id, &doc_at_two, 1);
    assert!(matches!(
        result,
        Err(StoreError::SnapshotOlderThanStored {
            offered: 1,
            stored: 2
        })
    ));
}

#[test]
fn a_refused_page_update_and_the_updates_after_it_are_dropped() {
    let mut store = open_linked_in_memory();
    let page_id = PageId::generate();
    let other_page = PageId::generate();
    let (updates, _) = edited_page(1);
    store
        .store_remote_page_update(
            page_id,
            &SequencedBytes {
                seq: 1,
                bytes: updates[0].clone(),
            },
        )
        .unwrap();
    for bytes in [b"first".as_slice(), b"refused", b"later"] {
        store.record_local_page_update(page_id, bytes).unwrap();
    }
    store
        .record_local_page_update(other_page, b"elsewhere")
        .unwrap();
    let update_ids: Vec<Uuid> = test_support::queued_changes(&store)
        .unwrap()
        .into_iter()
        .filter_map(|change| match change {
            QueuedChange::PageUpdate(update) => Some(update.update_id),
            QueuedChange::Transaction(_) => None,
        })
        .collect();

    assert_eq!(
        store.reject_page_update(update_ids[1]).unwrap(),
        Some(page_id)
    );
    let remaining: Vec<Vec<u8>> = test_support::queued_changes(&store)
        .unwrap()
        .into_iter()
        .filter_map(|change| match change {
            QueuedChange::PageUpdate(update) => Some(update.update),
            QueuedChange::Transaction(_) => None,
        })
        .collect();
    assert_eq!(remaining, vec![b"first".to_vec(), b"elsewhere".to_vec()]);
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 0);
    assert_eq!(store.reject_page_update(update_ids[1]).unwrap(), None);
}

#[test]
fn deleting_a_page_on_a_local_only_workspace_removes_its_content_and_search_text() {
    use crate::operation::{Operation, Record, RecordId, Transaction};
    use crate::position::Position;
    use crate::record::Page;

    let mut store = open_in_memory();
    let page = Page {
        id: PageId::generate(),
        parent: None,
        title: "Doomed".into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    };
    let page_id = page.id;
    store
        .apply_local_transaction(&Transaction::new(vec![Operation::Create(Record::Page(
            page,
        ))]))
        .unwrap();
    let (updates, _) = edited_page(2);
    for update in &updates {
        store.record_local_page_update(page_id, update).unwrap();
    }
    let doc = store.load_page_doc(page_id).unwrap();
    store.save_page_doc(page_id, &doc, 2).unwrap();
    assert_eq!(store.search("Doomed", 5).unwrap().len(), 1);

    store
        .apply_local_transaction(&Transaction::new(vec![Operation::Delete(RecordId::Page(
            page_id,
        ))]))
        .unwrap();
    assert_eq!(store.last_seen_page_seq(page_id).unwrap(), 0);
    assert!(
        store
            .load_page_doc(page_id)
            .unwrap()
            .blocks()
            .unwrap()
            .is_empty()
    );
    assert!(store.search("Doomed", 5).unwrap().is_empty());
    assert!(store.search("0", 5).unwrap().is_empty());
}

#[test]
fn refreshing_search_indexes_text_still_waiting_for_the_server() {
    use crate::operation::{Operation, Record, Transaction};
    use crate::position::Position;
    use crate::record::Page;

    let mut store = open_linked_in_memory();
    let page = Page {
        id: PageId::generate(),
        parent: None,
        title: "Garden".into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    };
    let page_id = page.id;
    store
        .apply_local_transaction(&Transaction::new(vec![Operation::Create(Record::Page(
            page,
        ))]))
        .unwrap();
    let mut doc = PageDoc::new();
    let block = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    doc.insert_text(block, 0, "tomatoes").unwrap();
    store
        .record_local_page_update(page_id, &doc.take_local_update().unwrap().unwrap())
        .unwrap();
    assert!(store.search("tomatoes", 5).unwrap().is_empty());

    store.refresh_page_search(page_id).unwrap();

    let hits = store.search("tomatoes", 5).unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.page_id).collect::<Vec<_>>(),
        vec![page_id]
    );
}
