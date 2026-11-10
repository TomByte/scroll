//! How the editor draws: a virtual list of block rows, each with its kind's marker and
//! styling, plus the floating completion list and link prompt.

use gpui::{
    AnyElement, AppContext, Context, ElementId, ElementInputHandler, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, canvas, div, list, prelude::FluentBuilder,
};
use scroll_ui::{
    Checkbox, Clickable, DragPreview, Icon, IconName, IconSize, Scrollbar, TextSize, Theme,
};

use super::block_element::BlockElement;
use super::{
    BLOCK_MARKER_WIDTH, BlockState, CONTENT_MAX_WIDTH, KEY_CONTEXT, NEST_INDENT, PageEditor,
    QUOTE_BAR_WIDTH,
};
use crate::editor::document::BlockKind;
use crate::page_layout::PAGE_MARGIN;

/// Room to scroll the last block up from the window's bottom edge.
const END_OF_PAGE_SPACE: gpui::Pixels = gpui::px(160.0);

impl Render for PageEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        self.frame += 1;
        self.reveal_caret_if_needed(window);
        let editor = cx.entity().downgrade();
        let blocks = list(
            self.list_state.clone(),
            cx.processor(|editor, index: usize, window, cx| editor.render_row(index, window, cx)),
        )
        .size_full();
        let completion = self.render_completion(cx);
        let link_prompt = self.render_link_prompt(&theme, cx);

        div()
            .id("page-editor")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor_text()
            .on_action(when_focused(cx, Self::backspace))
            .on_action(when_focused(cx, Self::delete))
            .on_action(when_focused(cx, Self::delete_word_backward))
            .on_action(when_focused(cx, Self::delete_word_forward))
            .on_action(when_focused(cx, Self::delete_to_line_start))
            .on_action(when_focused(cx, Self::newline))
            .on_action(when_focused(cx, Self::insert_line_break))
            .on_action(when_focused(cx, Self::indent))
            .on_action(when_focused(cx, Self::outdent))
            .on_action(when_focused(cx, Self::toggle_to_do))
            .on_action(when_focused(cx, Self::set_block_kind))
            .on_action(when_focused(cx, Self::select_blocks))
            .on_action(when_focused(cx, Self::move_blocks_up))
            .on_action(when_focused(cx, Self::move_blocks_down))
            .on_action(when_focused(cx, Self::copy))
            .on_action(when_focused(cx, Self::cut))
            .on_action(when_focused(cx, Self::paste))
            .on_action(when_focused(cx, Self::undo))
            .on_action(when_focused(cx, Self::redo))
            .on_action(when_focused(cx, Self::toggle_bold))
            .on_action(when_focused(cx, Self::toggle_italic))
            .on_action(when_focused(cx, Self::toggle_strikethrough))
            .on_action(when_focused(cx, Self::toggle_code))
            .on_action(when_focused(cx, Self::toggle_link))
            .on_action(when_focused(cx, Self::move_left))
            .on_action(when_focused(cx, Self::move_right))
            .on_action(when_focused(cx, Self::move_up))
            .on_action(when_focused(cx, Self::move_down))
            .on_action(when_focused(cx, Self::move_word_left))
            .on_action(when_focused(cx, Self::move_word_right))
            .on_action(when_focused(cx, Self::move_to_line_start))
            .on_action(when_focused(cx, Self::move_to_line_end))
            .on_action(when_focused(cx, Self::move_to_start))
            .on_action(when_focused(cx, Self::move_to_end))
            .on_action(when_focused(cx, Self::select_left))
            .on_action(when_focused(cx, Self::select_right))
            .on_action(when_focused(cx, Self::select_up))
            .on_action(when_focused(cx, Self::select_down))
            .on_action(when_focused(cx, Self::select_word_left))
            .on_action(when_focused(cx, Self::select_word_right))
            .on_action(when_focused(cx, Self::select_to_line_start))
            .on_action(when_focused(cx, Self::select_to_line_end))
            .on_action(when_focused(cx, Self::select_to_start))
            .on_action(when_focused(cx, Self::select_to_end))
            .on_action(when_focused(cx, Self::select_all))
            .on_action(when_focused(cx, Self::show_character_palette))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::mouse_down_below_blocks),
            )
            .on_mouse_move(cx.listener(Self::mouse_drag))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_drag_move(cx.listener(Self::drag_blocks_over))
            .on_drop(cx.listener(Self::drop_blocks))
            .relative()
            .size_full()
            .text_color(theme.colors.text)
            .child(
                canvas(
                    {
                        let editor = editor.clone();
                        move |bounds, _, cx| {
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, _| editor.viewport = Some(bounds));
                            }
                        }
                    },
                    // Typing reaches the editor even when the caret's block is scrolled out of
                    // view (the list does not paint it then), so input is registered here.
                    move |bounds, _, window, cx| {
                        let Some(editor) = editor.upgrade() else {
                            return;
                        };
                        let focus_handle = editor.read(cx).focus_handle().clone();
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, editor),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(blocks)
            .child(Scrollbar::new("page-scrollbar", self.list_state.clone()))
            .children(completion)
            .child(self.popups.clone())
            .children(link_prompt)
    }
}

