//! Opening the workspace file: connection settings, migrating, and housekeeping (planner
//! statistics, reclaiming free space).

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::store::{StoreError, WorkspaceStore, schema};

const BUSY_TIMEOUT: Duration = Duration::from_secs(10);
/// Retries a locked write every millisecond until `BUSY_TIMEOUT`. SQLite's own busy timeout
/// backs off to 100 ms between tries, so a connection writing in a loop of short transactions
/// (a snapshot's finish) can starve it for seconds; a write retried this often gets into the
/// gap between two of those transactions.
fn retry_every_millisecond(attempts: i32) -> bool {
    std::thread::sleep(Duration::from_millis(1));
    u128::from(attempts.unsigned_abs()) < BUSY_TIMEOUT.as_millis()
}

impl WorkspaceStore {
    /// Several stores may share one file (the app and its sync engine each open one). Write
    /// transactions take the write lock when they begin, so a transaction that reads before it
    /// writes waits for the other connection instead of failing at its first write.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut conn = Connection::open(path)?;
        // Only takes effect on a new file, before its first table; files made earlier keep
        // whatever they had (a full VACUUM would convert them).
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        conn.busy_handler(Some(retry_every_millisecond))?;
        conn.set_transaction_behavior(TransactionBehavior::Immediate);
        conn.pragma_update(None, "journal_mode", "WAL")?;
        configure(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Whether opening the file will first migrate it, which for a large file written by an
    /// older version can take seconds (2.9 s at a million rows). Cheap: it reads one header field.
    /// When it says yes, open the file on a background thread and show the window meanwhile. A
    /// file that does not exist yet needs no slow migration.
    pub fn needs_migration(path: &Path) -> Result<bool, StoreError> {
        if !path.exists() {
            return Ok(false);
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        schema::is_behind(&conn)
    }

    /// A second, read-only connection to the same file, for running queries off the UI thread
    /// while this one writes. The file must already exist and be migrated (open it with `open`
    /// first).
    pub fn open_reader(path: &Path) -> Result<Self, StoreError> {
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_handler(Some(retry_every_millisecond))?;
        configure(&conn)?;
        Ok(Self { conn })
    }

    /// Refreshes the query planner's statistics where they are missing or stale. On a file never
    /// analysed it reads every table once (about 5 s for a million rows from a cold disk, though
    /// little CPU), so call it from a background thread after the workspace has opened, not when
    /// opening it.
    pub fn optimize(&self) -> Result<(), StoreError> {
        self.conn.execute_batch("PRAGMA optimize = 0x10002;")?;
        Ok(())
    }

    /// Returns up to `max_pages` unused pages to the file system in one short write, and how many
    /// unused pages remain. Space freed by deletes (a drained publishing queue, a finished
    /// snapshot's staging, deleted rows) stays in the file until this runs; call it repeatedly from
    /// a background thread until it returns 0, for example after the sync queue empties.
    /// Files created before incremental vacuuming was turned on cannot be shrunk this way and
    /// return `CannotReclaimSpace`.
    pub fn reclaim_free_space(&mut self, max_pages: usize) -> Result<usize, StoreError> {
        // 2 is INCREMENTAL.
        let auto_vacuum: i64 = self
            .conn
            .pragma_query_value(None, "auto_vacuum", |row| row.get(0))?;
        if auto_vacuum != 2 {
            return Err(StoreError::CannotReclaimSpace);
        }
        let tx = self.conn.transaction()?;
        {
            // Each step of the pragma frees one page, so it must be stepped to the end.
            let mut statement = tx.prepare(&format!("PRAGMA incremental_vacuum({max_pages})"))?;
            let mut steps = statement.query([])?;
            while steps.next()?.is_some() {}
        }
        tx.commit()?;
        let free_pages = self
            .conn
            .pragma_query_value(None, "freelist_count", |row| row.get(0))?;
        if free_pages == 0 {
            // The file only shrinks once the log is copied back into it; a busy reader just
            // postpones that to a later checkpoint.
            self.conn
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        Ok(free_pages)
    }

    /// The file this store reads and writes; `None` for an in-memory store.
    pub fn path(&self) -> Option<PathBuf> {
        self.conn
            .path()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
    }
}

/// Connection settings, each chosen by measurement on the scale fixture (see the `scale` example):
/// - `synchronous = FULL` and `fullfsync`: a committed transaction survives power loss. Under WAL
///   each commit syncs the log; macOS only flushes the drive's cache with `F_FULLFSYNC`.
/// - `cache_size` 64 MiB: scanning a 500,000-row view's values runs about a third faster than
///   with the 2 MiB default once warm. Pages are cached on demand, so small files use little.
/// - `journal_size_limit` 64 MiB: after a large transaction (a publish, a first join) the log is
///   cut back once checkpointed, instead of keeping its largest size (1 GB at a million rows).
/// - `temp_store = MEMORY`: sorts and temporary indexes never touch the disk.
/// - `analysis_limit`: bounds `optimize` to a sample of each index.
///
/// Memory mapping is left off: it gave no measurable gain over the page cache.
fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "PRAGMA synchronous = FULL;
         PRAGMA fullfsync = ON;
         PRAGMA checkpoint_fullfsync = ON;
         PRAGMA cache_size = -65536;
         PRAGMA journal_size_limit = 67108864;
         PRAGMA temp_store = MEMORY;
         PRAGMA analysis_limit = 400;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
