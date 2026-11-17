//! The outline: the page's headings listed beside it, with the ones whose sections are on
//! screen marked and kept in view as the page scrolls. Clicking a heading scrolls to it and
//! puts the caret there.

use std::ops::Range;

use gpui::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder, px,
};
use scroll_settings::{AnySetting, Category, Setting};
use scroll_ui::{Ellipsis, SectionHeader, Theme};

use super::PageEditor;
use crate::editor::document::{BlockId, BlockKind};

pub const SHOW_OUTLINE: Setting<bool> = Setting {
    key: "appearance.outline",
    category: Category::Appearance,
    label: "Outline",
    description: "Lists a document's headings beside it. Click one to go to it.",
    default: false,
};

pub static SETTINGS: &[&dyn AnySetting] = &[&SHOW_OUTLINE];

const OUTLINE_WIDTH: Pixels = px(232.0);
/// The rail down the outline's left edge, lit beside the headings on screen.
const RAIL_WIDTH: Pixels = px(2.0);
const UNTITLED_HEADING: &str = "Untitled";

struct OutlineEntry {
    block: BlockId,
    /// 1 for the largest heading.
    level: usize,
    title: SharedString,
    is_in_view: bool,
}

/// The outline's own scroll position, and what it last brought into view.
#[derive(Default)]
pub(super) struct OutlineScroll {
    handle: ScrollHandle,
    /// The first heading in view as of the last frame, so the outline follows the page when
    /// that changes and is otherwise left where the reader scrolled it.
    followed: Option<usize>,
}

fn heading_level(kind: BlockKind) -> Option<usize> {
    match kind {
        BlockKind::Heading1 => Some(1),
        BlockKind::Heading2 => Some(2),
        BlockKind::Heading3 => Some(3),
        _ => None,
    }
}

/// Which of `headings` (their places among the page's blocks, in order) are in view when the
/// blocks in `visible` are on screen: the heading whose section the top of the screen is in,
/// and every heading that starts on screen. A range of positions in `headings`.
fn headings_in_view(headings: &[usize], visible: &Range<usize>) -> Range<usize> {
    if visible.is_empty() {
        return 0..0;
    }
    let started_by_top = headings.partition_point(|heading| *heading <= visible.start);
    let started_by_bottom = headings.partition_point(|heading| *heading < visible.end);
    started_by_top.saturating_sub(1)..started_by_bottom
}

impl PageEditor {
    /// The blocks shown by the list rows `visible_rows`.
    fn visible_blocks(&self, visible_rows: &Range<usize>) -> Range<usize> {
        let header = self.header_rows();
        let first_row = visible_rows.start.saturating_sub(header);
        let Some(last_row) = visible_rows.end.checked_sub(header + 1) else {
            return 0..0;
        };
        if last_row >= self.rows.count() {
            return 0..0;
        }
        self.rows.span(first_row).start..self.rows.span(last_row).end
    }

