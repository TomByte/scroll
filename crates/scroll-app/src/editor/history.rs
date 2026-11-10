//! Undo and redo, putting the selection back where the change was.

use super::{PageEditor, PageEditorEvent, Selection};
use crate::editor::document::{BlockId, PageResult, StableCursor};
use gpui::{Context, Window};
use scroll_ui::TextSelection;
use scroll_ui::actions::{Redo, Undo};

impl PageEditor {
    pub(super) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only() {
            return;
        }
        let restored = self.page.undo();
        self.finish_history_step(restored, cx);
    }

    pub(super) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only() {
            return;
        }
        let restored = self.page.redo();
        self.finish_history_step(restored, cx);
    }

    pub(super) fn finish_history_step(
        &mut self,
        restored: PageResult<Vec<StableCursor>>,
        cx: &mut Context<Self>,
    ) {
        self.refresh_blocks();
        match restored {
            Ok(cursors) => self.select_cursors(&cursors),
            Err(error) => log::error!("undo/redo failed: {error}"),
        }
        self.block_selection = None;
        self.clamp_selection();
        self.restart_caret_blink(cx);
        cx.emit(PageEditorEvent::Edited);
        cx.emit(PageEditorEvent::CaretMoved);
        cx.notify();
    }

    /// Selects from the first cursor to the last, if both still resolve to one block.
    pub(super) fn select_cursors(&mut self, cursors: &[StableCursor]) {
        let resolved: Vec<(BlockId, usize)> = cursors
            .iter()
            .filter_map(|cursor| self.page.resolve_cursor(cursor))
            .collect();
        let (Some(&(anchor_block, anchor)), Some(&(head_block, head))) =
            (resolved.first(), resolved.last())
        else {
            return;
        };
        self.selection = if anchor_block == head_block {
            Selection {
                block: head_block,
                range: TextSelection { anchor, head },
            }
        } else {
            Selection::caret(head_block, head)
        };
    }
}
