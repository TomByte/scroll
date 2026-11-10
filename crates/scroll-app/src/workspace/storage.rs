//! The workspace file, owned by one background thread. Every read and write is a job run on
//! that thread in submission order, so a read always sees earlier writes and the main thread
//! never waits on SQLite.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use gpui::{App, Task};
use scroll_core::id::PageId;
use scroll_core::operation::RecordId;
use scroll_core::operation::Transaction;
use scroll_core::page_doc::PageDoc;
use scroll_core::record::Page;
use scroll_core::store::{SearchHit, StoreError, WorkspaceStore};

use crate::workspace::records::{ChangedRecords, ChildrenBatch, PageDetails, WorkspaceRecords};

type Job = Box<dyn FnOnce(&mut WorkspaceStore) + Send>;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the storage thread has stopped")]
    Stopped,
    #[error("could not start the storage thread: {0}")]
    Thread(std::io::Error),
}

pub struct Storage {
    jobs: Mutex<Option<Sender<Job>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Arc<Storage>, StorageError> {
        let store = WorkspaceStore::open(path)?;
        let (sender, receiver) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("workspace-storage".into())
            .spawn(move || run_jobs(store, receiver))
            .map_err(StorageError::Thread)?;
        Ok(Arc::new(Storage {
            jobs: Mutex::new(Some(sender)),
            thread: Mutex::new(Some(thread)),
        }))
    }

    /// Queues a write. Failures are logged with `what` for context, since nothing waits on them.
    pub fn write(
        &self,
        what: &'static str,
        job: impl FnOnce(&mut WorkspaceStore) -> Result<(), StorageError> + Send + 'static,
    ) {
        let submitted = self.submit(Box::new(move |store| {
            if let Err(error) = job(store) {
                log::error!("failed to {what}: {error}");
            }
        }));
        if !submitted {
            log::error!("failed to {what}: the storage thread has stopped");
        }
    }

    /// Queues a read and returns its result. The background task blocks one executor thread
    /// on the reply, never the main thread.
    pub fn read<R: Send + 'static>(
        &self,
        cx: &App,
        job: impl FnOnce(&mut WorkspaceStore) -> Result<R, StorageError> + Send + 'static,
    ) -> Task<Result<R, StorageError>> {
        let (reply, result) = mpsc::channel();
        let submitted = self.submit(Box::new(move |store| {
            // The reader may have gone away; nothing to do then.
            let _ = reply.send(job(store));
        }));
        cx.background_executor().spawn(async move {
            if !submitted {
                return Err(StorageError::Stopped);
            }
            result.recv().unwrap_or(Err(StorageError::Stopped))
        })
    }

    /// Finishes every queued job and stops the thread. Called on quit.
    pub fn shut_down(&self) {
        drop(
            self.jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(thread) = thread
            && thread.join().is_err()
        {
            log::error!("the storage thread panicked while shutting down");
        }
    }

    fn submit(&self, job: Job) -> bool {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok())
    }
}

fn run_jobs(mut store: WorkspaceStore, jobs: Receiver<Job>) {
    for job in jobs {
        job(&mut store);
    }
}

/// What the window needs at launch: the top level of the tree and every tag. Nothing else is
/// read until it is shown.
pub fn load_initial(store: &WorkspaceStore) -> Result<WorkspaceRecords, StorageError> {
    let mut records = WorkspaceRecords::default();
    let reading = std::time::Instant::now();
    // The first batch of the top level; the window asks for the rest once it is open.
    records.load_children(load_children(store, None, None)?);
    log::debug!("timing: top-level pages read in {:?}", reading.elapsed());
    let reading = std::time::Instant::now();
    records.load_tags(store.tags()?);
    log::debug!("timing: tags read in {:?}", reading.elapsed());
    Ok(records)
}

/// How many children are read at a time.
pub const CHILDREN_BATCH: usize = 2_000;

/// Up to `CHILDREN_BATCH` of a parent's children after `after`, and the databases they host.
pub fn load_children(
    store: &WorkspaceStore,
    parent: Option<PageId>,
    after: Option<PageId>,
) -> Result<ChildrenBatch, StorageError> {
    let listed = store.page_children(parent, after, CHILDREN_BATCH)?;
    // Only the few children that host a database need their database read, for the icon.
    let mut databases = Vec::new();
    for hosted in listed.iter().filter_map(|child| child.hosted_database) {
        if let Some(database) = store.database(hosted)? {
            let views = store.views(database.id)?;
            databases.push((database, views));
        }
    }
    let children = listed
        .into_iter()
        .map(|child| (child.page, child.child_count))
        .collect();
    Ok(ChildrenBatch {
        parent,
        children,
        databases,
    })
}

/// The page, its ancestors, its database (hosted or of its row), values and tags.
pub fn load_page_details(
    store: &WorkspaceStore,
    page: PageId,
) -> Result<PageDetails, StorageError> {
    let mut chain = Vec::new();
    let mut next = Some(page);
    while let Some(id) = next {
        let Some(stored) = store.page(id)? else {
            break;
        };
        next = stored
            .parent
            .filter(|parent| !chain.iter().any(|seen: &Page| seen.id == *parent));
        chain.push(stored);
    }
    let mut databases = Vec::new();
    if let Some(database) = store.database_hosted_by(page)? {
        let views = store.views(database.id)?;
        databases.push((database, views));
    }
    let row_of = chain.first().and_then(|stored| stored.row_of);
    if let Some(database) = row_of.map(|id| store.database(id)).transpose()?.flatten() {
        let views = store.views(database.id)?;
        databases.push((database, views));
    }
    Ok(PageDetails {
        page,
        chain,
        databases,
        values: store.property_values(page)?,
        tags: store.page_tags(page)?,
    })
}

