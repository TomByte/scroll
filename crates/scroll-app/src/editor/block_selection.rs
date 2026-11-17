//! Selecting whole blocks: Escape selects the current block, Shift+Up/Down extend (from a text
//! selection too, once it reaches its block's start or end), dragging across blocks or
//! shift-clicking another block selects the range, and the selection can be deleted, copied,
//! cut, indented or moved (with the keyboard or by a selected block's handle) as a unit.

use std::ops::Range;

use gpui::{Context, Window};

use super::navigation::{Direction, SelectionStep};
use super::structure::{has_text, next_sibling, outermost_blocks, previous_sibling};
use super::{
    BlockSelection, MoveBlocksDown, MoveBlocksUp, PageEditor, PageEditorEvent, SelectBlocks,
    Selection,
};
use crate::editor::document::{BlockId, Placement};

impl PageEditor {
    /// Escape: closes the completion list if open, otherwise selects the block holding the
    /// caret.
    pub(super) fn select_blocks(
        &mut self,
        _: &SelectBlocks,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_completion(cx) {
            return;
        }
        let block = self.whole_block(self.selection.block);
        self.diagram_being_edited = None;
        self.block_selection = Some(BlockSelection {
            anchor: block,
            head: block,
        });
        cx.notify();
    }

    /// Selects the blocks from `anchor` to `head`, as dragging across them or shift-clicking
    /// does. A table is selected whole, whichever of its parts is named.
    pub(super) fn select_block_range(
        &mut self,
        anchor: BlockId,
        head: BlockId,
        cx: &mut Context<Self>,
    ) {
        let selection = Some(BlockSelection {
            anchor: self.whole_block(anchor),
            head: self.whole_block(head),
        });
        self.diagram_being_edited = None;
        if self.block_selection != selection {
            self.block_selection = selection;
            self.completion = None;
            cx.notify();
        }
    }

    /// Indices (in document order) of the selected blocks. A selected table's rows and cells
    /// are among them.
    pub(super) fn selected_block_range(&self) -> Option<Range<usize>> {
        let selection = self.block_selection?;
        let anchor = self.block_index(selection.anchor)?;
        let head = self.block_index(selection.head)?;
        Some(anchor.min(head)..self.row_span(anchor.max(head)).end)
    }

    pub(super) fn is_block_selected(&self, index: usize) -> bool {
        self.selected_block_range()
            .is_some_and(|range| range.contains(&index))
    }

    /// The blocks a structural command acts on: the selected blocks without their nested
    /// descendants, or the block holding the caret.
    pub(super) fn target_blocks(&self) -> Vec<BlockId> {
        match self.selected_block_range() {
            Some(range) => outermost_blocks(&self.block_infos(), range),
            None => vec![self.selection.block],
        }
    }

    /// Extends the block selection one block up or down by moving its head (so it shrinks when
    /// the head is heading back to the anchor), or moves it to the single block just past its
    /// top or bottom. Steps are in document order whatever the blocks' nesting.
    pub(super) fn step_block_selection(
        &mut self,
        direction: Direction,
        step: SelectionStep,
        cx: &mut Context<Self>,
    ) {
        let (Some(selection), Some(range)) = (self.block_selection, self.selected_block_range())
        else {
            return;
        };
        let Some(head) = self.block_index(selection.head) else {
            return;
        };
        let last = self.blocks.len() - 1;
        let target = match (direction, step) {
            (Direction::Forward, SelectionStep::Extend) => self.row_span(head).end,
            (Direction::Forward, SelectionStep::Move) => range.end,
            (Direction::Backward, SelectionStep::Extend) => head.saturating_sub(1),
            (Direction::Backward, SelectionStep::Move) => range.start.saturating_sub(1),
        };
        // A step that lands in a table lands on the table.
        let target = self.row_span(target.min(last)).start;
        let target = self.blocks[target].info.id;
        self.block_selection = Some(BlockSelection {
            anchor: match step {
                SelectionStep::Extend => selection.anchor,
                SelectionStep::Move => target,
            },
            head: target,
        });
        self.reveal_attempts = 0;
        if let Some(index) = self.block_index(target) {
            self.scroll.reveal_row(self.list_row(index));
        }
        cx.notify();
    }

    /// Leaves block selection with the caret at the end of the head block, or of a table's
    /// first cell.
    pub(super) fn edit_selected_block(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.block_selection.take() else {
            return;
        };
        let block = self
            .block_index(selection.head)
            .and_then(|index| self.first_cell_of(index))
            .unwrap_or(selection.head);
        let length = self.content(block).map_or(0, |content| content.text.len());
        self.selection = Selection::caret(block, length);
        self.restart_caret_blink(cx);
        cx.emit(PageEditorEvent::CaretMoved);
        cx.notify();
    }

    /// Deletes the selected blocks with everything nested under them; the caret goes to the
    /// end of the block before them.
    pub(super) fn delete_selected_blocks(&mut self, cx: &mut Context<Self>) {
        let Some(range) = self.selected_block_range() else {
            return;
        };
        let doomed = self.target_blocks();
        let previous = range
            .start
            .checked_sub(1)
            .map(|index| self.blocks[index].info)
            .filter(|info| has_text(info.kind));
        let next = self.blocks.get(range.end).map(|block| block.info.id);
        let previous_length = previous
            .and_then(|info| self.content(info.id))
            .map_or(0, |content| content.text.len());
        self.apply_edit(cx, |page| {
            for block in &doomed {
                page.delete_block(*block)?;
            }
            let caret = match (previous, next) {
                (Some(previous), _) => Selection::caret(previous.id, previous_length),
                (None, Some(next)) => Selection::caret(next, 0),
                (None, None) => Selection::caret(page.ensure_block()?, 0),
            };
            Ok(caret)
        });
    }

    pub(super) fn move_blocks_up(
        &mut self,
        _: &MoveBlocksUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_blocks(Direction::Backward, cx);
    }

    pub(super) fn move_blocks_down(
        &mut self,
        _: &MoveBlocksDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_blocks(Direction::Forward, cx);
    }

    /// Swaps the target blocks with the sibling just above or below them. With the caret in a
    /// table, that is the caret's row and the row above or below it.
    fn move_blocks(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let is_down = direction == Direction::Forward;
        let table_row = self
            .caret_table_row()
            .filter(|_| self.block_selection.is_none());
        let blocks = match table_row {
            Some(row) => vec![row],
            None => self.target_blocks(),
        };
        let infos = self.block_infos();
        let (Some(&first), Some(&last)) = (blocks.first(), blocks.last()) else {
            return;
        };
        let neighbour = if is_down {
            next_sibling(&infos, last)
        } else {
            previous_sibling(&infos, first)
        };
        let Some(neighbour) = neighbour else {
            return;
        };
        let selection = self.selection;
        let block_selection = self.block_selection;
        self.apply_edit(cx, |page| {
            let placement = if is_down {
                Placement::Before(first)
            } else {
                Placement::After(last)
            };
            page.move_block(neighbour, placement)?;
            Ok(selection)
        });
        self.block_selection = block_selection;
    }
}