impl PageEditor {
    fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(window, cx);
        let header_rows = self.header_rows();
        let row = if index < header_rows {
            self.header.clone().map(|header| header.into_any_element())
        } else {
            Some(self.render_block(index - header_rows, &theme, window, cx))
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .when(index + 1 == header_rows + self.blocks.len(), |column| {
                column.pb(END_OF_PAGE_SPACE)
            })
            .child(
                div()
                    .w_full()
                    .max_w(CONTENT_MAX_WIDTH + PAGE_MARGIN * 2.0)
                    .px(PAGE_MARGIN)
                    .children(row),
            )
            .into_any_element()
    }

    fn render_block(
        &self,
        index: usize,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let block = &self.blocks[index];
        let id = block.info.id;
        let indent = NEST_INDENT * block.info.depth as f32;
        let is_block_selected = self.is_block_selected(index);
        let drop_line = self.drop_indicator_for(id);
        let group = SharedString::from(format!("block-{index}"));

        let body = if block.info.kind == BlockKind::Divider {
            div()
                .flex_1()
                .py(theme.spacing.lg)
                .child(
                    div()
                        .w_full()
                        .h(theme.size.border_width)
                        .bg(theme.colors.border_strong),
                )
                .into_any_element()
        } else {
            let is_focused = self.focus_handle.is_focused(window);
            let element =
                BlockElement::new(cx.entity(), id, self.block_paint(index, theme, is_focused));
            decorate_text(block, theme, div().flex_1().min_w_0().child(element))
        };

        div()
            .id(ElementId::Name(group.clone()))
            .group(group.clone())
            .relative()
            .w_full()
            .pl(indent)
            .pt(top_spacing(block.info.kind, theme))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |editor, event, window, cx| {
                    editor.mouse_down(id, event, window, cx)
                }),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .rounded(theme.radius.sm)
                    .when(block.info.kind == BlockKind::Callout, |callout| {
                        callout
                            .py(theme.spacing.lg)
                            .pr(theme.spacing.lg)
                            .pl(theme.spacing.sm)
                            .rounded(theme.radius.lg)
                            .bg(theme.colors.callout_background)
                    })
                    .when(is_block_selected, |row| row.bg(theme.colors.text_selection))
                    .children(self.render_marker(index, theme, window, cx))
                    .child(body),
            )
            // A viewer cannot move blocks, so there is nothing to grab.
            .when(!self.is_read_only(), |row| {
                row.child(self.render_drag_handle(index, group, theme))
            })
            .children(drop_line)
            .into_any_element()
    }

    fn render_marker(
        &self,
        index: usize,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let block = &self.blocks[index];
        let text = text_size(block.info.kind, theme);
        let marker_column = || {
            div()
                .flex()
                .flex_none()
                .justify_center()
                .items_center()
                .w(BLOCK_MARKER_WIDTH)
                .h(text.line_height)
        };
        let id = block.info.id;
        let marker = match block.info.kind {
            BlockKind::BulletedListItem => marker_column()
                .text_size(text.size)
                .child(bullet_for_depth(block.info.depth))
                .into_any_element(),
            BlockKind::NumberedListItem => marker_column()
                .text_size(text.size)
                .child(format!("{}.", self.list_numbers[index].unwrap_or(1)))
                .into_any_element(),
            BlockKind::ToDo => marker_column()
                .child(
                    Checkbox::new(ElementId::Name(format!("to-do-{index}").into()))
                        .when(block.info.is_checked, Checkbox::checked)
                        .on_toggle(cx.listener(move |editor, is_checked: &bool, _, cx| {
                            editor.set_to_do_checked(id, *is_checked, cx)
                        })),
                )
                .into_any_element(),
            BlockKind::Callout => {
                let icon = marker_column()
                    .id(ElementId::Name(format!("callout-icon-{index}").into()))
                    .text_size(text.size)
                    .rounded(theme.radius.sm)
                    .child(self.callout_icon(id));
                Clickable::new(theme.radius.sm)
                    .on_click(
                        cx.listener(move |editor, event: &gpui::ClickEvent, window, cx| {
                            editor.open_callout_picker(id, event, window, cx)
                        }),
                    )
                    .apply(icon, window, cx)
                    .into_any_element()
            }
            _ => return None,
        };
        Some(marker)
    }

    /// The handle shown left of a block on hover; dragging it moves the block (or the
    /// selected blocks, if it is one of them).
    fn render_drag_handle(
        &self,
        index: usize,
        group: SharedString,
        theme: &Theme,
    ) -> impl IntoElement {
        let block = &self.blocks[index];
        let dragged = self.blocks_to_drag(index);
        let indent = NEST_INDENT * block.info.depth as f32;
        let line_height = text_size(block.info.kind, theme).line_height;
        div()
            .id(ElementId::Name(format!("drag-{index}").into()))
            .absolute()
            .left(indent - BLOCK_MARKER_WIDTH)
            .top(top_spacing(block.info.kind, theme))
            .h(line_height)
            .w(drag_handle_width(theme))
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme.radius.sm)
            .cursor_grab()
            .invisible()
            .group_hover(group, |handle| handle.visible())
            .hover(|handle| handle.bg(theme.colors.hover))
            .child(
                Icon::new(IconName::Grip)
                    .size(IconSize::Small)
                    .color(theme.colors.text_placeholder),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| DragPreview::new(dragged.label.clone()).icon(IconName::Grip))
            })
    }

    /// Scrolls so the caret's line is inside the viewport. Uses the caret block's layout from
    /// the previous frame; a block that was not on screen is first scrolled to the top, and
    /// the next frame fine-tunes once it has a layout.
    fn reveal_caret_if_needed(&mut self, window: &mut Window) {
        if !std::mem::take(&mut self.needs_caret_revealed) {
            return;
        }
        let Some(index) = self.block_index(self.selection.block) else {
            return;
        };
        let caret_block = &self.blocks[index];
        let current_layout = caret_block
            .layout
            .as_ref()
            .filter(|_| caret_block.layout_frame + 1 == self.frame);
        let (Some(viewport), Some(layout)) = (self.viewport, current_layout) else {
            self.list_state.scroll_to(gpui::ListOffset {
                item_ix: index + self.header_rows(),
                offset_in_item: gpui::px(0.0),
            });
            self.needs_caret_revealed = true;
            window.request_animation_frame();
            return;
        };
        let caret_top = layout.caret_origin(self.display_selection().head).y;
        let caret_bottom = caret_top + layout.line_height();
        let margin = layout.line_height() * 2.0;
        if caret_top < viewport.top() + margin {
            self.list_state
                .scroll_by(caret_top - viewport.top() - margin);
        } else if caret_bottom > viewport.bottom() - margin {
            self.list_state
                .scroll_by(caret_bottom - viewport.bottom() + margin);
        }
    }
}

