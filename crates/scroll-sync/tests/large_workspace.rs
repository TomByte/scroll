//! Publishing and joining a large workspace end to end, with timings. Slow, so ignored; run in release:
//! `cargo test -p scroll-sync --release --test large_workspace -- --ignored --nocapture`.

mod common;

use std::time::{Duration, Instant};

use common::{Team, TempDir, TestApp, TestServer, new_page, sorted_records};
use scroll_api::limits::MAX_MESSAGE_BYTES;
use scroll_core::id::{DatabaseId, PropertyId};
use scroll_core::operation::{FieldChange, Operation, Record, Transaction};
use scroll_core::page_doc::{BlockKind, BlockPlacement, PageDoc};
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue};
use scroll_core::store::{QueuedChange, WorkspaceStore};
use scroll_sync::{SyncEvent, publish_workspace};

const PAGES: usize = 5_000;
const ROWS: usize = 100_000;
const PROPERTIES: usize = 4;
const EDITOR_COMMIT_INTERVAL: Duration = Duration::from_millis(200);

fn megabytes(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// `pages` top-level pages, each with a paragraph of content, and one database with `rows` rows.
fn populate(store: &mut WorkspaceStore, pages: usize, rows: usize) {
    store.set_workspace_name("Large").unwrap();
    let page_records: Vec<Page> = (0..pages)
        .map(|index| new_page(&format!("Page {index}"), None))
        .collect();
    let mut operations: Vec<Operation> = page_records
        .iter()
        .map(|page| Operation::Create(Record::Page(page.clone())))
        .collect();
    let properties: Vec<Property> = (0..PROPERTIES)
        .map(|index| Property {
            id: PropertyId::generate(),
            name: format!("Property {index}"),
            property_type: PropertyType::Text,
            options: vec![],
        })
        .collect();
    let database = Database {
        id: DatabaseId::generate(),
        host_page: page_records[0].id,
        properties: properties.clone(),
    };
    operations.push(Operation::Create(Record::Database(database.clone())));
    for index in 0..rows {
        let row = Page {
            row_of: Some(database.id),
            ..new_page(&format!("Row {index}"), None)
        };
        operations.push(Operation::Create(Record::Page(row.clone())));
        for property in &properties {
            operations.push(Operation::Set(FieldChange::PropertyValue {
                page: row.id,
                property: property.id,
                value: Some(PropertyValue::Text(format!("value {index}"))),
            }));
        }
    }
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    for page in &page_records {
        let mut doc = PageDoc::new();
        let block = doc
            .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
            .unwrap();
        doc.insert_text(block, 0, &"Some words on the page. ".repeat(20))
            .unwrap();
        store
            .record_local_page_update(page.id, &doc.take_local_update().unwrap().unwrap())
            .unwrap();
    }
}

fn queue_shape(store: &WorkspaceStore) -> (usize, usize) {
    let queued: Vec<QueuedChange> = store
        .queued_changes_after(0, usize::MAX)
        .unwrap()
        .into_iter()
        .map(|entry| entry.change)
        .collect();
    let transactions = queued
        .iter()
        .filter(|change| matches!(change, QueuedChange::Transaction(_)))
        .count();
    (transactions, queued.len() - transactions)
}

/// The longest gap between the starts of consecutive committed transactions. The server commits one
/// workspace's transactions one at a time, each holding the workspace lock until it commits, so no single
/// commit held the lock longer than this.
async fn longest_commit_gap(server: &TestServer, workspace_id: uuid::Uuid) -> Duration {
    let rows = server
        .pool
        .get()
        .await
        .unwrap()
        .query(
            "SELECT extract(epoch FROM created_at)::float8 AS started FROM record_transactions
             WHERE workspace_id = $1 ORDER BY seq",
            &[&workspace_id],
        )
        .await
        .unwrap();
    let starts: Vec<f64> = rows.iter().map(|row| row.get("started")).collect();
    let longest = starts
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0, f64::max);
    Duration::from_secs_f64(longest)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn publishing_and_joining_a_large_workspace() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let files = TempDir::new();
    let team = Team::create(&server, &accounts).await;
    let mut laptop = TestApp::open(files.file("laptop.scroll"));
    let started = Instant::now();
    populate(&mut laptop.store, PAGES, ROWS);
    println!("populated locally in {:.1?}", started.elapsed());

    let membership = publish_workspace(&accounts, &team.owner.session_token, &mut laptop.store)
        .await
        .unwrap();
    let invite = accounts
        .create_invite(
            &team.owner.session_token,
            membership.workspace_id,
            scroll_core::id::Role::Editor,
        )
        .await
        .unwrap();
    accounts
        .accept_invite(&team.editor.session_token, &invite)
        .await
        .unwrap();
    let (chunks, page_updates) = queue_shape(&laptop.store);
    println!("queued: {chunks} publishing transactions and {page_updates} page updates");

    // Another member keeps editing the same workspace throughout.
    let mut editor = TestApp::join(files.file("editor.scroll"), &server.url(), &membership);
    editor.start(&team.editor.session_token);
    let started = Instant::now();
    laptop.start(&team.owner.session_token);
    let mut slowest_editor_ack = Duration::ZERO;
    let mut editor_commit: Option<Instant> = None;
    let mut next_editor_commit = Instant::now();
    let mut events_read = 0;
    let mut remaining = None;
    loop {
        laptop.pump();
        editor.pump();
        // Followed from progress events, as the app would; reading the 30 MB queue each time is too slow.
        for event in &laptop.log[events_read..] {
            if let SyncEvent::UploadProgress { remaining: now } = event {
                remaining = Some(*now);
            }
        }
        events_read = laptop.log.len();
        if let Some(committed_at) = editor_commit
            && editor.is_settled()
        {
            slowest_editor_ack = slowest_editor_ack.max(committed_at.elapsed());
            editor_commit = None;
        }
        if editor_commit.is_none() && Instant::now() >= next_editor_commit {
            editor.create_page("Edited during the publish");
            editor_commit = Some(Instant::now());
            next_editor_commit = Instant::now() + EDITOR_COMMIT_INTERVAL;
        }
        if remaining == Some(0) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(1_200),
            "the publish did not finish"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let publish_time = started.elapsed();
    let progress_reports = laptop
        .log
        .iter()
        .filter(|event| matches!(event, SyncEvent::UploadProgress { .. }))
        .count();
    let longest_lock = longest_commit_gap(&server, membership.workspace_id.0).await;
    println!("published in {publish_time:.1?}, with {progress_reports} upload progress reports");
    println!("the editor's slowest acknowledgement meanwhile: {slowest_editor_ack:.2?}");
    println!("longest the workspace lock was held (upper bound): {longest_lock:.2?}");

    // A third device joins.
    let mut desktop = TestApp::join(files.file("desktop.scroll"), &server.url(), &membership);
    let started = Instant::now();
    desktop.start(&team.owner.session_token);
    common::wait_until(&mut [&mut desktop], |apps| {
        apps[0]
            .log
            .iter()
            .any(|event| matches!(event, SyncEvent::AllRecordsChanged))
    })
    .await;
    let parts = desktop.log.iter().rev().find_map(|event| match event {
        SyncEvent::DownloadProgress { received_parts, .. } => Some(*received_parts),
        _ => None,
    });
    println!(
        "joined in {:.1?}, the catch-up in {} parts",
        started.elapsed(),
        parts.unwrap_or(1)
    );
    editor.stop();
    laptop.stop();
    desktop.stop();
    let laptop_records = sorted_records(&laptop.store);
    let desktop_records = sorted_records(&desktop.store);
    assert_eq!(laptop_records.len(), desktop_records.len());
    assert_eq!(laptop_records, desktop_records);
}

#[test]
#[ignore]
fn the_size_of_a_very_large_page() {
    const PARAGRAPHS: usize = 20_000;
    let mut doc = PageDoc::new();
    for index in 0..PARAGRAPHS {
        let block = doc
            .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
            .unwrap();
        doc.insert_text(
            block,
            0,
            &format!("{index}: {}", "a sentence of text ".repeat(5)),
        )
        .unwrap();
    }
    let snapshot = doc.export_snapshot().unwrap();
    let as_one_update = doc.take_local_update().unwrap().unwrap();
    println!(
        "{PARAGRAPHS} paragraphs of about 100 characters: snapshot {:.1} MiB, as one update {:.1} MiB, limit {:.0} MiB",
        megabytes(snapshot.len()),
        megabytes(as_one_update.len()),
        megabytes(MAX_MESSAGE_BYTES)
    );
}
