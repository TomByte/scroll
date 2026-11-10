//! Single-line text field: caret, keyboard and mouse selection, clipboard, IME, undo.
//! The line either scrolls sideways or, with `wrapping()`, wraps onto as many visual lines
//! as it needs.

mod element;
mod history;
mod shaped_text;

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render,
    SharedString, Styled, UTF16Selection, Window, div, point, prelude::FluentBuilder,
};

use crate::foundation::actions::{
    Backspace, Cancel, Copy, Cut, Delete, DeleteToLineStart, DeleteWordBackward, DeleteWordForward,
    MoveDown, MoveLeft, MoveRight, MoveToEnd, MoveToLineEnd, MoveToLineStart, MoveToStart, MoveUp,
    MoveWordLeft, MoveWordRight, Newline, Paste, Redo, SelectAll, SelectLeft, SelectRight,
    SelectToEnd, SelectToLineEnd, SelectToLineStart, SelectToStart, SelectWordLeft,
    SelectWordRight, ShowCharacterPalette, TEXT_INPUT_CONTEXT, Undo,
};
use crate::foundation::focus::focus_outline;
use crate::text::TextSelection;
use crate::text::{
    next_grapheme_boundary, next_word_end, previous_grapheme_boundary, previous_word_start,
    utf8_range_to_utf16, utf8_to_utf16, utf16_range_to_utf8, word_range_at,
};
use crate::theme::{TextSize, Theme};
use element::TextInputElement;
use gpui::{AnyView, AppContext, Entity};
use history::{EditHistory, EditKind, Snapshot};
use shaped_text::ShapedText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    Changed,
    /// Enter was pressed.
    Submitted,
    /// Escape was pressed. The event also continues to enclosing popovers and modals.
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Appearance {
    /// A bordered control with a focus ring.
    #[default]
    Boxed,
    /// Bare text, for editing in place: an inline label or a page title.
    Plain,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    text: String,
    placeholder: SharedString,
    selection: TextSelection,
    marked_range: Option<Range<usize>>,
    history: EditHistory,
    is_dragging: bool,
    layout: Option<LineLayoutCache>,
    appearance: Appearance,
    text_size: Option<TextSize>,
    font_weight: FontWeight,
    is_masked: bool,
    access: InputAccess,
    is_wrapping: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputAccess {
    #[default]
    Editable,
    ReadOnly,
}

/// Shown in place of each character of a masked field.
const MASK: char = '•';

/// What the element last painted, kept for hit-testing and IME geometry.
struct LineLayoutCache {
    text: ShapedText,
    bounds: Bounds<Pixels>,
    scroll_offset: Pixels,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        TextInput {
            focus_handle: cx.focus_handle().tab_stop(true),
            text: String::new(),
            placeholder: SharedString::default(),
            selection: TextSelection::default(),
            marked_range: None,
            history: EditHistory::default(),
            is_dragging: false,
            layout: None,
            appearance: Appearance::default(),
            text_size: None,
            font_weight: FontWeight::NORMAL,
            is_masked: false,
            access: InputAccess::Editable,
            is_wrapping: false,
        }
    }

    /// Starting text, not recorded as an undoable edit.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self.selection = TextSelection::caret(self.text.len());
        self
    }

    /// Draws the field as bare text with no border, background or padding.
    pub fn plain(mut self) -> Self {
        self.appearance = Appearance::Plain;
        self
    }

    /// Defaults to the theme's UI text size.
    pub fn text_size(mut self, size: TextSize) -> Self {
        self.text_size = Some(size);
        self
    }

    pub fn font_weight(mut self, weight: FontWeight) -> Self {
        self.font_weight = weight;
        self
    }

    /// Selects all the text, e.g. when starting to edit in place.
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selection = TextSelection::from_range(0..self.text.len(), false);
        cx.notify();
    }

    /// Whether typing changes the text; a read-only field can still be selected and copied.
    pub fn set_access(&mut self, access: InputAccess, cx: &mut Context<Self>) {
        self.access = access;
        cx.notify();
    }

    /// Shows a dot for each character and never copies, as for a password.
    pub fn masked(mut self) -> Self {
        self.is_masked = true;
        self
    }

    /// Wraps the text to the field's width and grows a line at a time to fit it, as for a
    /// page title. It stays one logical line: Enter still submits.
    pub fn wrapping(mut self) -> Self {
        self.is_wrapping = true;
        self
    }

    /// The text as drawn: itself, or one dot per character when masked.
    fn display_text(&self) -> String {
        if self.is_masked {
            self.text.chars().map(|_| MASK).collect()
        } else {
            self.text.clone()
        }
    }

    /// Where text offset `offset` falls in the drawn text.
    fn display_offset(&self, offset: usize) -> usize {
        if self.is_masked {
            self.text[..offset.min(self.text.len())].chars().count() * MASK.len_utf8()
        } else {
            offset
        }
    }

    fn text_offset(&self, display: usize) -> usize {
        if !self.is_masked {
            return display;
        }
        let characters = display / MASK.len_utf8();
        self.text
            .char_indices()
            .nth(characters)
            .map_or(self.text.len(), |(offset, _)| offset)
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the whole text as one undoable step and puts the caret at the end.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.history.record(self.snapshot(), EditKind::Other);
        self.text = text.into();
        self.selection = TextSelection::caret(self.text.len());
        self.marked_range = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            selection: self.selection,
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.text = snapshot.text;
        self.selection = snapshot.selection.clamped(self.text.len());
        self.marked_range = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = TextSelection::caret(offset);
        self.history.close_run();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = self.selection.extended_to(offset);
        self.history.close_run();
        cx.notify();
    }

    /// Replaces `range` with `new_text` and leaves the caret after the inserted text.
    fn replace(&mut self, range: Range<usize>, new_text: &str, cx: &mut Context<Self>) {
        if self.access == InputAccess::ReadOnly {
            return;
        }
        let kind = if new_text.is_empty() {
            EditKind::Deleting
        } else {
            EditKind::Typing
        };
        self.history.record(self.snapshot(), kind);
        self.text.replace_range(range.clone(), new_text);
        self.selection = TextSelection::caret(range.start + new_text.len());
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn delete_selection_or(&mut self, target: usize, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() {
            let caret = self.selection.head;
            caret.min(target)..caret.max(target)
        } else {
            self.selection.range()
        };
        if !range.is_empty() {
            self.replace(range, "", cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let target = previous_grapheme_boundary(&self.text, self.selection.head);
        self.delete_selection_or(target, cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_grapheme_boundary(&self.text, self.selection.head);
        self.delete_selection_or(target, cx);
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = previous_word_start(&self.text, self.selection.head);
        self.delete_selection_or(target, cx);
    }

    fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = next_word_end(&self.text, self.selection.head);
        self.delete_selection_or(target, cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selection_or(0, cx);
    }

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            previous_grapheme_boundary(&self.text, self.selection.head)
        } else {
            self.selection.range().start
        };
        self.move_to(target, cx);
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            next_grapheme_boundary(&self.text, self.selection.head)
        } else {
            self.selection.range().end
        };
        self.move_to(target, cx);
    }

    /// A single line has nothing above or below, so vertical movement is left to the
    /// parent (a command palette uses it to move through results).
    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.propagate();
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.propagate();
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(previous_word_start(&self.text, self.selection.head), cx);
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(next_word_end(&self.text, self.selection.head), cx);
    }

    fn move_to_line_start(&mut self, _: &MoveToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn move_to_line_end(&mut self, _: &MoveToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), cx);
    }

    fn move_to_start(&mut self, _: &MoveToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(
            previous_grapheme_boundary(&self.text, self.selection.head),
            cx,
        );
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_grapheme_boundary(&self.text, self.selection.head), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(previous_word_start(&self.text, self.selection.head), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_word_end(&self.text, self.selection.head), cx);
    }

    fn select_to_line_start(
        &mut self,
        _: &SelectToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    fn select_to_line_end(&mut self, _: &SelectToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.len(), cx);
    }

    fn select_to_start(&mut self, _: &SelectToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.len(), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = TextSelection::from_range(0..self.text.len(), false);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() || self.is_masked {
            return;
        }
        let selected = self.text[self.selection.range()].to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(selected));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        self.copy(&Copy, window, cx);
        self.replace(self.selection.range(), "", cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(pasted) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let single_line = pasted.replace(['\r', '\n'], " ");
        self.history.close_run();
        self.replace(self.selection.range(), &single_line, cx);
        self.history.close_run();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.access == InputAccess::ReadOnly {
            return;
        }
        if let Some(previous) = self.history.undo(self.snapshot()) {
            self.restore(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.access == InputAccess::ReadOnly {
            return;
        }
        if let Some(next) = self.history.redo(self.snapshot()) {
            self.restore(next, cx);
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Cancelled);
        cx.propagate();
    }

    fn submit(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Submitted);
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        let offset = self.offset_for_position(event.position);
        match event.click_count {
            2 => {
                self.selection =
                    TextSelection::from_range(word_range_at(&self.text, offset), false);
                cx.notify();
            }
            3.. => self.select_all(&SelectAll, window, cx),
            _ if event.modifiers.shift => self.select_to(offset, cx),
            _ => {
                self.is_dragging = true;
                self.move_to(offset, cx);
            }
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_dragging = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_dragging {
            self.select_to(self.offset_for_position(event.position), cx);
        }
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = self.layout.as_ref() else {
            return 0;
        };
        let within_text = point(
            position.x - layout.bounds.left() + layout.scroll_offset,
            position.y - layout.bounds.top(),
        );
        self.text_offset(layout.text.offset_for_position(within_text))
            .min(self.text.len())
    }

    /// The range an IME edit applies to when the platform does not name one.
    fn implicit_edit_range(&self) -> Range<usize> {
        self.marked_range
            .clone()
            .unwrap_or_else(|| self.selection.range())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = utf16_range_to_utf8(&self.text, &range_utf16);
        actual_range.replace(utf8_range_to_utf16(&self.text, &range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: utf8_range_to_utf16(&self.text, &self.selection.range()),
            reversed: self.selection.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| utf8_range_to_utf16(&self.text, range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|range| utf16_range_to_utf8(&self.text, &range))
            .unwrap_or_else(|| self.implicit_edit_range());
        self.marked_range = None;
        self.replace(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.access == InputAccess::ReadOnly {
            return;
        }
        let range = range_utf16
            .map(|range| utf16_range_to_utf8(&self.text, &range))
            .unwrap_or_else(|| self.implicit_edit_range());
        self.replace(range.clone(), new_text, cx);
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        if let Some(selected_utf16) = new_selected_range_utf16 {
            // The platform gives the selection relative to the marked text.
            let selected = utf16_range_to_utf8(new_text, &selected_utf16);
            self.selection = TextSelection::from_range(
                range.start + selected.start..range.start + selected.end,
                false,
            );
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = utf16_range_to_utf8(&self.text, &range_utf16);
        let start = self.display_offset(range.start);
        let end = self.display_offset(range.end);
        // The platform takes one rectangle, so a range that wraps reports its first line,
        // where composition starts.
        let bounds = layout
            .text
            .range_bounds(start..end)
            .first()
            .copied()
            .unwrap_or_else(|| layout.text.caret_bounds(start, Pixels::ZERO));
        let origin = point(
            element_bounds.left() - layout.scroll_offset,
            element_bounds.top(),
        );
        Some(bounds + origin)
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_for_position(position);
        Some(utf8_to_utf16(&self.text, offset))
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let is_focused = self.focus_handle.is_focused(window);

        div()
            .key_context(TEXT_INPUT_CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::move_to_line_start))
            .on_action(cx.listener(Self::move_to_line_end))
            .on_action(cx.listener(Self::move_to_start))
            .on_action(cx.listener(Self::move_to_end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_to_line_start))
            .on_action(cx.listener(Self::select_to_line_end))
            .on_action(cx.listener(Self::select_to_start))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .flex()
            .items_center()
            .w_full()
            .when(self.appearance == Appearance::Boxed, |input| {
                input
                    .map(|input| {
                        if self.is_wrapping {
                            input.min_h(theme.size.control_medium).py(theme.spacing.xs)
                        } else {
                            input.h(theme.size.control_medium)
                        }
                    })
                    .px(theme.spacing.md)
                    .rounded(theme.radius.md)
                    .border(theme.size.border_width)
                    .border_color(theme.colors.border_strong)
                    .bg(theme.colors.control)
                    .relative()
                    .when(is_focused, |input| {
                        input
                            .border_color(theme.colors.accent)
                            .child(focus_outline(&theme, theme.radius.md))
                    })
            })
            .child(TextInputElement::new(cx.entity()))
    }
}

impl TextInput {
    pub fn story(window: &mut Window, cx: &mut App) -> AnyView {
        let theme = Theme::current(window, cx);
        let inputs = [
            cx.new(|cx| TextInput::new(cx).placeholder("Search or jump to…")),
            cx.new(|cx| TextInput::new(cx).with_text("Quarterly planning")),
            cx.new(|cx| TextInput::new(cx).masked().with_text("hunter22")),
            cx.new(|cx| {
                TextInput::new(cx)
                    .plain()
                    .text_size(theme.text.title)
                    .font_weight(FontWeight::BOLD)
                    .placeholder("Untitled")
                    .with_text("Page title")
            }),
            cx.new(|cx| {
                TextInput::new(cx)
                    .plain()
                    .text_size(theme.text.title)
                    .font_weight(FontWeight::BOLD)
                    .wrapping()
                    .with_text(
                        "A long page title that wraps onto the next line instead of scrolling",
                    )
            }),
        ];
        cx.new(|_| TextInputStory { inputs }).into()
    }
}

struct TextInputStory {
    inputs: [Entity<TextInput>; 5],
}

impl Render for TextInputStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.md)
            .children(self.inputs.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(TextInput::story, cx);
    }
}
