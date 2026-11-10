//! Edits: every change goes into the Loro page as one transaction, then the view refreshes
//! from Loro's change events.

use std::ops::Range;

use gpui::{Context, Window};
use scroll_ui::TextSelection;
use scroll_ui::actions::{
    Backspace, Delete, DeleteToLineStart, DeleteWordBackward, DeleteWordForward, Newline,
    ShowCharacterPalette,
};
use scroll_ui::text::{
    floor_char_boundary, next_grapheme_boundary, next_word_end, previous_grapheme_boundary,
    previous_word_start,
};

use super::inline_completion::CompletionKind;
use super::markdown::{block_shortcut, inline_shortcut};
use super::rich_text::{MarkKind, Marks};
use super::{Indent, InsertLineBreak, Outdent, PageEditor, PageEditorEvent, Selection, ToggleToDo};
use crate::editor::document::{
    BlockId, BlockKind, PageDocument, PageResult, Placement, StableCursor,
};

/// What Tab inserts inside a code block.
const CODE_INDENT: &str = "    ";

/// The selection pinned to Loro cursors so it can be found again after the text changes.
pub(super) struct AnchoredSelection {
    anchor: Option<StableCursor>,
    head: Option<StableCursor>,
    composition: Option<StableCursor>,
    block_index: usize,
}

