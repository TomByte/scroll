//! Server-side compaction of page updates into Loro snapshots.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{SyncClient, TestServer};
use scroll_api::protocol::ServerMessage;
use scroll_core::id::{PageId, WorkspaceId};
use scroll_core::page_doc::{Block, BlockKind, BlockPlacement, PageDoc};
use scroll_core::store::{PageCatchUp, SequencedBytes};
use scroll_server::Settings;
use uuid::Uuid;

const THRESHOLD: u64 = 10;

fn compacting_settings() -> Settings {
    Settings {
        compaction_threshold: THRESHOLD,
        ..TestServer::test_settings()
    }
}

/// A page edited by one peer in `step_count` steps: each step's Loro update, in order.
fn edits(step_count: usize) -> Vec<Vec<u8>> {
    let mut doc = PageDoc::new();
    let block = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    (0..step_count)
        .map(|step| {
            doc.insert_text(block, 0, &format!("{step} ")).unwrap();
            doc.take_local_update().unwrap().unwrap()
        })
        .collect()
}

fn blocks_from_updates<'a>(updates: impl IntoIterator<Item = &'a Vec<u8>>) -> Vec<Arc<Block>> {
    let doc = PageDoc::new();
    for update in updates {
        doc.import(update).unwrap();
    }
    doc.blocks().unwrap()
}

/// Imports the catch-up into `doc`, as a client does on opening the page.
fn import_catch_up(doc: &PageDoc, catch_up: &PageCatchUp) {
    if let Some(snapshot) = &catch_up.snapshot {
        doc.import(&snapshot.bytes).unwrap();
    }
    for update in &catch_up.updates {
        doc.import(&update.bytes).unwrap();
    }
}

