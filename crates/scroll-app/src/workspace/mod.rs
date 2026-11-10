//! The open workspace as the views see it: the records loaded so far (never the whole
//! workspace), changed only through transactions applied here at once and then queued for the
//! store, and, for a workspace linked to a server, the sync engine that keeps it in step.

mod loading;
mod maintenance;
pub mod page_tree;
pub mod presence;
pub mod records;
pub mod settings;
pub mod storage;
mod sync;
pub mod sync_state;
pub mod tag_colors;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, EventEmitter, Global, Task};
use scroll_core::id::{DatabaseId, PageId, Role, UserId, ViewId};
use scroll_core::operation::RecordId;
use scroll_core::operation::Transaction;
use scroll_core::store::WorkspaceStore;
use scroll_sync::ConnectionId;
use scroll_sync::{ConnectionStatus, SyncEngine};

use crate::workspace::page_tree::{MoveError, PageDrop, PageTree};
use crate::workspace::records::WorkspaceRecords;
use crate::workspace::storage::{Storage, StorageError};

/// How long changes from elsewhere are gathered before the changed records are re-read, so a
/// burst of remote transactions costs one read.
const CHANGE_BATCH_DELAY: Duration = Duration::from_millis(100);
/// How long after opening the store's statistics are refreshed.
const OPTIMIZE_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    /// Records changed: refresh what shows them, apart from whatever made the change.
    RecordsChanged(ChangeSource),
    /// More of the workspace was read from the store (children, an opened page): redraw.
    Loaded,
    /// Something happened to an open page on the server side.
    Page(PageId, PageSyncEvent),
    ConnectionChanged,
    /// A short message for the user, e.g. that the server refused a change.
    Notice(String),
}

/// Where a change to the records came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeSource {
    /// A view in this window, through `Workspace`.
    Here,
    /// A table, through its own connection.
    Table,
    /// Another device, through the server.
    Server,
}

#[derive(Clone, Debug)]
pub enum PageSyncEvent {
    /// Content to import into the live document, in order.
    Updates(Vec<Vec<u8>>),
    /// The server will not take this device's edits: reload the page from the store.
    EditsRefused,
    Presence {
        user: UserId,
        connection: ConnectionId,
        state: Vec<u8>,
    },
    PresenceLeft {
        connection: ConnectionId,
    },
}

pub struct Workspace {
    path: PathBuf,
    storage: Arc<Storage>,
    records: WorkspaceRecords,
    sync: Option<Arc<SyncEngine>>,
    connection: Option<ConnectionStatus>,
    /// Local changes not yet on the server, while uploading.
    upload_remaining: usize,
    /// Parts (about a megabyte each) of a large first download received so far.
    downloaded_parts: Option<u32>,
    /// Changed records from elsewhere waiting to be re-read.
    pending_changes: Vec<RecordId>,
    change_batch: Task<()>,
    /// This person's role on the server, for a linked workspace whose membership is known.
    role: Option<Role>,
    optimizing: Task<()>,
    /// The download has arrived and the engine is writing it into the file.
    is_finishing_download: bool,
    /// The engine has not yet applied the workspace's first catch-up from its server.
    is_first_catch_up_pending: bool,
    /// How far that writing has got, as (applied, total) records, once the engine says.
    finish_progress: Option<(usize, usize)>,
    /// Databases on screen (a table or a row page), with how many views show each.
    open_databases: HashMap<DatabaseId, usize>,
    /// Pages open in a view, with how many views show each.
    open_pages: HashMap<PageId, usize>,
    _sync_events: Task<()>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

/// The workspace open in the window, for code outside the window that acts on it.
#[derive(Default)]
struct CurrentWorkspace(Option<Entity<Workspace>>);

impl Global for CurrentWorkspace {}

impl Workspace {
    pub fn current(cx: &App) -> Option<Entity<Workspace>> {
        cx.try_global::<CurrentWorkspace>()
            .and_then(|current| current.0.clone())
    }

    pub fn set_current(workspace: Option<Entity<Workspace>>, cx: &mut App) {
        cx.set_global(CurrentWorkspace(workspace));
    }

