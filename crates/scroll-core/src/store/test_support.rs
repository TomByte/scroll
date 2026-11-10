//! Builders and readers shared by the store's tests.

use crate::id::{DatabaseId, PageId};
use crate::record::Page;
use crate::store::kinds::page::{PAGE_COLUMNS, page_from_row};
use crate::store::{QueuedChange, StoreChanges, StoreError, WorkspaceStore};
use crate::sync::WorkspaceSnapshot;

/// The whole queue, read in bounded steps as the sync engine reads it.
pub(crate) fn queued_changes(store: &WorkspaceStore) -> Result<Vec<QueuedChange>, StoreError> {
    let mut changes = Vec::new();
    let mut position = 0;
    loop {
        let batch = store.queued_changes_after(position, 100)?;
        let Some(last) = batch.last() else {
            return Ok(changes);
        };
        position = last.position;
        changes.extend(batch.into_iter().map(|entry| entry.change));
    }
}

/// Applies a whole snapshot through the staged calls, as one part.
pub(crate) fn apply_snapshot(
    store: &mut WorkspaceStore,
    snapshot: WorkspaceSnapshot,
) -> Result<StoreChanges, StoreError> {
    store.begin_workspace_snapshot(snapshot.seq)?;
    store.stage_workspace_snapshot_part(&snapshot)?;
    store.finish_workspace_snapshot(snapshot.seq)
}

/// Every row of the database, in position order.
pub(crate) fn rows_of_database(
    store: &WorkspaceStore,
    database: DatabaseId,
) -> Result<Vec<Page>, StoreError> {
    let sql = format!(
        "SELECT {PAGE_COLUMNS} FROM page WHERE row_of_database_id = ?1 ORDER BY position, id"
    );
    let pages = store
        .conn
        .prepare(&sql)?
        .query_map([database], page_from_row)?
        .collect::<Result<_, _>>()?;
    Ok(pages)
}

/// Pages directly under `parent` (`None` for top-level pages), in sibling order, archived ones
/// included and database rows left out.
pub(crate) fn child_pages(
    store: &WorkspaceStore,
    parent: Option<PageId>,
) -> Result<Vec<Page>, StoreError> {
    let sql = format!(
        "SELECT {PAGE_COLUMNS} FROM page WHERE parent_id IS ?1 AND row_of_database_id IS NULL
         ORDER BY position, id"
    );
    let pages = store
        .conn
        .prepare_cached(&sql)?
        .query_map([parent], page_from_row)?
        .collect::<Result<_, _>>()?;
    Ok(pages)
}