impl PageEditor {
    /// Runs `edit` as one local transaction, refreshes the view and moves the selection to
    /// the one `edit` returns.
    pub(super) fn apply_edit(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut PageDocument) -> PageResult<Selection>,
    ) {
        if self.is_read_only() {
            return;
        }
        self.page
            .remember_selection_for_undo(&self.selection_cursors());
        let outcome = edit(&mut self.page);
        self.page.commit();
        self.refresh_blocks();
        match outcome {
            Ok(selection) => self.selection = selection,
            Err(error) => log::error!("edit failed: {error}"),
        }
        self.block_selection = None;
        self.clamp_selection();
        self.goal_x = None;
        self.restart_caret_blink(cx);
        cx.emit(PageEditorEvent::Edited);
        cx.emit(PageEditorEvent::CaretMoved);
        cx.notify();
    }

    fn selection_cursors(&self) -> Vec<StableCursor> {
        let block = self.selection.block;
        [self.selection.range.anchor, self.selection.range.head]
            .into_iter()
            .filter_map(|offset| self.page.cursor_at(block, offset))
            .collect()
    }

    pub(super) fn anchor_selection(&self) -> AnchoredSelection {
        let block = self.selection.block;
        let cursor = |offset| self.page.cursor_at(block, offset);
        AnchoredSelection {
            anchor: cursor(self.selection.range.anchor),
            head: cursor(self.selection.range.head),
            composition: self
                .composition
                .as_ref()
                .and_then(|composition| self.page.cursor_at(composition.block, composition.offset)),
            block_index: self.block_index(block).unwrap_or(0),
        }
    }

    pub(super) fn restore_selection(&mut self, anchored: AnchoredSelection) {
        let resolve = |cursor: &Option<StableCursor>| {
            cursor
                .as_ref()
                .and_then(|cursor| self.page.resolve_cursor(cursor))
        };
        let anchor = resolve(&anchored.anchor);
        let head = resolve(&anchored.head);
        self.selection = match (anchor, head) {
            (Some((anchor_block, anchor)), Some((head_block, head)))
                if anchor_block == head_block =>
            {
                Selection {
                    block: head_block,
                    range: TextSelection { anchor, head },
                }
            }
            (_, Some((block, head))) => Selection::caret(block, head),
            _ => self.fallback_selection(anchored.block_index),
        };
        let composition_position = resolve(&anchored.composition);
        if let Some(composition) = self.composition.as_mut() {
            match composition_position {
                Some((block, offset)) => {
                    composition.block = block;
                    composition.offset = offset;
                }
                None => self.composition = None,
            }
        }
        self.clamp_selection();
    }

    /// Where the caret goes when its block was deleted by someone else: the end of the
    /// block that now sits closest to where it was.
    fn fallback_selection(&self, block_index: usize) -> Selection {
        let index = block_index.min(self.blocks.len().saturating_sub(1));
        match self.blocks.get(index) {
            Some(block) => Selection::caret(block.info.id, block.content.text.len()),
            None => self.selection,
        }
    }

    pub(super) fn clamp_selection(&mut self) {
        let Some(text) = self
            .content(self.selection.block)
            .map(|content| content.text.as_str())
        else {
            self.selection = self.fallback_selection(0);
            return;
        };
        let clamp = |offset: usize| floor_char_boundary(text, offset);
        self.selection.range = TextSelection {
            anchor: clamp(self.selection.range.anchor),
            head: clamp(self.selection.range.head),
        };
    }

    /// Replaces `range` in the selected block with `text` as typed (newlines stay inside the
    /// block), then applies any Markdown shortcut or slash command the typing completed.
    pub(super) fn type_text(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let block = self.selection.block;
        let pending_marks = self.pending_marks.take();
        let is_code = self.kind_of(block) == BlockKind::Code;
        self.apply_edit(cx, |page| {
            replace_range(page, block, range.clone(), text)?;
            let caret = range.start + text.len();
            if is_code {
                page.clear_marks(block, range.start..caret)?;
            } else if let Some(marks) = pending_marks.as_ref() {
                apply_marks(page, block, range.start..caret, marks)?;
            }
            Ok(Selection::caret(block, caret))
        });
        if !is_code && text.chars().count() == 1 {
            self.after_typing(text, cx);
        }
    }

    /// Reacts to one typed character: slash menu, block shortcuts, inline shortcuts.
    fn after_typing(&mut self, typed: &str, cx: &mut Context<Self>) {
        let block = self.selection.block;
        let caret = self.selection.range.head;
        let text_before_caret = self.selected_text()[..caret].to_string();
        if self.open_completion_for(typed, &text_before_caret, cx) {
            return;
        }
        // Code blocks hold literal text, so Markdown prefixes typed there stay as text.
        if self.kind_of(block) != BlockKind::Code
            && let Some(shortcut) = block_shortcut(&text_before_caret)
        {
            return self.convert_block(block, shortcut.kind, 0..shortcut.prefix_length, cx);
        }
        if let Some(shortcut) = inline_shortcut(&text_before_caret) {
            let content = shortcut.content_after_removal();
            let close = shortcut.range.end - shortcut.delimiter_length..shortcut.range.end;
            let open = shortcut.range.start..shortcut.range.start + shortcut.delimiter_length;
            self.apply_edit(cx, |page| {
                page.delete_text(block, close)?;
                page.delete_text(block, open)?;
                page.add_mark(block, content.clone(), shortcut.kind)?;
                Ok(Selection::caret(block, content.end))
            });
            // Typing on after the closing delimiter should not continue the mark.
            let mut marks = self
                .content(block)
                .map(|content| content.marks_at(self.selection.range.head))
                .unwrap_or_default();
            marks.set(shortcut.kind, false);
            self.pending_marks = Some(marks);
        }
    }

    /// Turns `block` into `kind`, removing `markup` (the shortcut or slash command typed).
    /// A divider has no text, so the caret moves to a new paragraph after it.
    pub(super) fn convert_block(
        &mut self,
        block: BlockId,
        kind: BlockKind,
        markup: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.completion = None;
        self.apply_edit(cx, |page| {
            page.delete_text(block, markup.clone())?;
            page.set_kind(block, kind)?;
            if kind == BlockKind::Code {
                let length = page.block_content(block)?.text.len();
                page.clear_marks(block, 0..length)?;
            }
            if kind == BlockKind::Divider {
                let length = page.block_content(block)?.text.len();
                page.delete_text(block, 0..length)?;
                let next = page.insert_block(Placement::After(block), BlockKind::Paragraph)?;
                return Ok(Selection::caret(next, 0));
            }
            Ok(Selection::caret(block, markup.start))
        });
    }

    pub(super) fn delete_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let block = self.selection.block;
        self.apply_edit(cx, |page| {
            page.delete_text(block, range.clone())?;
            Ok(Selection::caret(block, range.start))
        });
    }

    /// Deletes back to `target`, or applies the rules for Backspace at a block's start.
    fn delete_backward_to(
        &mut self,
        target: impl FnOnce(&str, usize) -> usize,
        cx: &mut Context<Self>,
    ) {
        let range = self.selection.range.range();
        if !range.is_empty() {
            return self.delete_range(range, cx);
        }
        let caret = self.selection.range.head;
        if caret == 0 {
            return self.backspace_at_block_start(cx);
        }
        let start = target(self.selected_text(), caret);
        self.delete_range(start..caret, cx);
    }

    /// A styled block first becomes a paragraph, a nested paragraph moves out a level, and
    /// only then does the block join the one before it.
    fn backspace_at_block_start(&mut self, cx: &mut Context<Self>) {
        let block = self.selection.block;
        let Some(state) = self.block_state(block) else {
            return;
        };
        let info = state.info;
        if info.kind != BlockKind::Paragraph {
            return self.apply_edit(cx, |page| {
                page.set_kind(block, BlockKind::Paragraph)?;
                Ok(Selection::caret(block, 0))
            });
        }
        if let Some(parent) = info.parent {
            return self.apply_edit(cx, |page| {
                page.move_block(block, Placement::After(parent))?;
                Ok(Selection::caret(block, 0))
            });
        }
        self.merge_into_previous_block(cx);
    }

    fn merge_into_previous_block(&mut self, cx: &mut Context<Self>) {
        let block = self.selection.block;
        let Some(previous) = self
            .block_index(block)
            .and_then(|index| index.checked_sub(1))
            .map(|index| self.blocks[index].info)
        else {
            return;
        };
        if previous.kind == BlockKind::Divider {
            return self.apply_edit(cx, |page| {
                page.delete_block(previous.id)?;
                Ok(Selection::caret(block, 0))
            });
        }
        self.apply_edit(cx, |page| {
            let merged = page.merge_into_previous(block)?;
            Ok(merged.map_or(Selection::caret(block, 0), |(into, join)| {
                Selection::caret(into, join)
            }))
        });
    }

    /// Deletes forward to `target`, or pulls the next block up from the very end.
    fn delete_forward_to(
        &mut self,
        target: impl FnOnce(&str, usize) -> usize,
        cx: &mut Context<Self>,
    ) {
        let range = self.selection.range.range();
        if !range.is_empty() {
            return self.delete_range(range, cx);
        }
        let caret = self.selection.range.head;
        if caret == self.selected_text().len() {
            return self.merge_next_block(cx);
        }
        let end = target(self.selected_text(), caret);
        self.delete_range(caret..end, cx);
    }

    fn merge_next_block(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self
            .block_index(self.selection.block)
            .and_then(|index| self.blocks.get(index + 1))
            .map(|state| state.info)
        else {
            return;
        };
        let selection = self.selection;
        self.apply_edit(cx, |page| {
            if next.kind == BlockKind::Divider {
                page.delete_block(next.id)?;
            } else {
                // The block above `next` in document order is the one holding the caret.
                page.merge_into_previous(next.id)?;
            }
            Ok(selection)
        });
    }

    pub(super) fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.block_selection.is_some() {
            return self.delete_selected_blocks(cx);
        }
        self.delete_backward_to(previous_grapheme_boundary, cx);
    }

    pub(super) fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_backward_to(previous_word_start, cx);
    }

    pub(super) fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line_start = self.visual_line_start(self.selection.block, self.selection.range.head);
        self.delete_backward_to(|_, _| line_start, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.block_selection.is_some() {
            return self.delete_selected_blocks(cx);
        }
        self.delete_forward_to(next_grapheme_boundary, cx);
    }

    pub(super) fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_forward_to(next_word_end, cx);
    }

    /// Enter: confirms the slash menu, edits a selected block, adds a line in code, turns an
    /// empty list item back into a paragraph, or splits the block.
    pub(super) fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.choose_highlighted_completion(cx) {
            return;
        }
        if self.block_selection.is_some() {
            return self.edit_selected_block(cx);
        }
        let block = self.selection.block;
        let kind = self.kind_of(block);
        if kind == BlockKind::Code {
            return self.type_text(self.selection.range.range(), "\n", cx);
        }
        let is_empty = self.selected_text().is_empty();
        if is_empty
            && matches!(
                kind,
                BlockKind::ToDo
                    | BlockKind::BulletedListItem
                    | BlockKind::NumberedListItem
                    | BlockKind::Quote
                    | BlockKind::Callout
            )
        {
            return self.apply_edit(cx, |page| {
                page.set_kind(block, BlockKind::Paragraph)?;
                Ok(Selection::caret(block, 0))
            });
        }
        let range = self.selection.range.range();
        let text_length = self.selected_text().len();
        self.pending_marks = None;
        if range.start == 0 && range.is_empty() && text_length > 0 {
            // At the very start, push the block down rather than splitting off all its text.
            let kind_above = kind_after_split(kind, true);
            return self.apply_edit(cx, |page| {
                page.insert_block(Placement::Before(block), kind_above)?;
                Ok(Selection::caret(block, 0))
            });
        }
        let new_kind = kind_after_split(kind, range.end == text_length);
        self.apply_edit(cx, |page| {
            page.delete_text(block, range.clone())?;
            let new_block = page.split_block(block, range.start, new_kind)?;
            Ok(Selection::caret(new_block, 0))
        });
    }

    pub(super) fn insert_line_break(
        &mut self,
        _: &InsertLineBreak,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.type_text(self.selection.range.range(), "\n", cx);
    }

    /// Tab: indents in code, otherwise nests the block (or selected blocks) under the
    /// sibling before it.
    pub(super) fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        if self.completion_kind() == Some(CompletionKind::Emoji)
            && self.choose_highlighted_completion(cx)
        {
            return;
        }
        let block = self.selection.block;
        if self.block_selection.is_none() && self.kind_of(block) == BlockKind::Code {
            return self.type_text(self.selection.range.range(), CODE_INDENT, cx);
        }
        let blocks = self.target_blocks();
        let infos = self.block_infos();
        let Some(&first) = blocks.first() else {
            return;
        };
        let Some(new_parent) = super::structure::previous_sibling(&infos, first) else {
            return;
        };
        let selection = self.selection;
        let block_selection = self.block_selection;
        self.apply_edit(cx, |page| {
            for block in &blocks {
                page.move_block(*block, Placement::LastChildOf(Some(new_parent)))?;
            }
            Ok(selection)
        });
        self.block_selection = block_selection;
    }

    /// Shift+Tab: moves the block (or selected blocks) out one level, after its parent.
    pub(super) fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        let blocks = self.target_blocks();
        let infos = self.block_infos();
        let Some(parent) = blocks
            .first()
            .and_then(|first| infos.iter().find(|info| info.id == *first))
            .and_then(|info| info.parent)
        else {
            return;
        };
        let selection = self.selection;
        let block_selection = self.block_selection;
        self.apply_edit(cx, |page| {
            let mut previous = parent;
            for block in &blocks {
                page.move_block(*block, Placement::After(previous))?;
                previous = *block;
            }
            Ok(selection)
        });
        self.block_selection = block_selection;
    }

    /// Turns the selected blocks, or the caret's block, into `action.kind`.
    pub(super) fn set_block_kind(
        &mut self,
        action: &super::SetBlockKind,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blocks: Vec<BlockId> = match self.selected_block_range() {
            Some(range) => self.blocks[range]
                .iter()
                .map(|block| block.info.id)
                .collect(),
            None => vec![self.selection.block],
        };
        if let [only] = blocks.as_slice() {
            return self.convert_block(*only, action.kind, 0..0, cx);
        }
        let kind = action.kind;
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            for block in &blocks {
                page.set_kind(*block, kind)?;
            }
            Ok(selection)
        });
    }

    pub(super) fn toggle_to_do(&mut self, _: &ToggleToDo, _: &mut Window, cx: &mut Context<Self>) {
        let block = self.selection.block;
        let Some(info) = self.block_state(block).map(|state| state.info) else {
            return;
        };
        if info.kind != BlockKind::ToDo {
            return;
        }
        self.set_to_do_checked(block, !info.is_checked, cx);
    }

    pub(super) fn set_to_do_checked(
        &mut self,
        block: BlockId,
        is_checked: bool,
        cx: &mut Context<Self>,
    ) {
        let selection = self.selection;
        self.apply_edit(cx, |page| {
            page.set_checked(block, is_checked)?;
            Ok(selection)
        });
    }

    pub(super) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }
}

