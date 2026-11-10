use super::*;
use crate::id::PageId;
use crate::operation::{Operation, Record, RecordId, Transaction};
use crate::position::Position;
use crate::record::Page;
use uuid::Uuid;

#[test]
fn a_write_gets_in_between_another_connection_s_back_to_back_transactions() {
    let directory = std::env::temp_dir().join(format!("scroll-busy-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    let mut busy = WorkspaceStore::open(&path).unwrap();
    let mut other = WorkspaceStore::open(&path).unwrap();
    let busy_writer = std::thread::spawn(move || {
        // About three seconds of 100 ms transactions with 3 ms gaps, like a snapshot's finish.
        for _ in 0..30 {
            let tx = busy.conn.transaction().unwrap();
            tx.execute("UPDATE workspace SET name = 'busy'", [])
                .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            tx.commit().unwrap();
            std::thread::sleep(Duration::from_millis(3));
        }
    });
    std::thread::sleep(Duration::from_millis(100));
    let started = std::time::Instant::now();
    other.set_workspace_name("mine").unwrap();
    let waited = started.elapsed();
    busy_writer.join().unwrap();
    assert!(waited < Duration::from_millis(500), "waited {waited:?}");
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_file_says_whether_opening_it_will_migrate() {
    let directory = std::env::temp_dir().join(format!("scroll-behind-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    assert!(!WorkspaceStore::needs_migration(&path).unwrap());
    drop(WorkspaceStore::open(&path).unwrap());
    assert!(!WorkspaceStore::needs_migration(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 3)
        .unwrap();
    assert!(WorkspaceStore::needs_migration(&path).unwrap());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_file_made_before_incremental_vacuuming_says_it_cannot_reclaim_space() {
    let directory = std::env::temp_dir().join(format!("scroll-old-file-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE made_earlier (x INTEGER);")
        .unwrap();
    let mut store = WorkspaceStore::open(&path).unwrap();
    assert!(matches!(
        store.reclaim_free_space(10),
        Err(StoreError::CannotReclaimSpace)
    ));
    drop(store);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn space_freed_by_deletes_is_reclaimed_in_short_steps() {
    let directory = std::env::temp_dir().join(format!("scroll-reclaim-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut store = WorkspaceStore::open(&directory.join("workspace.scroll")).unwrap();
    let pages: Vec<Page> = (0..3_000)
        .map(|index| Page {
            id: PageId::generate(),
            parent: None,
            title: format!("A page with a long enough title to fill space {index}"),
            icon: None,
            position: Position::between(None, None).unwrap(),
            row_of: None,
            is_archived: false,
        })
        .collect();
    let creates = pages
        .iter()
        .cloned()
        .map(|page| Operation::Create(Record::Page(page)));
    store
        .apply_local_transaction(&Transaction::new(creates.collect()))
        .unwrap();
    let deletes = pages
        .iter()
        .map(|page| Operation::Delete(RecordId::Page(page.id)));
    store
        .apply_local_transaction(&Transaction::new(deletes.collect()))
        .unwrap();
    let page_count = |store: &WorkspaceStore| -> usize {
        store
            .conn
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap()
    };
    let before = page_count(&store);

    let free_before: usize = store
        .conn
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .unwrap();
    let mut remaining = store.reclaim_free_space(10).unwrap();
    assert_eq!(remaining, free_before - 10);
    for _ in 0..1_000 {
        if remaining == 0 {
            break;
        }
        remaining = store.reclaim_free_space(10).unwrap();
    }
    assert_eq!(remaining, 0);
    assert!(page_count(&store) < before);
    drop(store);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn connections_are_durable_on_commit_and_cache_generously() {
    let directory = std::env::temp_dir().join(format!("scroll-config-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    let writer = WorkspaceStore::open(&path).unwrap();
    let reader = WorkspaceStore::open_reader(&path).unwrap();
    for store in [&writer, &reader] {
        let pragma = |name: &str| -> i64 {
            store
                .conn
                .pragma_query_value(None, name, |row| row.get(0))
                .unwrap()
        };
        // FULL is 2.
        assert_eq!(pragma("synchronous"), 2);
        assert_eq!(pragma("fullfsync"), 1);
        assert_eq!(pragma("checkpoint_fullfsync"), 1);
        assert_eq!(pragma("cache_size"), -65536);
        assert_eq!(pragma("journal_size_limit"), 67_108_864);
        // MEMORY is 2.
        assert_eq!(pragma("temp_store"), 2);
    }
    let journal: String = writer
        .conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    writer.optimize().unwrap();
    let analysed: usize = writer
        .conn
        .query_row("SELECT count(*) FROM sqlite_stat1", [], |row| row.get(0))
        .unwrap();
    assert!(analysed > 0);
    drop((writer, reader));
    std::fs::remove_dir_all(&directory).unwrap();
}
