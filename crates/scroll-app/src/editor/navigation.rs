//! Caret movement and selection by keyboard and mouse. Text selections stay within one
//! block; plain caret movement crosses block boundaries and skips dividers.

use gpui::{Context, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Window};
use scroll_ui::TextSelection;
use scroll_ui::actions::{
    MoveDown, MoveLeft, MoveRight, MoveToEnd, MoveToLineEnd, MoveToLineStart, MoveToStart, MoveUp,
    MoveWordLeft, MoveWordRight, SelectAll, SelectDown, SelectLeft, SelectRight, SelectToEnd,
    SelectToLineEnd, SelectToLineStart, SelectToStart, SelectUp, SelectWordLeft, SelectWordRight,
};
use scroll_ui::text::{
    next_grapheme_boundary, next_word_end, previous_grapheme_boundary, previous_word_start,
    word_range_at,
};

use super::{BlockState, PageEditor, PageEditorEvent, Selection};
use crate::editor::document::{BlockId, BlockKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Backward,
    Forward,
}

/// Whether a block-selection step moves the whole selection or extends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectionStep {
    Move,
    Extend,
}

/// A caret position reached by moving up or down, with the column to keep aiming for.
struct VerticalTarget {
    block: BlockId,
    offset: usize,
    goal_x: Pixels,
}

