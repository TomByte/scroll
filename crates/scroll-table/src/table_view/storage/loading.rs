use std::ops::Range;
use std::time::Instant;

use gpui::{Context, ScrollStrategy};
use scroll_core::store::StoreError;
use scroll_core::view_query::TableRow;

use super::super::TableView;
use crate::table_model::{ReadRequest, ReadResult, window_around};

impl TableView {
    /// Re-reads the view, schema and rows in the background and shows them when they arrive.
    pub(in crate::table_view) fn load(&mut self, cx: &mut Context<Self>) {
        if self.background.is_writing || !self.background.writes.is_empty() {
            self.background.wants_load = true;
            return;
        }
        self.background.load_generation += 1;
        let generation = self.background.load_generation;
        let writes_at_start = self.background.writes_queued;
        let request = ReadRequest {
            view: self.model.view().id,
            visible: self.visible_rows.clone(),
            follow: self.model.row_id(self.selection.row),
            session_view: self.session_view.clone(),
        };
        let storage = self.storage.clone();
        let background = cx.background_executor().clone();
        self.is_loading = true;
        self.background.load = Some(cx.spawn(async move |table, cx| {
            let result = background
                .spawn(async move {
                    let started = Instant::now();
                    let result = storage.read(&request);
                    log::debug!("table load took {:?}", started.elapsed());
                    result
                })
                .await;
            // The table may have been closed while the load ran; then there is nothing to show.
            let _ = table.update(cx, |table, cx| {
                table.show_loaded(generation, writes_at_start, result, cx)
            });
        }));
        cx.notify();
    }

    /// Swaps in a load unless a newer one started or the table wrote something since it began.
    /// The selected cell stays on the row it was on, wherever that row now is.
    fn show_loaded(
        &mut self,
        generation: u64,
        writes_at_start: u64,
        result: Result<ReadResult, StoreError>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.background.load_generation {
            return;
        }
        if writes_at_start != self.background.writes_queued {
            self.load(cx);
            return;
        }
        self.is_loading = false;
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                self.show_error(&error);
                cx.notify();
                return;
            }
        };
        let replaced = std::mem::replace(&mut self.model, loaded.model);
        // Windows loading for the old row order no longer fit the new one.
        self.background.window = None;
        // Freeing a million row ids takes longer than a frame; do it off the UI thread.
        cx.background_executor()
            .spawn(async move { drop(replaced) })
            .detach();
        if let Some(row) = loaded.followed_row
            && row != self.selection.row
        {
            self.selection.row = row;
            self.anchor = self.selection;
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
        self.after_schema_change();
        cx.notify();
    }

    /// Called as rows come into view: loads the window around them unless it is loaded or a load
    /// for the screen is already running (it checks again when that finishes).
    pub(in crate::table_view) fn ensure_rows_loaded(
        &mut self,
        visible: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.visible_rows = visible.clone();
        if self.background.window.is_some() || self.model.missing_rows(visible).is_empty() {
            return;
        }
        let wanted = self.model.missing_rows(window_around(
            self.visible_rows.clone(),
            self.model.row_count(),
        ));
        let generation = self.background.load_generation;
        let writes_at_start = self.background.writes_queued;
        let storage = self.storage.clone();
        let background = cx.background_executor().clone();
        self.background.window = Some(cx.spawn(async move |table, cx| {
            let result = background
                .spawn(async move { storage.read_window(&wanted) })
                .await;
            // The table may have been closed while the window loaded.
            let _ = table.update(cx, |table, cx| {
                table.show_window(generation, writes_at_start, result, cx)
            });
        }));
    }

    /// Adds a loaded window unless the rows were re-queried or written to since it started.
    fn show_window(
        &mut self,
        generation: u64,
        writes_at_start: u64,
        result: Result<Vec<(usize, TableRow)>, StoreError>,
        cx: &mut Context<Self>,
    ) {
        self.background.window = None;
        let is_current = generation == self.background.load_generation
            && writes_at_start == self.background.writes_queued;
        match result {
            Ok(rows) if is_current => {
                self.model.insert_rows(rows, self.visible_rows.clone());
            }
            Ok(_) => {}
            Err(error) => {
                self.show_error(&error);
                cx.notify();
                return;
            }
        }
        self.ensure_rows_loaded(self.visible_rows.clone(), cx);
        cx.notify();
    }
}