/// Compaction runs in the background after a push; waits until a snapshot at `seq` is served.
async fn wait_for_snapshot_at(
    client: &mut SyncClient,
    page: (WorkspaceId, PageId),
    seq: u64,
) -> PageCatchUp {
    for _ in 0..200 {
        let catch_up = client.open_page(page.0, page.1, 0).await;
        if catch_up
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.seq == seq)
        {
            return catch_up;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no snapshot at seq {seq} was made");
}

#[tokio::test]
async fn catch_up_after_compaction_rebuilds_the_same_document() {
    let server = TestServer::start_with(compacting_settings()).await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    writer.open_page(workspace_id, page_id, 0).await;
    let updates = edits(THRESHOLD as usize);

    for update in &updates {
        writer.push(page_id, update).await;
    }
    let catch_up = wait_for_snapshot_at(&mut writer, (workspace_id, page_id), THRESHOLD).await;

    assert!(catch_up.updates.is_empty());
    let rebuilt = PageDoc::new();
    import_catch_up(&rebuilt, &catch_up);
    assert_eq!(rebuilt.blocks().unwrap(), blocks_from_updates(&updates));
}

#[tokio::test]
async fn the_snapshot_is_served_only_to_readers_who_have_not_seen_all_it_covers() {
    let server = TestServer::start_with(compacting_settings()).await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    writer.open_page(workspace_id, page_id, 0).await;
    let updates = edits(THRESHOLD as usize + 3);
    for update in &updates[..THRESHOLD as usize] {
        writer.push(page_id, update).await;
    }
    wait_for_snapshot_at(&mut writer, (workspace_id, page_id), THRESHOLD).await;
    for update in &updates[THRESHOLD as usize..] {
        writer.push(page_id, update).await;
    }
    let mut reader = server.connect(&owner).await;

    let behind_the_snapshot = reader.open_page(workspace_id, page_id, 4).await;
    assert_eq!(
        behind_the_snapshot.snapshot.as_ref().unwrap().seq,
        THRESHOLD
    );
    let later_seqs: Vec<u64> = behind_the_snapshot
        .updates
        .iter()
        .map(|update| update.seq)
        .collect();
    assert_eq!(later_seqs, vec![11, 12, 13]);
    // A reader that already had the first four updates ends up with the whole document.
    let reader_doc = PageDoc::new();
    for update in &updates[..4] {
        reader_doc.import(update).unwrap();
    }
    import_catch_up(&reader_doc, &behind_the_snapshot);
    assert_eq!(reader_doc.blocks().unwrap(), blocks_from_updates(&updates));

    let past_the_snapshot = reader.open_page(workspace_id, page_id, 11).await;
    assert_eq!(past_the_snapshot.snapshot, None);
    assert_eq!(
        past_the_snapshot.updates,
        vec![
            SequencedBytes {
                seq: 12,
                bytes: updates[11].clone()
            },
            SequencedBytes {
                seq: 13,
                bytes: updates[12].clone()
            },
        ]
    );
    let at_the_snapshot = reader.open_page(workspace_id, page_id, THRESHOLD).await;
    assert_eq!(at_the_snapshot.snapshot, None);
    assert_eq!(at_the_snapshot.updates.len(), 3);
}

#[tokio::test]
async fn resending_an_update_folded_into_the_snapshot_returns_its_original_ack() {
    let server = TestServer::start_with(compacting_settings()).await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    writer.open_page(workspace_id, page_id, 0).await;
    let updates = edits(THRESHOLD as usize + 1);
    let first_update_id = Uuid::now_v7();
    writer
        .send_update(first_update_id, page_id, &updates[0])
        .await;
    writer.receive_ack(first_update_id).await;
    for update in &updates[1..THRESHOLD as usize] {
        writer.push(page_id, update).await;
    }
    wait_for_snapshot_at(&mut writer, (workspace_id, page_id), THRESHOLD).await;

    writer
        .send_update(first_update_id, page_id, &updates[0])
        .await;
    assert_eq!(writer.receive_ack(first_update_id).await, 1);
    assert_eq!(
        writer.push(page_id, &updates[THRESHOLD as usize]).await,
        THRESHOLD + 1
    );
}

#[tokio::test]
async fn an_unreadable_update_is_left_out_of_the_snapshot_without_stopping_compaction() {
    let server = TestServer::start_with(compacting_settings()).await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    writer.open_page(workspace_id, page_id, 0).await;
    let updates = edits(2 * THRESHOLD as usize - 1);

    writer.push(page_id, &updates[0]).await;
    writer.push(page_id, b"not a loro update").await;
    for update in &updates[1..THRESHOLD as usize - 1] {
        writer.push(page_id, update).await;
    }
    let first = wait_for_snapshot_at(&mut writer, (workspace_id, page_id), THRESHOLD).await;
    let rebuilt = PageDoc::new();
    import_catch_up(&rebuilt, &first);
    assert_eq!(
        rebuilt.blocks().unwrap(),
        blocks_from_updates(&updates[..THRESHOLD as usize - 1])
    );

    for update in &updates[THRESHOLD as usize - 1..] {
        writer.push(page_id, update).await;
    }
    let second = wait_for_snapshot_at(&mut writer, (workspace_id, page_id), 2 * THRESHOLD).await;
    let rebuilt = PageDoc::new();
    import_catch_up(&rebuilt, &second);
    assert_eq!(rebuilt.blocks().unwrap(), blocks_from_updates(&updates));
}

#[tokio::test]
async fn compaction_during_concurrent_pushes_loses_nothing() {
    const WRITERS: usize = 4;
    const UPDATES_PER_WRITER: usize = 30;
    let server = TestServer::start_with(Settings {
        compaction_threshold: 7,
        ..TestServer::test_settings()
    })
    .await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let all_updates: Vec<Vec<Vec<u8>>> = (0..WRITERS).map(|_| edits(UPDATES_PER_WRITER)).collect();
    let mut tasks = Vec::new();
    for writer_updates in all_updates.clone() {
        let mut client = server.connect(&owner).await;
        client.open_page(workspace_id, page_id, 0).await;
        tasks.push(tokio::spawn(async move {
            for update in &writer_updates {
                let update_id = Uuid::now_v7();
                client.send_update(update_id, page_id, update).await;
                loop {
                    match client.receive().await {
                        ServerMessage::PageUpdateAck {
                            update_id: acked, ..
                        } if acked == update_id => break,
                        // Writers that finish first disconnect, so the others see them leave.
                        ServerMessage::RemotePageUpdate { .. }
                        | ServerMessage::PresenceLeft { .. } => {}
                        other => panic!("unexpected {other:?}"),
                    }
                }
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    let expected = blocks_from_updates(all_updates.iter().flatten());
    let mut reader = server.connect(&owner).await;
    for _ in 0..2 {
        let catch_up = reader.open_page(workspace_id, page_id, 0).await;
        assert!(catch_up.snapshot.is_some());
        let rebuilt = PageDoc::new();
        import_catch_up(&rebuilt, &catch_up);
        assert_eq!(rebuilt.blocks().unwrap(), expected);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