impl PageEditor {
    fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = selection;
        self.block_selection = self.diagram_reached_without_editing(selection.block);
        self.pending_marks = None;
        self.close_completion_if_left();
        self.restart_caret_blink(cx);
        cx.emit(PageEditorEvent::CaretMoved);
        cx.notify();
    }

    pub(super) fn move_caret(&mut self, block: BlockId, offset: usize, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.set_selection(Selection::caret(block, offset), cx);
    }

    /// Moves the selection's head within its block, keeping the anchor.
    fn extend_selection(&mut self, head: usize, cx: &mut Context<Self>) {
        self.goal_x = None;
        let range = self.selection.range.extended_to(head);
        self.set_selection(
            Selection {
                block: self.selection.block,
                range,
            },
            cx,
        );
    }

    /// The nearest block with text before or after `block` in document order.
    fn neighbour(&self, block: BlockId, direction: Direction) -> Option<&BlockState> {
        let index = self.block_index(block)?;
        let has_text = |state: &&BlockState| state.info.kind != BlockKind::Divider;
        match direction {
            Direction::Backward => self.blocks[..index].iter().rev().find(has_text),
            Direction::Forward => self.blocks[index + 1..].iter().find(has_text),
        }
    }

    /// One grapheme left or right, stepping into the neighbouring block at either end.
    fn step(&self, direction: Direction) -> (BlockId, usize) {
        let Selection { block, range } = self.selection;
        let text = self.selected_text();
        let caret = range.head;
        match direction {
            Direction::Backward if caret > 0 => (block, previous_grapheme_boundary(text, caret)),
            Direction::Forward if caret < text.len() => {
                (block, next_grapheme_boundary(text, caret))
            }
            _ => match self.neighbour(block, direction) {
                Some(neighbour) if direction == Direction::Backward => {
                    (neighbour.info.id, neighbour.content.text.len())
                }
                Some(neighbour) => (neighbour.info.id, 0),
                None => (block, caret),
            },
        }
    }

    pub(super) fn visual_line_start(&self, block: BlockId, offset: usize) -> usize {
        self.block_state(block)
            .and_then(|state| state.layout.as_ref())
            .map_or(0, |layout| {
                layout.offset_at_x(layout.line_of(offset), Pixels::MIN)
            })
    }

    fn visual_line_end(&self, block: BlockId, offset: usize) -> usize {
        let text_length = self.content(block).map_or(0, |content| content.text.len());
        self.block_state(block)
            .and_then(|state| state.layout.as_ref())
            .map_or(text_length, |layout| {
                layout.offset_at_x(layout.line_of(offset), Pixels::MAX)
            })
    }

    fn vertical_target(&self, direction: Direction) -> Option<VerticalTarget> {
        let Selection { block, range } = self.selection;
        let layout = self.block_state(block)?.layout.as_ref()?;
        let goal_x = self.goal_x.unwrap_or_else(|| layout.x_of(range.head));
        let line = layout.line_of(range.head);
        let target_line = match direction {
            Direction::Backward => line.checked_sub(1),
            Direction::Forward => Some(line + 1).filter(|&next| next < layout.line_count()),
        };
        if let Some(target_line) = target_line {
            let offset = layout.offset_at_x(target_line, goal_x);
            return Some(VerticalTarget {
                block,
                offset,
                goal_x,
            });
        }
        let Some(neighbour) = self.neighbour(block, direction) else {
            let offset = match direction {
                Direction::Backward => 0,
                Direction::Forward => self.selected_text().len(),
            };
            return Some(VerticalTarget {
                block,
                offset,
                goal_x,
            });
        };
        // A neighbour not laid out in the last frame (scrolled out of view, or a diagram showing
        // its drawing instead of its source) may have an old layout of other text; land at its
        // nearest end.
        let current_layout = neighbour
            .layout
            .as_ref()
            .filter(|_| neighbour.layout_frame == self.frame);
        let Some(neighbour_layout) = current_layout else {
            let offset = match direction {
                Direction::Backward => neighbour.content.text.len(),
                Direction::Forward => 0,
            };
            return Some(VerticalTarget {
                block: neighbour.info.id,
                offset,
                goal_x,
            });
        };
        let neighbour_line = match direction {
            Direction::Backward => neighbour_layout.line_count() - 1,
            Direction::Forward => 0,
        };
        Some(VerticalTarget {
            block: neighbour.info.id,
            offset: neighbour_layout.offset_at_x(neighbour_line, goal_x),
            goal_x,
        })
    }

    fn move_vertically(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(target) = self.vertical_target(direction) else {
            return;
        };
        self.set_selection(Selection::caret(target.block, target.offset), cx);
        self.goal_x = Some(target.goal_x);
    }

    /// Extends the selection up or down within the current block. Once the head is at the
    /// block's start (going up) or end (going down), the next step selects this block and its
    /// neighbour as whole blocks, as in Notion.
    fn select_vertically(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.is_head_at_block_edge(direction) && self.has_adjacent_block(direction) {
            let block = self.selection.block;
            self.select_block_range(block, block, cx);
            return self.step_block_selection(direction, SelectionStep::Extend, cx);
        }
        let Some(target) = self.vertical_target(direction) else {
            return;
        };
        let head = if target.block == self.selection.block {
            target.offset
        } else if direction == Direction::Backward {
            0
        } else {
            self.selected_text().len()
        };
        let range = self.selection.range.extended_to(head);
        self.set_selection(
            Selection {
                block: self.selection.block,
                range,
            },
            cx,
        );
        self.goal_x = Some(target.goal_x);
    }

    fn is_head_at_block_edge(&self, direction: Direction) -> bool {
        let head = self.selection.range.head;
        match direction {
            Direction::Backward => head == 0,
            Direction::Forward => head == self.selected_text().len(),
        }
    }

    /// Whether any block, a divider included, lies next to the selection's block.
    fn has_adjacent_block(&self, direction: Direction) -> bool {
        let Some(index) = self.block_index(self.selection.block) else {
            return false;
        };
        match direction {
            Direction::Backward => index > 0,
            Direction::Forward => index + 1 < self.blocks.len(),
        }
    }

    pub(super) fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection.range.range();
        if !range.is_empty() {
            return self.move_caret(self.selection.block, range.start, cx);
        }
        let (block, offset) = self.step(Direction::Backward);
        self.move_caret(block, offset, cx);
    }

    pub(super) fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection.range.range();
        if !range.is_empty() {
            return self.move_caret(self.selection.block, range.end, cx);
        }
        let (block, offset) = self.step(Direction::Forward);
        self.move_caret(block, offset, cx);
    }

    pub(super) fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.move_completion_highlight(-1, cx) {
            return;
        }
        if self.block_selection.is_some() {
            return self.step_block_selection(Direction::Backward, SelectionStep::Move, cx);
        }
        self.move_vertically(Direction::Backward, cx);
    }

    pub(super) fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.move_completion_highlight(1, cx) {
            return;
        }
        if self.block_selection.is_some() {
            return self.step_block_selection(Direction::Forward, SelectionStep::Move, cx);
        }
        self.move_vertically(Direction::Forward, cx);
    }

    pub(super) fn move_word_left(
        &mut self,
        _: &MoveWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caret = self.selection.range.head;
        if caret == 0 {
            let (block, offset) = self.step(Direction::Backward);
            return self.move_caret(block, offset, cx);
        }
        let target = previous_word_start(self.selected_text(), caret);
        self.move_caret(self.selection.block, target, cx);
    }

    pub(super) fn move_word_right(
        &mut self,
        _: &MoveWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caret = self.selection.range.head;
        if caret == self.selected_text().len() {
            let (block, offset) = self.step(Direction::Forward);
            return self.move_caret(block, offset, cx);
        }
        let target = next_word_end(self.selected_text(), caret);
        self.move_caret(self.selection.block, target, cx);
    }

    pub(super) fn move_to_line_start(
        &mut self,
        _: &MoveToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let block = self.selection.block;
        self.move_caret(
            block,
            self.visual_line_start(block, self.selection.range.head),
            cx,
        );
    }

    pub(super) fn move_to_line_end(
        &mut self,
        _: &MoveToLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let block = self.selection.block;
        self.move_caret(
            block,
            self.visual_line_end(block, self.selection.range.head),
            cx,
        );
    }

    pub(super) fn move_to_start(
        &mut self,
        _: &MoveToStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(first) = self
            .blocks
            .iter()
            .find(|state| state.info.kind != BlockKind::Divider)
        {
            let block = first.info.id;
            self.move_caret(block, 0, cx);
        }
    }

    pub(super) fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self
            .blocks
            .iter()
            .rev()
            .find(|state| state.info.kind != BlockKind::Divider)
        {
            let (block, offset) = (last.info.id, last.content.text.len());
            self.move_caret(block, offset, cx);
        }
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = previous_grapheme_boundary(self.selected_text(), self.selection.range.head);
        self.extend_selection(target, cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_grapheme_boundary(self.selected_text(), self.selection.range.head);
        self.extend_selection(target, cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.block_selection.is_some() {
            return self.step_block_selection(Direction::Backward, SelectionStep::Extend, cx);
        }
        self.select_vertically(Direction::Backward, cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.block_selection.is_some() {
            return self.step_block_selection(Direction::Forward, SelectionStep::Extend, cx);
        }
        self.select_vertically(Direction::Forward, cx);
    }

    pub(super) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = previous_word_start(self.selected_text(), self.selection.range.head);
        self.extend_selection(target, cx);
    }

    pub(super) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = next_word_end(self.selected_text(), self.selection.range.head);
        self.extend_selection(target, cx);
    }

    pub(super) fn select_to_line_start(
        &mut self,
        _: &SelectToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.visual_line_start(self.selection.block, self.selection.range.head);
        self.extend_selection(target, cx);
    }

    pub(super) fn select_to_line_end(
        &mut self,
        _: &SelectToLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.visual_line_end(self.selection.block, self.selection.range.head);
        self.extend_selection(target, cx);
    }

    pub(super) fn select_to_start(
        &mut self,
        _: &SelectToStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_selection(0, cx);
    }

    pub(super) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_selection(self.selected_text().len(), cx);
    }

    /// Selects the whole current block; text selection across blocks is not supported, so a
    /// second Cmd+A selects every block instead.
    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let length = self.selected_text().len();
        let is_whole_block = self.selection.range.range() == (0..length);
        if is_whole_block || self.block_selection.is_some() {
            if let (Some(first), Some(last)) = (self.blocks.first(), self.blocks.last()) {
                self.block_selection = Some(super::BlockSelection {
                    anchor: first.info.id,
                    head: last.info.id,
                });
                cx.notify();
            }
            return;
        }
        let range = TextSelection::from_range(0..length, false);
        self.set_selection(
            Selection {
                block: self.selection.block,
                range,
            },
            cx,
        );
    }

    pub(super) fn mouse_down(
        &mut self,
        block: BlockId,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus_handle);
        let Some(state) = self.block_state(block) else {
            return;
        };
        let Some(offset) = state
            .layout
            .as_ref()
            .map(|layout| layout.offset_for_point(event.position))
        else {
            return;
        };
        if event.modifiers.platform
            && let Some(url) = state.content.link_at(offset)
        {
            cx.open_url(url);
            return;
        }
        let text = state.content.text.clone();
        self.goal_x = None;
        match event.click_count {
            2 => {
                let range = TextSelection::from_range(word_range_at(&text, offset), false);
                self.set_selection(Selection { block, range }, cx);
            }
            3.. => {
                let range = TextSelection::from_range(0..text.len(), false);
                self.set_selection(Selection { block, range }, cx);
            }
            _ if event.modifiers.shift && block == self.selection.block => {
                self.extend_selection(offset, cx)
            }
            _ if event.modifiers.shift => {
                let anchor = self
                    .block_selection
                    .map_or(self.selection.block, |selection| selection.anchor);
                self.select_block_range(anchor, block, cx);
            }
            _ => {
                self.is_dragging = true;
                self.move_caret(block, offset, cx);
            }
        }
    }

    /// A click in the empty space below the last block puts the caret at its end.
    pub(super) fn mouse_down_below_blocks(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        if let Some(last) = self
            .blocks
            .iter()
            .rev()
            .find(|state| state.info.kind != BlockKind::Divider)
        {
            let (block, offset) = (last.info.id, last.content.text.len());
            self.move_caret(block, offset, cx);
        }
    }

    /// Any pointer movement gives the drag handle back to the pointer. While a press that began
    /// in a block's text is held, it selects text within that block, or whole blocks from it to
    /// the block under the pointer once the pointer leaves it.
    pub(super) fn mouse_drag(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::take(&mut self.is_handle_at_caret) {
            cx.notify();
        }
        if !self.is_dragging {
            return;
        }
        let origin = self.selection.block;
        let over = self.block_at_height(event.position.y).unwrap_or(origin);
        if over != origin {
            return self.select_block_range(origin, over, cx);
        }
        let Some(layout) = self
            .block_state(origin)
            .and_then(|state| state.layout.as_ref())
        else {
            return;
        };
        let head = layout.offset_for_point(event.position);
        if self.block_selection.is_some() || head != self.selection.range.head {
            self.extend_selection(head, cx);
        }
    }

    /// The block whose row is at window height `y`, judged by the blocks laid out in the last
    /// frame (only those on screen are): the last one starting above `y`, so the gap between two
    /// blocks, and a divider, count as the block above.
    fn block_at_height(&self, y: Pixels) -> Option<BlockId> {
        self.blocks
            .iter()
            .filter(|state| state.layout_frame == self.frame)
            .filter_map(|state| Some((state.info.id, state.layout.as_ref()?.bounds().top())))
            .take_while(|(_, top)| *top <= y)
            .last()
            .map(|(block, _)| block)
    }

    pub(super) fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_dragging = false;
        self.clear_drop_target(cx);
    }
}
