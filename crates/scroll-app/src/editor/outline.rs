//! The outline: the page's headings listed beside it, with the one the reader is on marked and
//! kept in view as the page scrolls. Clicking a heading scrolls to it and puts the caret
//! there. The outline's left edge drags to resize it, and a double click there resets it.

use std::ops::Range;

use gpui::{
    AnyElement, App, Context, DispatchPhase, ElementId, Global, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, prelude::FluentBuilder,
    px,
};
use scroll_settings::{AnySetting, Category, Setting};
use scroll_ui::{Ellipsis, ResizeHandle, SectionHeader, Theme};

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

const DEFAULT_WIDTH: Pixels = px(232.0);
const MIN_WIDTH: Pixels = px(176.0);
const MAX_WIDTH: Pixels = px(420.0);
const UNTITLED_HEADING: &str = "Untitled";

/// How wide the outline was last dragged. It is as wide beside every page, and the window keeps
/// the width between runs.
#[derive(Clone, Copy)]
struct DraggedWidth(Pixels);

impl Global for DraggedWidth {}

/// The width the outline was last dragged to; `None` if it never was.
pub fn dragged_width(cx: &App) -> Option<Pixels> {
    cx.try_global::<DraggedWidth>().map(|dragged| dragged.0)
}

/// Makes the outline `width` wide, or as near to that as it may be.
pub fn set_width(width: Pixels, cx: &mut App) {
    cx.set_global(DraggedWidth(width.clamp(MIN_WIDTH, MAX_WIDTH)));
}

struct OutlineEntry {
    block: BlockId,
    /// 1 for the largest heading.
    level: usize,
    title: SharedString,
    /// Whether it is the heading the reader is on.
    is_current: bool,
}

/// The outline's own state: where it is scrolled to, what it last brought into view, and a drag
/// of its edge.
#[derive(Default)]
pub(super) struct OutlinePane {
    scroll: ScrollHandle,
    /// The current heading as of the last frame, so the outline follows the page when that
    /// changes and is otherwise left where the reader scrolled it.
    followed: Option<usize>,
    resizing: Option<Resizing>,
}

/// A drag of the outline's edge in progress.
#[derive(Clone, Copy)]
struct Resizing {
    start_x: Pixels,
    start_width: Pixels,
}

fn heading_level(kind: BlockKind) -> Option<usize> {
    match kind {
        BlockKind::Heading1 => Some(1),
        BlockKind::Heading2 => Some(2),
        BlockKind::Heading3 => Some(3),
        _ => None,
    }
}

