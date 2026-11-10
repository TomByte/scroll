//! Which rows are loaded. `read` finds the view's rows (ids only, so a million rows cost a few
//! megabytes) and loads the rows around a given place; `read_rows` loads more as the person
//! scrolls, and `insert_rows` keeps at most `MAX_LOADED_ROWS`, dropping those farthest from the
//! screen.

use std::collections::HashMap;
use std::ops::Range;

use scroll_core::id::{PageId, ViewId};
use scroll_core::record::View;
use scroll_core::store::{StoreError, WorkspaceStore};
use scroll_core::view_query::TableRow;

use super::{LoadedRow, TableModel};

/// The most rows kept loaded; about ten screens.
pub const MAX_LOADED_ROWS: usize = 1_000;

/// What a load should bring along besides the row ids.
#[derive(Debug, Clone)]
pub struct ReadRequest {
    pub view: ViewId,
    /// The rows on screen when the load started; their contents are loaded too.
    pub visible: Range<usize>,
    /// A row to find again: its contents are loaded wherever it now is, instead of `visible`.
    pub follow: Option<PageId>,
    /// The view as changed in this session without being stored (a viewer's sort or filter);
    /// `None` reads the stored view.
    pub session_view: Option<View>,
}

/// A loaded table and where the followed row ended up.
pub struct ReadResult {
    pub model: TableModel,
    pub followed_row: Option<usize>,
}

impl TableModel {
    /// The view, its schema, its row ids, and the contents of the rows the request names.
    pub fn read(store: &WorkspaceStore, request: &ReadRequest) -> Result<ReadResult, StoreError> {
        let ids = match &request.session_view {
            Some(view) => store.rows_for_view(view.clone())?,
            None => store.view_row_ids(request.view)?,
        };
        let followed_row = request
            .follow
            .and_then(|page| ids.rows.iter().position(|row| *row == page));
        let visible_len = request.visible.len().max(1);
        let around = match followed_row {
            Some(row) => row.saturating_sub(visible_len / 2)..row + visible_len,
            None => request.visible.clone(),
        };
        let mut model = TableModel {
            last_position: store.last_row_position(ids.database.id)?,
            view: ids.view,
            database: ids.database,
            order: ids.rows,
            loaded: HashMap::new(),
        };
        let wanted = model.missing_rows(window_around(around, model.order.len()));
        let rows = read_rows(store, &wanted)?;
        model.insert_rows(rows, 0..0);
        Ok(ReadResult {
            model,
            followed_row,
        })
    }

    /// The rows in `range` not loaded yet, with their places.
    pub fn missing_rows(&self, range: Range<usize>) -> Vec<(usize, PageId)> {
        let end = range.end.min(self.order.len());
        let start = range.start.min(end);
        (start..end)
            .map(|index| (index, self.order[index]))
            .filter(|(_, page)| !self.loaded.contains_key(page))
            .collect()
    }

    /// Adds loaded rows (those already loaded keep their contents, which may hold newer edits),
    /// then drops the rows farthest from `keep` while more than `MAX_LOADED_ROWS` are loaded.
    pub fn insert_rows(&mut self, rows: Vec<(usize, TableRow)>, keep: Range<usize>) {
        for (index, row) in rows {
            self.loaded
                .entry(row.page.id)
                .or_insert(LoadedRow { row, index });
        }
        let excess = self.loaded.len().saturating_sub(MAX_LOADED_ROWS);
        if excess == 0 {
            return;
        }
        let distance = |index: usize| {
            if index < keep.start {
                keep.start - index
            } else {
                index.saturating_sub(keep.end)
            }
        };
        let mut by_distance: Vec<(usize, PageId)> = self
            .loaded
            .iter()
            .map(|(page, loaded)| (distance(loaded.index), *page))
            .collect();
        by_distance.sort_unstable_by_key(|(distance, _)| std::cmp::Reverse(*distance));
        for (_, page) in by_distance.into_iter().take(excess) {
            self.loaded.remove(&page);
        }
    }
}

/// Loads the given rows' contents, keeping each with its place. Rows deleted meanwhile are left
/// out.
pub fn read_rows(
    store: &WorkspaceStore,
    wanted: &[(usize, PageId)],
) -> Result<Vec<(usize, TableRow)>, StoreError> {
    let ids: Vec<PageId> = wanted.iter().map(|(_, page)| *page).collect();
    let places: HashMap<PageId, usize> =
        wanted.iter().map(|(index, page)| (*page, *index)).collect();
    let rows = store.table_rows(&ids)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((*places.get(&row.page.id)?, row)))
        .collect())
}

/// How many rows beyond those on screen to load on each side, so scrolling a little shows rows
/// at once.
pub const WINDOW_MARGIN: usize = 100;

/// The rows to load for a screen showing `visible`.
pub fn window_around(visible: Range<usize>, row_count: usize) -> Range<usize> {
    let start = visible.start.saturating_sub(WINDOW_MARGIN);
    let end = (visible.end + WINDOW_MARGIN).min(row_count);
    start.min(end)..end
}