    fn outline_entries(&self, visible_rows: &Range<usize>) -> Vec<OutlineEntry> {
        let headings: Vec<(usize, usize)> = self
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| Some((index, heading_level(block.info.kind)?)))
            .collect();
        let places: Vec<usize> = headings.iter().map(|(index, _)| *index).collect();
        let in_view = headings_in_view(&places, &self.visible_blocks(visible_rows));
        headings
            .into_iter()
            .enumerate()
            .map(|(position, (index, level))| {
                let block = &self.blocks[index];
                let title = block.content.text.lines().next().unwrap_or_default().trim();
                OutlineEntry {
                    block: block.info.id,
                    level,
                    title: if title.is_empty() {
                        UNTITLED_HEADING.into()
                    } else {
                        title.to_string().into()
                    },
                    is_in_view: in_view.contains(&position),
                }
            })
            .collect()
    }

    /// Each heading's title and whether the outline marks it as in view, as of the last frame.
    #[cfg(test)]
    pub(super) fn outline_in_view(&self) -> Vec<(SharedString, bool)> {
        self.outline_entries(&self.scroll.visible_rows())
            .into_iter()
            .map(|entry| (entry.title, entry.is_in_view))
            .collect()
    }

    /// The outline beside the page, while its setting is on.
    pub(super) fn render_outline(
        &mut self,
        visible_rows: &Range<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !SHOW_OUTLINE.get(cx) {
            return None;
        }
        let entries = self.outline_entries(visible_rows);
        let is_empty = entries.is_empty();
        self.follow_page_in_outline(&entries);
        // Headings indent from the largest the page uses, so a page without a top-level one
        // does not start indented.
        let top_level = entries.iter().map(|entry| entry.level).min().unwrap_or(1);
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .map(|(position, entry)| {
                let depth = entry.level - top_level;
                self.render_outline_entry(position, depth, entry, theme, cx)
            })
            .collect();
        Some(
            div()
                .id("page-outline")
                .debug_selector(|| "page-outline".into())
                .flex_none()
                .w(OUTLINE_WIDTH)
                .h_full()
                .overflow_y_scroll()
                .track_scroll(&self.outline_scroll.handle)
                .cursor_default()
                .border_l(theme.size.border_width)
                .border_color(theme.colors.border)
                .px(theme.spacing.sm)
                .pb(theme.spacing.lg)
                .text_size(theme.text.ui.size)
                .line_height(theme.text.ui.line_height)
                // A press here is the outline's own: it must not reach the page, which would
                // put the caret in its last block.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(SectionHeader::new("Outline"))
                .when(is_empty, |outline| {
                    outline.child(
                        div()
                            .px(theme.spacing.sm)
                            .text_color(theme.colors.text_placeholder)
                            .child("No headings on this page"),
                    )
                })
                .children(rows)
                .into_any_element(),
        )
    }

    /// Scrolls the outline to the first heading in view when that has changed.
    fn follow_page_in_outline(&mut self, entries: &[OutlineEntry]) {
        let first_in_view = entries.iter().position(|entry| entry.is_in_view);
        if first_in_view == self.outline_scroll.followed {
            return;
        }
        self.outline_scroll.followed = first_in_view;
        if let Some(position) = first_in_view {
            // The outline's first child is its title; its headings follow.
            self.outline_scroll.handle.scroll_to_item(position + 1);
        }
    }

    /// One heading in the outline, `depth` levels in from the largest heading on the page.
    fn render_outline_entry(
        &self,
        position: usize,
        depth: usize,
        entry: OutlineEntry,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let block = entry.block;
        let hover = theme.colors.hover;
        let (text, rail) = if entry.is_in_view {
            (theme.colors.text, theme.colors.accent)
        } else {
            (theme.colors.text_muted, theme.colors.border)
        };
        div()
            .id(ElementId::NamedInteger(
                "outline-entry".into(),
                position as u64,
            ))
            .debug_selector(|| format!("outline-entry-{position}"))
            .border_l(RAIL_WIDTH)
            .border_color(rail)
            .pl(theme.spacing.sm + theme.spacing.md * depth as f32)
            .pr(theme.spacing.xs)
            .py(theme.spacing.xxs)
            .rounded_r(theme.radius.sm)
            .cursor_pointer()
            .text_color(text)
            .when(entry.is_in_view, |row| row.font_weight(FontWeight::MEDIUM))
            .hover(move |row| row.bg(hover))
            .ellipsis()
            .child(entry.title)
            .on_click(
                cx.listener(move |editor, _, window, cx| editor.go_to_heading(block, window, cx)),
            )
            .into_any_element()
    }

    /// Scrolls the heading to the top of the page area and puts the caret at its start, so
    /// reading or typing carries on from there. Revealing the caret then settles the heading a
    /// couple of lines down from the edge, where typing in it will not move the page again.
    fn go_to_heading(&mut self, block: BlockId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.block_index(block) else {
            return;
        };
        window.focus(&self.focus_handle);
        self.scroll.scroll_row_to_top(self.list_row(index));
        self.move_caret(block, 0, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_at_the_top_and_headings_starting_on_screen_are_in_view() {
        // Headings at blocks 0, 10, 20 and 30.
        let headings = [0, 10, 20, 30];
        assert_eq!(headings_in_view(&headings, &(0..5)), 0..1);
        assert_eq!(
            headings_in_view(&headings, &(12..18)),
            1..2,
            "inside the second section, whose heading has scrolled off"
        );
        assert_eq!(headings_in_view(&headings, &(8..22)), 0..3);
        assert_eq!(headings_in_view(&headings, &(10..11)), 1..2);
        assert_eq!(headings_in_view(&headings, &(35..40)), 3..4);
    }

    #[test]
    fn text_above_the_first_heading_belongs_to_no_section() {
        assert_eq!(headings_in_view(&[5, 9], &(0..3)), 0..0);
        assert_eq!(headings_in_view(&[5, 9], &(0..6)), 0..1);
        assert_eq!(headings_in_view(&[], &(0..6)), 0..0);
        assert_eq!(
            headings_in_view(&[5, 9], &(0..0)),
            0..0,
            "nothing on screen"
        );
    }
}
