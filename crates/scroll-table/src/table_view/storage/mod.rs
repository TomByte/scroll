//! The table's own connections to the workspace file.
//!
//! The table is deliberately a second reader and writer of the file, beside the app's
//! `WorkspaceStore`: it opens two read connections and one write connection of its own and uses
//! them only on background threads, so the UI thread never waits on SQLite and a million-row
//! query never blocks the app's own reads and writes. SQLite's WAL mode lets the readers run
//! while either writer writes; the store's busy handler makes the two writers take turns. The
//! app learns of the table's writes from `TableEvent::Changed`, and tells the table about other
//! changes with `TableView::refresh`.
//!
//! - `loading`: loads read the view, its schema, the ids of the rows it shows and the rows near
//!   the screen together. A newer load supersedes an older one, and a load that started before
//!   one of the table's own writes is discarded and repeated after the writes finish, so it never
//!   shows data older than what is on screen. As the person scrolls, the rows coming into view
//!   are loaded on a connection of their own, one window at a time; until then their places show
//!   placeholders.
//! - `writing`: writes go out one at a time, in the order they were made. Each change is applied
//!   to the model first (`TableModel::apply`), so the screen never waits for the write. If a
//!   write fails, its error is shown and the table reloads what the store has.
//! - `resort`: after an edit that can move rows, the query re-runs once the person stops editing.

mod loading;
mod resort;
mod writing;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use gpui::Task;
use scroll_core::id::PageId;
use scroll_core::operation::Transaction;
use scroll_core::store::{StoreError, WorkspaceStore};
use scroll_core::view_query::TableRow;

use crate::table_model::{ReadRequest, ReadResult, TableModel, read_rows};

#[derive(Clone)]
pub(super) struct TableStorage {
    path: PathBuf,
    reader: Arc<Mutex<Option<WorkspaceStore>>>,
    /// Loads rows while scrolling, so they never wait behind a re-sort on `reader`.
    window_reader: Arc<Mutex<Option<WorkspaceStore>>>,
    writer: Arc<Mutex<Option<WorkspaceStore>>>,
}

impl TableStorage {
    pub(in crate::table_view) fn new(path: PathBuf) -> Self {
        TableStorage {
            path,
            reader: Arc::default(),
            window_reader: Arc::default(),
            writer: Arc::default(),
        }
    }

    fn read(&self, request: &ReadRequest) -> Result<ReadResult, StoreError> {
        let mut reader = lock(&self.reader);
        let store = match reader.as_mut() {
            Some(store) => store,
            None => reader.insert(WorkspaceStore::open_reader(&self.path)?),
        };
        TableModel::read(store, request)
    }

    pub(in crate::table_view) fn read_window(
        &self,
        wanted: &[(usize, PageId)],
    ) -> Result<Vec<(usize, TableRow)>, StoreError> {
        let mut reader = lock(&self.window_reader);
        let store = match reader.as_mut() {
            Some(store) => store,
            None => reader.insert(WorkspaceStore::open_reader(&self.path)?),
        };
        read_rows(store, wanted)
    }

    fn write(&self, transaction: &Transaction) -> Result<(), StoreError> {
        let mut writer = lock(&self.writer);
        let store = match writer.as_mut() {
            Some(store) => store,
            None => writer.insert(WorkspaceStore::open(&self.path)?),
        };
        store.apply_local_transaction(transaction)?;
        Ok(())
    }
}

/// A poisoned lock means an earlier use panicked; the connection itself is still sound.
fn lock(
    connection: &Mutex<Option<WorkspaceStore>>,
) -> std::sync::MutexGuard<'_, Option<WorkspaceStore>> {
    connection.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Background work in flight. Replacing a task drops it, which cancels it.
#[derive(Default)]
pub(super) struct BackgroundWork {
    load_generation: u64,
    load: Option<Task<()>>,
    /// The window of rows being loaded for the screen, if any.
    window: Option<Task<()>>,
    resort: Option<Task<()>>,
    writes: VecDeque<Transaction>,
    is_writing: bool,
    /// Counts writes queued, so a load can tell whether one was made while it ran.
    writes_queued: u64,
    /// A load was asked for while writes were pending; it runs when they finish.
    wants_load: bool,
}
