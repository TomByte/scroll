use std::ops::Range;

use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, Window, actions, div, prelude::FluentBuilder, uniform_list,
};

use crate::controls::text_input::{TextInput, TextInputEvent};
use crate::display::scrollbar::Scrollbar;
use crate::emoji::{Emoji, all_emoji, recent_emoji, remember_emoji, search_emoji};
use crate::foundation::actions::{Confirm, TEXT_INPUT_CONTEXT};
use crate::foundation::ellipsis::Ellipsis;
use crate::theme::Theme;
use crate::{Button, ButtonSize, ButtonVariant};
use gpui::AnyView;

actions!(
    emoji_picker,
    [HighlightLeft, HighlightRight, HighlightUp, HighlightDown]
);

const KEY_CONTEXT: &str = "EmojiPicker";
const COLUMNS: usize = 9;

/// The arrows move through the grid even while the search field has focus.
pub(crate) fn bind_keys(cx: &mut App) {
    let context = format!("{KEY_CONTEXT} > {TEXT_INPUT_CONTEXT}");
    let context = Some(context.as_str());
    cx.bind_keys([
        KeyBinding::new("left", HighlightLeft, context),
        KeyBinding::new("right", HighlightRight, context),
        KeyBinding::new("up", HighlightUp, context),
        KeyBinding::new("down", HighlightDown, context),
        KeyBinding::new("enter", Confirm, context),
    ]);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmojiPickerEvent {
    Picked(SharedString),
    /// "Remove" was chosen; only offered when built with `removable`.
    Removed,
}

/// A row of the grid: a section heading, or up to `COLUMNS` emoji from `cells`.
#[derive(Clone, Debug)]
enum Row {
    Heading(SharedString),
    Cells(Range<usize>),
}

/// Searchable, grouped, virtualised emoji grid with recently used emoji first. Arrows move the
/// highlight, Enter picks it, Escape dismisses. Opened by a `PopupHost`, or shown in place.
pub struct EmojiPicker {
    search: Entity<TextInput>,
    cells: Vec<&'static str>,
    /// The name shown in the footer for each cell.
    names: Vec<&'static str>,
    rows: Vec<Row>,
    highlighted: usize,
    is_removable: bool,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<EmojiPickerEvent> for EmojiPicker {}
impl EventEmitter<DismissEvent> for EmojiPicker {}

impl Focusable for EmojiPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
}

impl EmojiPicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx).placeholder("Search emoji"));
        let subscription = cx.subscribe(&search, |picker, _, event, cx| match event {
            TextInputEvent::Changed => picker.rebuild(cx),
            TextInputEvent::Submitted => picker.pick_highlighted(cx),
            TextInputEvent::Cancelled => cx.emit(DismissEvent),
        });
        let mut picker = EmojiPicker {
            search,
            cells: Vec::new(),
            names: Vec::new(),
            rows: Vec::new(),
            highlighted: 0,
            is_removable: false,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        };
        picker.rebuild(cx);
        picker
    }

    /// Offers "Remove", e.g. for a page that already has an icon.
    pub fn removable(mut self) -> Self {
        self.is_removable = true;
        self
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).text().to_string();
        let sections: Vec<(SharedString, Vec<&'static Emoji>)> = if query.trim().is_empty() {
            grouped_sections(cx)
        } else {
            vec![("Results".into(), search_emoji(&query))]
        };
        self.cells.clear();
        self.names.clear();
        self.rows.clear();
        for (heading, emoji) in sections {
            if emoji.is_empty() {
                continue;
            }
            self.rows.push(Row::Heading(heading));
            let start = self.cells.len();
            self.cells.extend(emoji.iter().map(|emoji| emoji.emoji));
            self.names.extend(emoji.iter().map(|emoji| emoji.name));
            let end = self.cells.len();
            self.rows.extend(
                (start..end)
                    .step_by(COLUMNS)
                    .map(|row_start| Row::Cells(row_start..(row_start + COLUMNS).min(end))),
            );
        }
        self.highlighted = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn move_highlight(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.cells.is_empty() {
            return;
        }
        let last = self.cells.len() as isize - 1;
        self.highlighted = (self.highlighted as isize + step).clamp(0, last) as usize;
        if let Some(row) = self.row_of(self.highlighted) {
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn row_of(&self, cell: usize) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Cells(range) if range.contains(&cell)))
    }

    fn pick_highlighted(&mut self, cx: &mut Context<Self>) {
        if let Some(emoji) = self.cells.get(self.highlighted).copied() {
            self.pick(emoji, cx);
        }
    }

    fn pick(&mut self, emoji: &'static str, cx: &mut Context<Self>) {
        remember_emoji(emoji, cx);
        cx.emit(EmojiPickerEvent::Picked(emoji.into()));
        cx.emit(DismissEvent);
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        range
            .filter_map(|index| self.rows.get(index).cloned())
            .map(|row| match row {
                Row::Heading(heading) => div()
                    .flex()
                    .items_end()
                    .h(theme.size.emoji_cell)
                    .px(theme.spacing.xs)
                    .pb(theme.spacing.xxs)
                    .text_size(theme.text.caption.size)
                    .text_color(theme.colors.text_placeholder)
                    .child(heading)
                    .into_any_element(),
                Row::Cells(cells) => div()
                    .flex()
                    .h(theme.size.emoji_cell)
                    .children(cells.map(|cell| self.render_cell(cell, theme, cx)))
                    .into_any_element(),
            })
            .collect()
    }

    fn render_cell(&self, cell: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let emoji = self.cells[cell];
        div()
            .id(ElementId::Integer(cell as u64))
            .flex()
            .items_center()
            .justify_center()
            .size(theme.size.emoji_cell)
            .rounded(theme.radius.md)
            // Not clipped: a few newer glyphs draw a little wider than their advance. 🫡 is
            // still cut on its right, inside GPUI: font-kit's raster bounds for it are narrower
            // than Apple's image.
            .text_size(theme.text.heading_3.size)
            .when(cell == self.highlighted, |cell| {
                cell.bg(theme.colors.selected)
            })
            .hover(|cell| cell.bg(theme.colors.hover))
            .cursor_pointer()
            .child(emoji)
            .on_mouse_move(cx.listener(move |picker, _, _, cx| {
                if picker.highlighted != cell {
                    picker.highlighted = cell;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |picker, _, _, cx| picker.pick(emoji, cx)))
            .into_any_element()
    }
}

/// Recently used first, then every group in Unicode's order.
fn grouped_sections(cx: &App) -> Vec<(SharedString, Vec<&'static Emoji>)> {
    let recent: Vec<&'static Emoji> = recent_emoji(cx)
        .iter()
        .filter_map(|recent| all_emoji().iter().find(|emoji| emoji.emoji == recent))
        .collect();
    let mut sections: Vec<(SharedString, Vec<&'static Emoji>)> =
        vec![("Recently used".into(), recent)];
    for emoji in all_emoji() {
        match sections.last_mut() {
            Some((group, members)) if group.as_ref() == emoji.group => members.push(emoji),
            _ => sections.push((emoji.group.into(), vec![emoji])),
        }
    }
    sections
}

impl Render for EmojiPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let row_count = self.rows.len();
        let list_theme = theme.clone();
        let footer = self
            .names
            .get(self.highlighted)
            .map(|name| format!("{}  {name}", self.cells[self.highlighted]))
            .unwrap_or_else(|| "No emoji match".into());
        div()
            .key_context(KEY_CONTEXT)
            .on_action(
                cx.listener(|picker, _: &HighlightLeft, _, cx| picker.move_highlight(-1, cx)),
            )
            .on_action(
                cx.listener(|picker, _: &HighlightRight, _, cx| picker.move_highlight(1, cx)),
            )
            .on_action(cx.listener(|picker, _: &HighlightUp, _, cx| {
                picker.move_highlight(-(COLUMNS as isize), cx)
            }))
            .on_action(cx.listener(|picker, _: &HighlightDown, _, cx| {
                picker.move_highlight(COLUMNS as isize, cx)
            }))
            .on_action(cx.listener(|picker, _: &Confirm, _, cx| picker.pick_highlighted(cx)))
            .flex()
            .flex_col()
            .gap(theme.spacing.sm)
            .p(theme.spacing.sm)
            .w(theme.size.emoji_cell * COLUMNS as f32 + theme.spacing.sm * 2.0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.spacing.sm)
                    .child(div().flex_1().child(self.search.clone()))
                    .when(self.is_removable, |header| {
                        header.child(
                            Button::new("remove-emoji", "Remove")
                                .size(ButtonSize::Small)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(EmojiPickerEvent::Removed);
                                    cx.emit(DismissEvent);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .h(theme.size.emoji_grid_height)
                    .child(
                        uniform_list(
                            "emoji-rows",
                            row_count,
                            cx.processor(move |picker, range, _, cx| {
                                picker.render_rows(range, &list_theme, cx)
                            }),
                        )
                        .track_scroll(self.scroll.clone())
                        .size_full(),
                    )
                    .child(Scrollbar::new("emoji-scrollbar", self.scroll.clone())),
            )
            .child(
                div()
                    .px(theme.spacing.xs)
                    .text_size(theme.text.small.size)
                    .text_color(theme.colors.text_muted)
                    .ellipsis()
                    .child(footer),
            )
    }
}

impl EmojiPicker {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let picker = cx.new(|cx| EmojiPicker::new(cx).removable());
        cx.new(|_| EmojiPickerStory { picker }).into()
    }
}

/// Shown in place; in the app it opens in a popover.
struct EmojiPickerStory {
    picker: Entity<EmojiPicker>,
}

impl Render for EmojiPickerStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div().flex().child(
            div()
                .rounded(theme.radius.lg)
                .border(theme.size.border_width)
                .border_color(theme.colors.border)
                .bg(theme.colors.surface_elevated)
                .shadow(theme.shadow.popover.clone())
                .child(self.picker.clone()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(EmojiPicker::story, cx);
    }
}
