//! What each block row hands its element to paint: text runs and styling, the placeholder,
//! and other people's carets and selections in it.

use super::Collaborator;
use super::PageEditor;
use super::block_element::{BlockPaint, Placeholder, RemotePresence};
use super::render::{is_heading, text_size};
use super::rich_text::RunStyle;
use crate::editor::document::{BlockId, BlockKind};
use gpui::{FontWeight, Hsla, SharedString, TextRun, font};
use scroll_ui::Theme;

impl PageEditor {
    pub(super) fn block_paint(&self, index: usize, theme: &Theme, is_focused: bool) -> BlockPaint {
        let block = &self.blocks[index];
        let id = block.info.id;
        let is_selected_block = id == self.selection.block && self.block_selection.is_none();
        let kind = block.info.kind;
        let text = text_size(kind, theme);
        let is_checked_to_do = kind == BlockKind::ToDo && block.info.is_checked;
        let run_style = RunStyle {
            font_family: if kind == BlockKind::Code {
                theme.font.mono.clone()
            } else {
                theme.font.ui.clone()
            },
            code_font_family: theme.font.mono.clone(),
            font_weight: if is_heading(kind) {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            },
            text_color: if is_checked_to_do {
                theme.colors.text_placeholder
            } else {
                theme.colors.text
            },
            link_color: theme.colors.link,
            code_background: theme.colors.code_background,
            decoration_thickness: theme.size.border_width,
            is_struck_through: is_checked_to_do,
        };
        let content = self.display_content(id);
        let content = if kind == BlockKind::Code {
            std::borrow::Cow::Owned(content.without_marks())
        } else {
            content
        };
        let marked_range = self.display_marked_range().filter(|_| is_selected_block);
        let selection = self.display_selection();
        let selection_range = selection.range();
        let shows_caret =
            is_selected_block && is_focused && self.is_caret_visible && selection_range.is_empty();

        BlockPaint {
            text: SharedString::from(content.text.clone()),
            runs: content.text_runs(&run_style, marked_range.as_ref()),
            font_size: text.size,
            line_height: text.line_height,
            selection: (is_selected_block && !selection_range.is_empty())
                .then_some(selection_range),
            caret: shows_caret.then_some(selection.head),
            remote: self.remote_presence_in(id, theme),
            placeholder: self.placeholder_for(index, is_selected_block && is_focused, theme),
            selection_color: theme.colors.text_selection,
            caret_color: theme.colors.caret,
            caret_width: theme.size.caret_width,
        }
    }

    pub(super) fn placeholder_for(
        &self,
        index: usize,
        is_active: bool,
        theme: &Theme,
    ) -> Option<Placeholder> {
        let block = &self.blocks[index];
        if !block.content.text.is_empty()
            || self
                .composition
                .as_ref()
                .is_some_and(|c| c.block == block.info.id)
        {
            return None;
        }
        let text = match block.info.kind {
            BlockKind::Paragraph if is_active => "Write, or type “/” for commands",
            BlockKind::Heading1 => "Heading 1",
            BlockKind::Heading2 => "Heading 2",
            BlockKind::Heading3 => "Heading 3",
            BlockKind::ToDo if is_active => "To-do",
            BlockKind::BulletedListItem | BlockKind::NumberedListItem if is_active => "List",
            BlockKind::Quote if is_active => "Quote",
            BlockKind::Callout if is_active => "Callout",
            _ => return None,
        };
        let weight = if is_heading(block.info.kind) {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        Some(Placeholder {
            text: text.into(),
            run: plain_run(theme, theme.colors.text_placeholder, weight),
        })
    }

    pub(super) fn remote_presence_in(&self, block: BlockId, theme: &Theme) -> Vec<RemotePresence> {
        self.collaborators
            .values()
            .filter_map(|collaborator| self.collaborator_in(collaborator, block, theme))
            .collect()
    }

    pub(super) fn collaborator_in(
        &self,
        collaborator: &Collaborator,
        block: BlockId,
        theme: &Theme,
    ) -> Option<RemotePresence> {
        let head = self.page.resolve_cursor(&collaborator.head);
        let anchor = collaborator
            .anchor
            .as_ref()
            .and_then(|anchor| self.page.resolve_cursor(anchor));
        let caret = head
            .filter(|(head_block, _)| *head_block == block)
            .map(|(_, offset)| offset);
        let selection = match (anchor, head) {
            (Some((anchor_block, anchor)), Some((head_block, head)))
                if anchor_block == block && head_block == block =>
            {
                Some(anchor.min(head)..anchor.max(head))
            }
            _ => None,
        };
        if caret.is_none() && selection.is_none() {
            return None;
        }
        Some(RemotePresence {
            caret,
            selection,
            color: theme.collaborator_color(collaborator.color_index),
            label: collaborator.name.clone(),
            label_run: plain_run(theme, theme.colors.text_on_accent, FontWeight::MEDIUM),
            label_font_size: theme.text.caption.size,
            label_padding: theme.spacing.xs,
        })
    }
}

pub(super) fn plain_run(theme: &Theme, color: Hsla, weight: FontWeight) -> TextRun {
    TextRun {
        len: 0,
        font: gpui::Font {
            weight,
            ..font(theme.font.ui.clone())
        },
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}
