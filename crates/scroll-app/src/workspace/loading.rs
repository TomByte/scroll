//! Reading records into memory when they are first shown, and again when they change elsewhere.

use gpui::{Context, Task};
use scroll_core::id::PageId;
use scroll_core::operation::RecordId;
use std::time::Instant;

use super::*;

impl Workspace {
    /// Runs `then` once `parent`'s children are loaded: at once if they are.
    pub(super) fn after_children_loaded<T: 'static>(
        &mut self,
        parent: Option<PageId>,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) -> T + 'static,
    ) -> Task<Option<T>> {
        if self.records.tree.are_children_loaded(parent) {
            return Task::ready(Some(then(self, cx)));
        }
        let loading = self.load_children(parent, cx);
        cx.spawn(async move |workspace, cx| {
            loading.await;
            workspace
                .update(cx, |workspace, cx| then(workspace, cx))
                .ok()
        })
    }

    /// Reads `parent`'s children (and the databases they host) on the storage thread, a batch
    /// at a time: each batch is taken in as its own small step, so a parent with 50,000
    /// children fills in without stalling the UI. Resolves once the first batch is in.
    pub fn load_children(&mut self, parent: Option<PageId>, cx: &mut Context<Self>) -> Task<()> {
        let first = self.read_children_batch(parent, None, cx);
        cx.spawn(async move |workspace, cx| {
            let Some(mut last) = first.await.flatten() else {
                return;
            };
            loop {
                let next = workspace.update(cx, |workspace, cx| {
                    workspace.read_children_batch(parent, Some(last), cx)
                });
                let Ok(next) = next else {
                    return;
                };
                let Some(Some(next_last)) = next.await else {
                    return;
                };
                last = next_last;
            }
        })
    }

    /// Reads and takes in one batch of children after `after`; resolves to the last child if
    /// the batch was full (so there may be more).
    pub(super) fn read_children_batch(
        &mut self,
        parent: Option<PageId>,
        after: Option<PageId>,
        cx: &mut Context<Self>,
    ) -> Task<Option<Option<PageId>>> {
        let asked = Instant::now();
        let reading = self.storage.read(cx, move |store| {
            storage::load_children(store, parent, after)
        });
        cx.spawn(async move |workspace, cx| {
            let batch = match reading.await {
                Ok(batch) => batch,
                Err(error) => {
                    log::error!("could not read pages: {error}");
                    return None;
                }
            };
            let read_time = asked.elapsed();
            workspace
                .update(cx, |workspace, cx| {
                    let applying = Instant::now();
                    let count = batch.children.len();
                    let last = (count == storage::CHILDREN_BATCH)
                        .then(|| batch.children.last().map(|(page, _)| page.id))
                        .flatten();
                    if after.is_some() {
                        workspace.records.append_children(batch);
                    } else {
                        workspace.records.load_children(batch);
                    }
                    cx.emit(WorkspaceEvent::Loaded);
                    cx.notify();
                    log::debug!(
                        "timing: {count} children read in {read_time:?}, taken in on the UI thread in {:?}",
                        applying.elapsed()
                    );
                    last
                })
                .ok()
        })
    }

    /// Reads what showing `page` needs; resolves to whether the page exists.
    pub fn load_page(&mut self, page: PageId, cx: &mut Context<Self>) -> Task<bool> {
        let reading = self
            .storage
            .read(cx, move |store| storage::load_page_details(store, page));
        cx.spawn(async move |workspace, cx| {
            let details = match reading.await {
                Ok(details) => details,
                Err(error) => {
                    log::error!("could not read page {page:?}: {error}");
                    return false;
                }
            };
            let exists = !details.chain.is_empty();
            let loaded = workspace.update(cx, |workspace, cx| {
                workspace.records.load_page_details(details);
                cx.emit(WorkspaceEvent::Loaded);
                cx.notify();
            });
            exists && loaded.is_ok()
        })
    }

    /// Re-reads records changed elsewhere, gathering a burst into one read.
    pub(super) fn records_changed_elsewhere(&mut self, ids: Vec<RecordId>, cx: &mut Context<Self>) {
        let is_first = self.pending_changes.is_empty();
        self.pending_changes.extend(ids);
        if !is_first {
            return;
        }
        self.change_batch = cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(CHANGE_BATCH_DELAY).await;
            let read = workspace.update(cx, |workspace, cx| {
                let mut ids = std::mem::take(&mut workspace.pending_changes);
                ids.sort_by_key(|id| format!("{id:?}"));
                ids.dedup();
                workspace.read_changed(ids, ChangeSource::Server, cx);
            });
            if read.is_err() {
                log::debug!("workspace closed before changes were read");
            }
        });
    }

    pub(super) fn read_changed(
        &mut self,
        ids: Vec<RecordId>,
        source: ChangeSource,
        cx: &mut Context<Self>,
    ) {
        let query = storage::ChangeQuery {
            ids,
            open_pages: self.open_pages.keys().copied().collect(),
            loaded_databases: self.records.loaded_databases(),
        };
        let reading = self
            .storage
            .read(cx, move |store| storage::read_changes(store, &query));
        cx.spawn(async move |workspace, cx| {
            let changes = match reading.await {
                Ok(changes) => changes,
                Err(error) => return log::error!("could not read changed records: {error}"),
            };
            let applied = workspace.update(cx, |workspace, cx| {
                let applying = Instant::now();
                let count = changes.pages.len();
                workspace.records.apply_changes(changes);
                cx.emit(WorkspaceEvent::RecordsChanged(source));
                cx.notify();
                log::debug!(
                    "timing: {count} changed pages applied on the UI thread in {:?}",
                    applying.elapsed()
                );
            });
            if applied.is_err() {
                log::debug!("workspace closed before changes applied");
            }
        })
        .detach();
    }

    /// The server replaced everything: re-read every loaded list, open page and the tags.
    pub(super) fn reload_everything(&mut self, cx: &mut Context<Self>) {
        let parents = self.records.tree.loaded_parents();
        let pages: Vec<PageId> = self.open_pages.keys().copied().collect();
        let reading = self.storage.read(cx, move |store| {
            let mut records = crate::workspace::records::WorkspaceRecords::default();
            for parent in parents {
                let mut after = None;
                loop {
                    let batch = storage::load_children(store, parent, after)?;
                    let is_full = batch.children.len() == storage::CHILDREN_BATCH;
                    let last = batch.children.last().map(|(page, _)| page.id);
                    if after.is_some() {
                        records.append_children(batch);
                    } else {
                        records.load_children(batch);
                    }
                    if !is_full {
                        break;
                    }
                    after = last;
                }
            }
            for page in pages {
                records.load_page_details(storage::load_page_details(store, page)?);
            }
            records.load_tags(store.tags()?);
            Ok(records)
        });
        cx.spawn(async move |workspace, cx| {
            let records = match reading.await {
                Ok(records) => records,
                Err(error) => return log::error!("could not re-read the workspace: {error}"),
            };
            let applied = workspace.update(cx, |workspace, cx| {
                workspace.records = records;
                cx.emit(WorkspaceEvent::RecordsChanged(ChangeSource::Server));
                cx.notify();
            });
            if applied.is_err() {
                log::debug!("workspace closed before it was re-read");
            }
        })
        .detach();
    }
}

/// The changed records plus the parents whose child lists changed: re-reading a parent page
/// brings its new child count, which is what a collapsed parent shows.
pub(super) fn with_parents(
    mut records: Vec<RecordId>,
    child_lists: Vec<Option<PageId>>,
) -> Vec<RecordId> {
    records.extend(child_lists.into_iter().flatten().map(RecordId::Page));
    records
}