/// The heading the reader is on while they are at the block `place` and the blocks in `visible`
/// are on screen, as a position in `headings` (their places among the page's blocks, in order):
/// the heading of the section `place` is in. Above the first heading there is no section, so
/// it is that heading once it is on screen.
fn current_heading(headings: &[usize], place: usize, visible: &Range<usize>) -> Option<usize> {
    let started_by_place = headings.partition_point(|heading| *heading <= place);
    if started_by_place > 0 {
        return Some(started_by_place - 1);
    }
    let first = headings.first()?;
    visible.contains(first).then_some(0)
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

    /// The block the reader is at: the one the caret is in or that is selected while that is on
    /// screen, since that is where they clicked or are typing, and otherwise the one at the top
    /// of the screen.
    fn reading_place(&self, visible: &Range<usize>) -> Option<usize> {
        if visible.is_empty() {
            return None;
        }
        let chosen = match self.block_selection {
            Some(selection) => selection.head,
            None => self.selection.block,
        };
        let chosen_on_screen = self.block_index(chosen).filter(|at| visible.contains(at));
        Some(chosen_on_screen.unwrap_or(visible.start))
    }

    fn outline_entries(&self, visible_rows: &Range<usize>) -> Vec<OutlineEntry> {
        let headings: Vec<(usize, usize)> = self
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| Some((index, heading_level(block.info.kind)?)))
            .collect();
        let places: Vec<usize> = headings.iter().map(|(index, _)| *index).collect();
        let visible = self.visible_blocks(visible_rows);
        let current = self
            .reading_place(&visible)
            .and_then(|place| current_heading(&places, place, &visible));
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
                    is_current: current == Some(position),
                }
            })
            .collect()
    }

    /// Each heading's title and whether the outline marks it as the current one, as of the
    /// last frame.
    #[cfg(test)]
    pub(super) fn outline_headings(&self) -> Vec<(SharedString, bool)> {
        self.outline_entries(&self.scroll.visible_rows())
            .into_iter()
            .map(|entry| (entry.title, entry.is_current))
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
        let headings = div()
            .id("outline-headings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.outline.scroll)
            .px(theme.spacing.sm)
            .pb(theme.spacing.lg)
            .text_size(theme.text.ui.size)
            .line_height(theme.text.ui.line_height)
            .child(SectionHeader::new("Outline"))
            .when(is_empty, |outline| {
                outline.child(
                    div()
                        .px(theme.spacing.sm)
                        .text_color(theme.colors.text_placeholder)
                        .child("No headings on this page"),
                )
            })
            .children(rows);
        let resize_handle =
            ResizeHandle::new("outline-resize")
                .on_left_edge()
                .on_press(cx.listener(|editor, event: &MouseDownEvent, _, cx| {
                    editor.start_resizing_outline(event, cx)
                }));
        Some(
            div()
                .debug_selector(|| "page-outline".into())
                .relative()
                .flex_none()
                .w(dragged_width(cx).unwrap_or(DEFAULT_WIDTH))
                .h_full()
                .cursor_default()
                .border_l(theme.size.border_width)
                .border_color(theme.colors.border)
                // A press here is the outline's own: it must not reach the page, which would
                // put the caret in its last block.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(headings)
                .child(resize_handle)
                .when(self.outline.resizing.is_some(), |outline| {
                    outline.child(self.outline_resize_tracker(cx))
                })
                .into_any_element(),
        )
    }

    /// Scrolls the outline to the current heading when that has changed.
    fn follow_page_in_outline(&mut self, entries: &[OutlineEntry]) {
        let current = entries.iter().position(|entry| entry.is_current);
        if current == self.outline.followed {
            return;
        }
        self.outline.followed = current;
        if let Some(position) = current {
            // The list's first child is its title; its headings follow.
            self.outline.scroll.scroll_to_item(position + 1);
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
        let (hover, strong) = (theme.colors.hover, theme.colors.text);
        // Every heading is quiet but the one the reader is on, which is also underlined: a
        // border under the words, since that sits lower and finer than the font's own line.
        let (text, underline) = if entry.is_current {
            (strong, strong)
        } else {
            (theme.colors.text_muted, gpui::transparent_black())
        };
        div()
            .id(ElementId::NamedInteger(
                "outline-entry".into(),
                position as u64,
            ))
            .debug_selector(|| format!("outline-entry-{position}"))
            // A row, so that the title in it is only as wide as its words.
            .flex()
            .pl(theme.spacing.sm + theme.spacing.lg * depth as f32)
            .pr(theme.spacing.sm)
            .py(theme.spacing.xxs)
            .rounded(theme.radius.sm)
            .cursor_pointer()
            .text_color(text)
            .hover(move |row| row.bg(hover).text_color(strong))
            .child(
                div()
                    .min_w_0()
                    .ellipsis()
                    .border_b(theme.size.border_width)
                    .border_color(underline)
                    .child(entry.title),
            )
            .on_click(
                cx.listener(move |editor, _, window, cx| editor.go_to_heading(block, window, cx)),
            )
            .into_any_element()
    }

    fn start_resizing_outline(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.notify();
        if event.click_count == 2 {
            self.outline.resizing = None;
            return set_width(DEFAULT_WIDTH, cx);
        }
        self.outline.resizing = Some(Resizing {
            start_x: event.position.x,
            start_width: dragged_width(cx).unwrap_or(DEFAULT_WIDTH),
        });
    }

    fn resize_outline_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(resizing) = self.outline.resizing else {
            return;
        };
        // The outline is at the right of the window, so dragging its edge left widens it.
        set_width(resizing.start_width - (x - resizing.start_x), cx);
        cx.notify();
    }

    fn end_resizing_outline(&mut self, cx: &mut Context<Self>) {
        if self.outline.resizing.take().is_some() {
            cx.notify();
        }
    }

    /// While the edge is being dragged, follows the pointer anywhere in the window until
    /// release.
    fn outline_resize_tracker(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let editor = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let on_move = editor.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && let Some(editor) = on_move.upgrade()
                    {
                        editor.update(cx, |editor, cx| {
                            editor.resize_outline_to(event.position.x, cx)
                        });
                    }
                });
                let on_up = editor.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && let Some(editor) = on_up.upgrade()
                    {
                        editor.update(cx, |editor, cx| editor.end_resizing_outline(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full()
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
    fn the_current_heading_is_the_one_whose_section_the_reader_is_in() {
        // Headings at blocks 0, 10, 20 and 30, with blocks 12 to 17 on screen.
        let headings = [0, 10, 20, 30];
        let visible = 12..18;
        assert_eq!(
            current_heading(&headings, 12, &visible),
            Some(1),
            "in the second section, whose heading has scrolled off"
        );
        assert_eq!(current_heading(&headings, 10, &(8..22)), Some(1));
        assert_eq!(current_heading(&headings, 20, &(8..22)), Some(2));
        assert_eq!(current_heading(&headings, 35, &(33..40)), Some(3));
    }

    #[test]
    fn above_the_first_heading_it_is_that_heading_once_on_screen() {
        assert_eq!(current_heading(&[5, 9], 0, &(0..3)), None);
        assert_eq!(current_heading(&[5, 9], 0, &(0..6)), Some(0));
        assert_eq!(current_heading(&[], 0, &(0..6)), None);
    }
}