    pub fn new(path: PathBuf, storage: Arc<Storage>, records: WorkspaceRecords) -> Self {
        Workspace {
            path,
            storage,
            records,
            sync: None,
            connection: None,
            upload_remaining: 0,
            downloaded_parts: None,
            pending_changes: Vec::new(),
            change_batch: Task::ready(()),
            open_pages: HashMap::new(),
            open_databases: HashMap::new(),
            optimizing: Task::ready(()),
            is_finishing_download: false,
            is_first_catch_up_pending: false,
            finish_progress: None,
            role: None,
            _sync_events: Task::ready(()),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn tree(&self) -> &PageTree {
        &self.records.tree
    }

    pub fn records(&self) -> &WorkspaceRecords {
        &self.records
    }

    pub fn storage(&self) -> &Arc<Storage> {
        &self.storage
    }

    /// `None` for a workspace that lives only on this device.
    pub fn connection(&self) -> Option<ConnectionStatus> {
        self.connection
    }

    /// Whether this person may only read the workspace (a viewer on its server).
    pub fn is_read_only(&self) -> bool {
        self.role.is_some_and(|role| !role.can_edit())
    }

    /// The person's role on the server, once known; viewers get read-only pages.
    pub fn set_role(&mut self, role: Option<Role>, cx: &mut Context<Self>) {
        if self.role != role {
            self.role = role;
            cx.emit(WorkspaceEvent::ConnectionChanged);
            cx.notify();
        }
    }

    pub fn is_linked(&self) -> bool {
        self.sync.is_some() || self.connection.is_some()
    }

    /// Queues a write behind earlier ones; a linked workspace's engine is told once it lands.
    pub fn write(
        &self,
        what: &'static str,
        job: impl FnOnce(&mut WorkspaceStore) -> Result<(), StorageError> + Send + 'static,
    ) {
        let engine = self.sync.clone();
        self.storage.write(what, move |store| {
            job(store)?;
            if let Some(engine) = engine {
                engine.local_change_queued();
            }
            Ok(())
        });
    }

    /// Resolves once every write queued so far has reached the file, so another connection
    /// (the table's) can read it.
    pub fn flushed(&self, cx: &gpui::App) -> Task<Result<(), StorageError>> {
        self.storage.read(cx, |_| Ok(()))
    }

    /// Applies `transaction` in memory now and in the store in order behind earlier ones.
    pub fn commit(&mut self, transaction: Transaction, cx: &mut Context<Self>) {
        self.records.apply(&transaction);
        self.write("save changes", move |store| {
            storage::apply(store, &transaction)
        });
        cx.emit(WorkspaceEvent::RecordsChanged(ChangeSource::Here));
        cx.notify();
    }

    /// Creates an untitled page as the last child of `parent`, loading the parent's children
    /// first if they are not loaded, and resolves to the new page.
    pub fn create_page(
        &mut self,
        parent: Option<PageId>,
        cx: &mut Context<Self>,
    ) -> Task<Option<PageId>> {
        let creating =
            self.after_children_loaded(parent, cx, move |workspace, cx| {
                match workspace.records.tree.create_page(parent, "") {
                    Ok((id, transaction)) => {
                        workspace.commit(transaction, cx);
                        Some(id)
                    }
                    Err(error) => {
                        log::error!("could not place a new page: {error}");
                        None
                    }
                }
            });
        cx.spawn(async move |_, _| creating.await.flatten())
    }

    /// Creates a page holding a new, empty database under `parent`; resolves to the page.
    pub fn create_database(
        &mut self,
        parent: Option<PageId>,
        cx: &mut Context<Self>,
    ) -> Task<Option<PageId>> {
        let creating = self.after_children_loaded(parent, cx, move |workspace, cx| match workspace
            .records
            .create_database(parent)
        {
            Ok((id, transaction)) => {
                workspace.commit(transaction, cx);
                Some(id)
            }
            Err(error) => {
                log::error!("could not place a new database: {error}");
                None
            }
        });
        cx.spawn(async move |_, _| creating.await.flatten())
    }

    pub fn rename(&mut self, id: PageId, title: &str, cx: &mut Context<Self>) {
        let is_unchanged = self.tree().page(id).is_some_and(|page| page.title == title);
        if !is_unchanged {
            let transaction = Transaction::set_title(id, title);
            self.commit(transaction, cx);
        }
    }

    /// Moves a page, first loading the destination's children if they are not loaded.
    pub fn move_page(&mut self, id: PageId, drop: PageDrop, cx: &mut Context<Self>) {
        match self.records.tree.move_page(id, drop) {
            Ok(transaction) => self.commit(transaction, cx),
            Err(MoveError::ChildrenNotLoaded) => {
                let parent = match drop {
                    PageDrop::Inside(target) => Some(target),
                    PageDrop::Before(target) | PageDrop::After(target) => {
                        self.records.tree.page(target).and_then(|page| page.parent)
                    }
                };
                self.after_children_loaded(parent, cx, move |workspace, cx| {
                    match workspace.records.tree.move_page(id, drop) {
                        Ok(transaction) => workspace.commit(transaction, cx),
                        Err(error) => log::warn!("page not moved: {error}"),
                    }
                })
                .detach();
            }
            Err(error) => log::warn!("page not moved: {error}"),
        }
    }

    /// Tags `page` with `name`. The store is asked for a tag of that name first (it may have
    /// arrived from another device and not be loaded yet), because creating a second tag with
    /// an existing name is rejected by the server.
    pub fn add_tag(&mut self, page: PageId, name: &str, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let looking = {
            let name = name.clone();
            self.storage
                .read(cx, move |store| Ok(store.tag_named(&name)?))
        };
        cx.spawn(async move |workspace, cx| {
            let existing = match looking.await {
                Ok(existing) => existing,
                Err(error) => return log::error!("could not look up the tag: {error}"),
            };
            let tagged = workspace.update(cx, |workspace, cx| {
                if let Some(tag) = existing {
                    workspace.records.remember_tag(tag);
                }
                let color = tag_colors::next_tag_color(workspace.records.tags_with_counts().len());
                let transaction = workspace.records.add_tag(page, &name, color);
                if !transaction.operations.is_empty() {
                    workspace.commit(transaction, cx);
                }
            });
            if tagged.is_err() {
                log::debug!("workspace closed before the tag was added");
            }
        })
        .detach();
    }

    pub fn set_icon(&mut self, id: PageId, icon: Option<String>, cx: &mut Context<Self>) {
        let transaction = self.records.tree.set_icon(id, icon);
        self.commit(transaction, cx);
    }

    /// Moves `id` to the trash.
    pub fn archive(&mut self, id: PageId, cx: &mut Context<Self>) {
        let transaction = self.records.tree.set_archived(id, true);
        self.commit(transaction, cx);
    }

    /// Brings `id` back from the trash.
    pub fn restore(&mut self, id: PageId, cx: &mut Context<Self>) {
        let transaction = self.records.tree.set_archived(id, false);
        self.commit(transaction, cx);
    }

    pub fn add_view(
        &mut self,
        database: DatabaseId,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Option<ViewId> {
        let (view, transaction) = self.records.add_view(database, name)?;
        self.commit(transaction, cx);
        Some(view)
    }

    /// A table wrote through its own connection: tell the engine, and re-read the open pages
    /// (a row's property panel may show what the table changed). Nothing else is re-read.
    pub fn table_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = self.sync.as_ref() {
            engine.local_change_queued();
        }
        let open: Vec<RecordId> = self
            .open_pages
            .keys()
            .map(|page| RecordId::Page(*page))
            .collect();
        self.read_changed(open, ChangeSource::Table, cx);
    }

    /// A view shows `page`'s content: keep its values current and tell the engine.
    pub fn open_page(&mut self, page: PageId) {
        *self.open_pages.entry(page).or_default() += 1;
        if let Some(engine) = self.sync.as_ref() {
            engine.open_page(page);
        }
    }

    pub fn close_page(&mut self, page: PageId) {
        if let Some(count) = self.open_pages.get_mut(&page) {
            *count -= 1;
            if *count == 0 {
                self.open_pages.remove(&page);
            }
        }
        if let Some(engine) = self.sync.as_ref() {
            engine.close_page(page);
        }
    }

    /// A table of `database`, or one of its row pages, is on screen: the server then sends
    /// that database's row changes at once instead of up to a second later.
    pub fn open_database(&mut self, database: DatabaseId) {
        let count = self.open_databases.entry(database).or_default();
        *count += 1;
        if *count == 1
            && let Some(engine) = self.sync.as_ref()
        {
            engine.open_database(database);
        }
    }

    pub fn close_database(&mut self, database: DatabaseId) {
        let Some(count) = self.open_databases.get_mut(&database) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        self.open_databases.remove(&database);
        if let Some(engine) = self.sync.as_ref() {
            engine.close_database(database);
        }
    }

    #[cfg(test)]
    pub fn is_database_open(&self, database: DatabaseId) -> bool {
        self.open_databases.contains_key(&database)
    }
}

/// How many unused pages one reclaiming write returns; each is a short write, so the sync
/// engine and the window's own writes interleave with them.
const RECLAIM_PAGES: usize = 2_048;

impl Drop for Workspace {
    fn drop(&mut self) {
        self.shut_down();
    }
}