/// Wraps an action handler so it runs only while the editor itself has focus. Views rendered
/// inside the editor (the page title, the link prompt) keep their own keystrokes: an action
/// they do not handle bubbles up here and is passed on instead of editing the page.
fn when_focused<A: gpui::Action>(
    cx: &mut Context<PageEditor>,
    handler: fn(&mut PageEditor, &A, &mut Window, &mut Context<PageEditor>),
) -> impl Fn(&A, &mut Window, &mut gpui::App) + 'static {
    cx.listener(move |editor, action, window, cx| {
        if editor.focus_handle.is_focused(window) {
            handler(editor, action, window, cx);
        } else {
            cx.propagate();
        }
    })
}

/// Kind-specific framing around a block's text: quote bar, callout and code backgrounds.
fn decorate_text(block: &BlockState, theme: &Theme, text: gpui::Div) -> AnyElement {
    match block.info.kind {
        BlockKind::Quote => div()
            .flex_1()
            .min_w_0()
            .border_l(QUOTE_BAR_WIDTH)
            .border_color(theme.colors.text)
            .pl(theme.spacing.lg)
            .child(text)
            .into_any_element(),
        BlockKind::Code => div()
            .flex_1()
            .min_w_0()
            .p(theme.spacing.lg)
            .rounded(theme.radius.md)
            .bg(theme.colors.code_background)
            .child(text)
            .into_any_element(),
        BlockKind::Callout => div().flex_1().min_w_0().child(text).into_any_element(),
        _ => text.into_any_element(),
    }
}

pub(super) fn text_size(kind: BlockKind, theme: &Theme) -> TextSize {
    match kind {
        BlockKind::Heading1 => theme.text.heading_1,
        BlockKind::Heading2 => theme.text.heading_2,
        BlockKind::Heading3 => theme.text.heading_3,
        BlockKind::Code => theme.text.code_block,
        _ => theme.text.body,
    }
}

fn top_spacing(kind: BlockKind, theme: &Theme) -> gpui::Pixels {
    match kind {
        BlockKind::Heading1 => theme.spacing.xxl,
        BlockKind::Heading2 => theme.spacing.xl,
        BlockKind::Heading3 => theme.spacing.lg,
        BlockKind::Code | BlockKind::Callout | BlockKind::Quote => theme.spacing.sm,
        _ => theme.spacing.xxs,
    }
}

pub(super) fn is_heading(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Heading1 | BlockKind::Heading2 | BlockKind::Heading3
    )
}

fn bullet_for_depth(depth: usize) -> &'static str {
    ["•", "◦", "▪"][depth % 3]
}

/// The handle fills the marker column, less a gap before the text.
fn drag_handle_width(theme: &Theme) -> gpui::Pixels {
    BLOCK_MARKER_WIDTH - theme.spacing.xs
}
