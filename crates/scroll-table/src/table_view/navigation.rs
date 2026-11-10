//! Moving the selection, the clipboard, and adding, deleting and opening rows.

use std::collections::HashMap;

use gpui::{ClipboardItem, Context, KeyDownEvent, ScrollStrategy, ScrollWheelEvent, Window};
use scroll_core::id::PageId;
use scroll_core::operation::{Operation, Transaction};
use scroll_core::record::value_for;
use scroll_core::view_query::TableRow;

use super::{Command, TableView};
use crate::TableEvent;
use crate::grid_text::clipboard_grid;
use crate::selection::{CellPosition, CellRange, Movement};
use crate::transactions::{delete_rows, new_row};
use crate::values::{display_text, value_from_text};

impl TableView {
    /// Grows or shrinks the block of selected cells; the anchor stays where it was.
    pub(super) fn extend_selection(&mut self, movement: Movement, cx: &mut Context<Self>) {
        let head = self.selection.moved(movement, self.grid_size());
        self.extend_to(head, cx);
    }

    pub(super) fn extend_to(&mut self, head: CellPosition, cx: &mut Context<Self>) {
        let anchor = self.anchor;
        self.select(head, cx);
        self.anchor = anchor;
    }

    /// A click on the table's empty space: saves any edit and lets go of the cells, so the
    /// outline goes and keys no longer act on a cell nobody can see is selected.
    pub(super) fn deselect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_edit.is_some() && !self.commit_text_edit(window, cx) {
            return;
        }
        self.anchor = self.selection;
        self.is_selection_shown = false;
        // Without this the press would hand focus straight back to the table.
        window.prevent_default();
        window.blur();
        cx.notify();
    }

    /// Tab arrives here while a cell is being edited; the edit is saved before moving on.
    pub(super) fn move_selection(
        &mut self,
        movement: Movement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text_edit.is_some() && !self.commit_text_edit(window, cx) {
            return;
        }
        self.select(self.selection.moved(movement, self.grid_size()), cx);
    }

    pub(super) fn select(&mut self, position: CellPosition, cx: &mut Context<Self>) {
        self.selection = position;
        self.anchor = position;
        self.is_selection_shown = true;
        self.scroll
            .scroll_to_item(position.row, ScrollStrategy::Top);
        if let Some(column) = position.column.checked_sub(1) {
            self.horizontal_offset =
                self.columns
                    .offset_revealing(column, self.horizontal_offset, self.scrolling_width);
        }
        cx.notify();
    }

    /// Copies the selected cells as tab- and newline-separated text, as spreadsheets do. Rows
    /// not loaded yet are read in the background first, so a large block copies a moment later.
    pub(super) fn copy_cells(&mut self, cx: &mut Context<Self>) {
        let range = self.selected_range();
        let rows = *range.rows().start()..*range.rows().end() + 1;
        let missing = self.model.missing_rows(rows);
        if missing.is_empty() {
            let text = self.block_text(range, &HashMap::new());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return;
        }
        let storage = self.storage.clone();
        let background = cx.background_executor().clone();
        cx.spawn(async move |table, cx| {
            let fetched = background
                .spawn(async move { storage.read_window(&missing) })
                .await;
            // The table may have been closed meanwhile; then there is nothing to copy from.
            let _ = table.update(cx, |table, cx| match fetched {
                Ok(fetched) => {
                    let fetched: HashMap<usize, TableRow> = fetched.into_iter().collect();
                    let text = table.block_text(range, &fetched);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Err(error) => {
                    table.show_error(&error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The block's cells as text; `fetched` holds rows read for the copy that are not loaded.
    fn block_text(&self, range: CellRange, fetched: &HashMap<usize, TableRow>) -> String {
        let lines: Vec<String> = range
            .rows()
            .map(|row| {
                let table_row = self.model.row(row).or_else(|| fetched.get(&row));
                range
                    .columns()
                    .map(|column| self.cell_text(table_row, column))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect();
        lines.join("\n")
    }

    fn cell_text(&self, row: Option<&TableRow>, column: usize) -> String {
        let Some(row) = row else {
            return String::new();
        };
        let Some(property) = self.property_at(column) else {
            return row.page.title.clone();
        };
        value_for(&row.values, property)
            .map(|value| display_text(value, property))
            .unwrap_or_default()
    }

    /// Pastes a block of tab- and newline-separated text starting at the selected cell, as one
    /// transaction. Cells whose text does not fit their property are skipped and reported.
    pub(super) fn paste_cells(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let grid = self.grid_size();
        let start = self.selected_range().top_left();
        let mut operations = Vec::new();
        let mut skipped = Vec::new();
        for (row_offset, line) in clipboard_grid(&text).into_iter().enumerate() {
            for (column_offset, cell) in line.into_iter().enumerate() {
                let position = CellPosition {
                    row: start.row + row_offset,
                    column: start.column + column_offset,
                };
                if position.row >= grid.rows || position.column >= grid.columns {
                    continue;
                }
                let Some(page) = self.model.row_id(position.row) else {
                    continue;
                };
                match self.property_at(position.column) {
                    None => operations.push(Operation::set_title(page, &cell)),
                    Some(property) => match value_from_text(&cell, property) {
                        Ok(value) => {
                            operations.push(Operation::set_value(page, property.id, value))
                        }
                        Err(error) => skipped.push(error.to_string()),
                    },
                }
            }
        }
        if operations.is_empty() {
            self.error = skipped.first().map(|message| message.clone().into());
            cx.notify();
            return;
        }
        let transaction = Transaction::new(operations);
        self.write(transaction, cx);
        if let Some(first) = skipped.first() {
            self.error = Some(format!("{} cells were not pasted: {first}", skipped.len()).into());
        }
        cx.notify();
    }

    /// Clears every selected cell, as one change.
    pub(super) fn clear_cells(&mut self, cx: &mut Context<Self>) {
        let range = self.selected_range();
        let mut operations = Vec::new();
        for row in range.rows() {
            let Some(page) = self.model.row_id(row) else {
                continue;
            };
            for column in range.columns() {
                operations.push(match self.property_at(column) {
                    None => Operation::set_title(page, ""),
                    Some(property) => Operation::set_value(page, property.id, None),
                });
            }
        }
        let transaction = Transaction::new(operations);
        self.write(transaction, cx);
        cx.notify();
    }

    pub(super) fn toggle_row(&mut self, page: PageId, cx: &mut Context<Self>) {
        if !self.selected_rows.remove(&page) {
            self.selected_rows.insert(page);
        }
        cx.notify();
    }

    /// Deletes the checked rows, or the selected cell's row when none are checked.
    pub(super) fn delete_rows(&mut self, cx: &mut Context<Self>) {
        let pages: Vec<_> = if self.selected_rows.is_empty() {
            self.model.row_id(self.selection.row).into_iter().collect()
        } else {
            self.selected_rows.iter().copied().collect()
        };
        if pages.is_empty() {
            return;
        }
        self.write(delete_rows(&pages), cx);
        self.selected_rows.clear();
    }

    /// Adds an empty row at the bottom, scrolls to it and starts editing its title, as the
    /// table's New buttons do. Does nothing while read-only.
    pub fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.perform(Command::AddRow, window, cx);
    }

    pub(super) fn add_row_and_edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (transaction, page) = match new_row(self.model.database(), self.model.last_position()) {
            Ok(row) => row,
            Err(error) => {
                self.show_error(&error);
                cx.notify();
                return;
            }
        };
        self.write(transaction, cx);
        self.new_row = Some(page.id);
        let row = self.model.row_count().saturating_sub(1);
        self.select(CellPosition { row, column: 0 }, cx);
        self.start_editing(None, window, cx);
    }

    pub(super) fn open_row(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.model.row_id(self.selection.row) {
            cx.emit(TableEvent::OpenPage(page));
        }
    }

    /// Escape: cancels an edit, otherwise clears the row checkboxes.
    pub(super) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_edit.is_some() {
            self.cancel_text_edit(window, cx);
            return;
        }
        self.selected_rows.clear();
        cx.notify();
    }

    /// Typing a character on a selected cell starts editing it with that character.
    pub(super) fn type_to_edit(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if !self.focus_handle.is_focused(window)
            || self.text_edit.is_some()
            || modifiers.platform
            || modifiers.control
            || modifiers.function
        {
            return;
        }
        let Some(typed) = event.keystroke.key_char.clone() else {
            return;
        };
        if typed.chars().any(char::is_control) {
            return;
        }
        self.perform(Command::TypeToEdit(typed), window, cx);
    }

    /// Trackpad and shift-wheel scrolling move the property columns; the list scrolls rows itself.
    pub(super) fn scroll_horizontally(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height());
        let horizontal = if event.modifiers.shift {
            delta.y
        } else {
            delta.x
        };
        if horizontal.abs() <= delta.y.abs() && !event.modifiers.shift {
            return;
        }
        let offset = self.horizontal_offset - horizontal;
        self.horizontal_offset =
            offset.clamp(gpui::px(0.0), self.columns.max_offset(self.scrolling_width));
        cx.notify();
    }
}