/// Which records changed elsewhere, and what the window has loaded that they might affect.
pub struct ChangeQuery {
    pub ids: Vec<RecordId>,
    pub open_pages: Vec<PageId>,
    pub loaded_databases: Vec<scroll_core::id::DatabaseId>,
}

/// Fresh copies of the changed records; values and tags only for open pages.
pub fn read_changes(
    store: &WorkspaceStore,
    query: &ChangeQuery,
) -> Result<ChangedRecords, StorageError> {
    let open_pages = &query.open_pages;
    let mut changes = ChangedRecords::default();
    let mut are_tags_changed = false;
    for id in &query.ids {
        match id {
            RecordId::Page(page) => {
                let stored = store
                    .pages_by_id(&[*page])?
                    .pop()
                    .map(|child| (child.page, child.child_count));
                changes.pages.push((*page, stored));
                if open_pages.contains(page) {
                    changes.values.push((*page, store.property_values(*page)?));
                    changes.page_tags.push((*page, store.page_tags(*page)?));
                }
            }
            RecordId::Database(database) => {
                let stored = match store.database(*database)? {
                    Some(stored) => {
                        let views = store.views(stored.id)?;
                        Some((stored, views))
                    }
                    None => None,
                };
                changes.databases.push((*database, stored));
            }
            RecordId::View(view) => {
                // A deleted view no longer says its database, so re-read every loaded one.
                let databases = match store.view(*view)? {
                    Some(view) => vec![view.database],
                    None => query.loaded_databases.clone(),
                };
                for database in databases {
                    if let Some(database) = store.database(database)? {
                        let views = store.views(database.id)?;
                        changes
                            .databases
                            .push((database.id, Some((database, views))));
                    }
                }
            }
            RecordId::Tag(_) => are_tags_changed = true,
            RecordId::PageTag { page, .. } => {
                are_tags_changed = true;
                if open_pages.contains(page) {
                    changes.page_tags.push((*page, store.page_tags(*page)?));
                }
            }
        }
    }
    if are_tags_changed {
        changes.tags = Some(store.tags()?);
    }
    Ok(changes)
}

pub fn apply(store: &mut WorkspaceStore, transaction: &Transaction) -> Result<(), StorageError> {
    store.apply_local_transaction(transaction)?;
    Ok(())
}

/// The page's document as stored, including local edits still waiting for the server.
pub fn load_page(store: &WorkspaceStore, page: PageId) -> Result<PageDoc, StorageError> {
    Ok(store.load_page_doc(page)?)
}

pub fn record_updates(
    store: &mut WorkspaceStore,
    page: PageId,
    updates: &[Vec<u8>],
) -> Result<(), StorageError> {
    for update in updates {
        store.record_local_page_update(page, update)?;
    }
    Ok(())
}

/// Folds the page's stored updates into one snapshot and refreshes its search text.
pub fn compact(store: &mut WorkspaceStore, page: PageId) -> Result<(), StorageError> {
    Ok(store.compact_page(page)?)
}

pub fn search(
    store: &WorkspaceStore,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, StorageError> {
    Ok(store.search(query, limit)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::{BlockKind, PageDocument};
    use crate::workspace::page_tree::PageTree;

    fn temporary_store() -> (WorkspaceStore, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("scroll-store-{}.sqlite", PageId::generate().0));
        (WorkspaceStore::open(&path).unwrap(), path)
    }

    #[test]
    fn pages_and_their_content_survive_a_reopen_and_become_searchable() {
        let (mut store, path) = temporary_store();
        let tree = PageTree::default();
        let (page_id, transaction) = tree.create_page(None, "Garden").unwrap();
        apply(&mut store, &transaction).unwrap();

        let mut page = PageDocument::empty();
        let block = page.ensure_block().unwrap();
        page.insert_text(block, 0, "tomatoes and basil").unwrap();
        page.set_kind(block, BlockKind::Heading1).unwrap();
        page.commit();
        record_updates(&mut store, page_id, &page.take_outgoing_updates()).unwrap();
        drop(store);

        let mut reopened = WorkspaceStore::open(&path).unwrap();
        let pages: Vec<Page> = reopened
            .page_children(None, None, CHILDREN_BATCH)
            .unwrap()
            .into_iter()
            .map(|child| child.page)
            .collect();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Garden");
        let loaded = PageDocument::new(load_page(&reopened, page_id).unwrap());
        assert_eq!(
            loaded.block_content(loaded.block_ids()[0]).unwrap().text,
            "tomatoes and basil"
        );
        assert_eq!(loaded.blocks()[0].kind, BlockKind::Heading1);

        compact(&mut reopened, page_id).unwrap();
        let hits = search(&reopened, "basil", 10).unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.page_id).collect::<Vec<_>>(),
            vec![page_id]
        );

        let mut after_compaction = PageDocument::new(load_page(&reopened, page_id).unwrap());
        after_compaction
            .insert_text(after_compaction.block_ids()[0], 0, "Ripe ")
            .unwrap();
        after_compaction.commit();
        record_updates(
            &mut reopened,
            page_id,
            &after_compaction.take_outgoing_updates(),
        )
        .unwrap();
        let loaded = PageDocument::new(load_page(&reopened, page_id).unwrap());
        assert_eq!(
            loaded.block_content(loaded.block_ids()[0]).unwrap().text,
            "Ripe tomatoes and basil"
        );
        drop(reopened);
        std::fs::remove_file(path).ok();
    }
}
