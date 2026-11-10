//! Copy, cut and paste. Copied blocks keep their kinds, nesting and marks; see `format` for what
//! goes on the clipboard.

pub mod format;

use super::rich_text::MarkKind;
use super::{PageEditor, Selection};
use crate::editor::document::{BlockId, BlockKind, PageDocument, PageResult, Placement};
use gpui::{ClipboardItem, Context, Window};
use scroll_ui::actions::{Copy, Cut, Paste};
use std::ops::Range;

impl PageEditor {
    /// Copies blocks (with kinds, nesting and marks for pasting here) or the selected text,
    /// with Markdown as the text other apps see.
    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let Some(copied) = self.copied_selection() else {
            return;
        };
        let markdown = format::to_markdown(&copied);
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            markdown, copied,
        ));
    }

    pub(super) fn copied_selection(&self) -> Option<format::Copied> {
        if let Some(range) = self.selected_block_range() {
            let blocks = &self.blocks[range];
            let base_depth = blocks
                .iter()
                .map(|block| block.info.depth)
                .min()
                .unwrap_or(0);
            return Some(format::Copied {
                blocks: blocks
                    .iter()
                    .map(|block| {
                        format::CopiedBlock::new(
                            block.info.kind,
                            block.info.depth - base_depth,
                            block.info.is_checked,
                            &block.content,
                        )
                    })
                    .collect(),
                is_fragment: false,
            });
        }
        let range = self.selection.range.range();
        if range.is_empty() {
            return None;
        }
        let content = self.content(self.selection.block)?;
        Some(format::Copied {
            blocks: vec![format::CopiedBlock::fragment(content, range)],
            is_fragment: true,
        })
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_selection.is_some() {
            self.copy(&Copy, window, cx);
            return self.delete_selected_blocks(cx);
        }
        let range = self.selection.range.range();
        if range.is_empty() {
            return;
        }
        self.copy(&Copy, window, cx);
        self.delete_range(range, cx);
    }

    /// Pastes what this app copied as blocks, or text from elsewhere read as Markdown. In a
    /// code block everything is pasted as literal text.
    pub(super) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let block = self.selection.block;
        let range = self.selection.range.range();
        let copied_here = item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::String(string) => string.metadata_json::<format::Copied>(),
            _ => None,
        });
        let Some(text) = item.text() else {
            return;
        };
        if self.kind_of(block) == BlockKind::Code {
            return self.type_text(range, &text.replace("\r\n", "\n"), cx);
        }
        let copied = copied_here.unwrap_or_else(|| format::from_markdown(&text));
        let is_current_empty = self
            .content(block)
            .is_none_or(|content| content.text.is_empty());
        self.apply_edit(cx, move |page| {
            paste_blocks(page, block, range.clone(), &copied, is_current_empty)
        });
    }
}

/// Puts `copied` at `range` in `block`: the first block's text joins the line (taking its
/// kind if the line was empty), and each further block follows as a block of its own, nested
/// under the pasted block above it as it was where it was copied.
pub(super) fn paste_blocks(
    page: &mut PageDocument,
    block: BlockId,
    range: Range<usize>,
    copied: &format::Copied,
    is_current_empty: bool,
) -> PageResult<Selection> {
    let Some((first, rest)) = copied.blocks.split_first() else {
        return Ok(Selection::caret(block, range.start));
    };
    page.delete_text(block, range.clone())?;
    insert_marked_text(page, block, range.start, first)?;
    if is_current_empty && !copied.is_fragment {
        page.set_kind(block, first.kind.block_kind())?;
        page.set_checked(block, first.is_checked)?;
    }
    let mut caret = range.start + first.text.len();
    let mut current = block;
    // The latest pasted block at each depth, to nest the next deeper one under.
    let mut parents: Vec<BlockId> = vec![block];
    for copied_block in rest {
        let next = page.split_block(current, caret, copied_block.kind.block_kind())?;
        let parent = copied_block
            .depth
            .checked_sub(1)
            .and_then(|depth| parents.get(depth).copied());
        if let Some(parent) = parent {
            page.move_block(next, Placement::LastChildOf(Some(parent)))?;
        }
        insert_marked_text(page, next, 0, copied_block)?;
        page.set_checked(next, copied_block.is_checked)?;
        parents.truncate(copied_block.depth);
        parents.push(next);
        caret = copied_block.text.len();
        current = next;
    }
    Ok(Selection::caret(current, caret))
}

pub(super) fn insert_marked_text(
    page: &mut PageDocument,
    block: BlockId,
    offset: usize,
    copied: &format::CopiedBlock,
) -> PageResult<()> {
    page.insert_text(block, offset, &copied.text)?;
    for span in &copied.spans {
        let range = offset + span.start..offset + span.end;
        let kinds = [
            (span.is_bold, MarkKind::Bold),
            (span.is_italic, MarkKind::Italic),
            (span.is_strikethrough, MarkKind::Strikethrough),
            (span.is_code, MarkKind::Code),
        ];
        for (is_marked, kind) in kinds {
            if is_marked {
                page.add_mark(block, range.clone(), kind)?;
            }
        }
        if let Some(url) = &span.link {
            page.add_link(block, range, url)?;
        }
    }
    Ok(())
}