/// The kind of the new block Enter creates: lists and to-dos continue; anything else gives
/// a plain paragraph when splitting at the end, and keeps its kind when splitting mid-text.
fn kind_after_split(kind: BlockKind, is_at_end: bool) -> BlockKind {
    match kind {
        BlockKind::BulletedListItem | BlockKind::NumberedListItem | BlockKind::ToDo => kind,
        _ if is_at_end => BlockKind::Paragraph,
        _ => kind,
    }
}

fn replace_range(
    page: &mut PageDocument,
    block: BlockId,
    range: Range<usize>,
    text: &str,
) -> PageResult<()> {
    page.delete_text(block, range.clone())?;
    if !text.is_empty() {
        page.insert_text(block, range.start, text)?;
    }
    Ok(())
}

/// Makes freshly typed text in `range` carry exactly the boolean marks in `marks`.
fn apply_marks(
    page: &mut PageDocument,
    block: BlockId,
    range: Range<usize>,
    marks: &Marks,
) -> PageResult<()> {
    if range.is_empty() {
        return Ok(());
    }
    let content = page.block_content(block)?;
    for kind in [
        MarkKind::Bold,
        MarkKind::Italic,
        MarkKind::Strikethrough,
        MarkKind::Code,
    ] {
        let is_wanted = marks.has(kind);
        if is_wanted == content.is_marked_throughout(&range, kind) {
            continue;
        }
        if is_wanted {
            page.add_mark(block, range.clone(), kind)?;
        } else {
            page.remove_mark(block, range.clone(), kind)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_continue_and_headings_end_at_the_end_of_the_line() {
        assert_eq!(kind_after_split(BlockKind::ToDo, true), BlockKind::ToDo);
        assert_eq!(
            kind_after_split(BlockKind::NumberedListItem, false),
            BlockKind::NumberedListItem
        );
        assert_eq!(
            kind_after_split(BlockKind::Heading1, true),
            BlockKind::Paragraph
        );
        assert_eq!(
            kind_after_split(BlockKind::Heading1, false),
            BlockKind::Heading1
        );
        assert_eq!(
            kind_after_split(BlockKind::Quote, true),
            BlockKind::Paragraph
        );
    }
}
