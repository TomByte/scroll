//! The table's frame: toolbar, header, virtualised rows, footer and the open popover.

use gpui::{
    App, Context, DispatchPhase, InteractiveElement, IntoElement, ListHorizontalSizingBehavior,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render, Styled, WeakEntity,
    Window, canvas, div, prelude::FluentBuilder, px, uniform_list,
};
use scroll_ui::{
    Button, ButtonSize, ButtonVariant, EmptyState, IconName, ScrollPosition, ScrollSource,
    Scrollbar, Theme,
};

use super::popups::{FILTER_TRIGGER, PROPERTIES_TRIGGER, SORT_TRIGGER};
use super::{Command, TableView};
use crate::KEY_CONTEXT;
use crate::actions::on_actions;
use crate::grid_text::row_count_label;
use crate::sizes::{GUTTER_WIDTH, TITLE_COLUMN_WIDTH};

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        if std::mem::take(&mut self.wants_focus) {
            window.focus(&self.focus_handle);
        }

        let table = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle);
        on_actions(table, cx)
            .on_key_down(cx.listener(Self::type_to_edit))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.font.ui.clone())
            .text_size(theme.text.ui.size)
            .line_height(theme.text.ui.line_height)
            .child(self.render_toolbar(&theme, cx))
            .child(self.render_header(&theme, cx))
            .child(self.render_body(cx))
            .child(self.render_footer(&theme, cx))
            .child(self.popups.clone())
            .when(self.pointer_drag.is_some(), |table| {
                table.child(self.pointer_drag_tracker(cx))
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|table, _, _, _| table.is_drag_selecting = false),
            )
    }
}

impl TableView {
    fn render_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.model.view();
        let selected = self.selected_rows.len();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.spacing.xs)
            .px(theme.spacing.lg)
            .py(theme.spacing.sm)
            .border_b(theme.size.border_width)
            .border_color(theme.colors.border)
            .child(div().flex_1())
            .when(selected > 0, |toolbar| {
                toolbar.child(
                    Button::new("delete-rows", format!("Delete {selected}"))
                        .icon(IconName::Trash)
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Small)
                        .on_click(cx.listener(|table, _, window, cx| {
                            table.perform(Command::DeleteRows, window, cx)
                        })),
                )
            })
            .child(
                div()
                    .relative()
                    .child(self.trigger(FILTER_TRIGGER, cx))
                    .child(
                        Button::new("filter", "Filter")
                            .badge(view.filter.len())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Small)
                            .on_click(cx.listener(|table, _, window, cx| {
                                table.open_filter_editor(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .child(self.trigger(SORT_TRIGGER, cx))
                    .child(
                        Button::new("sort", "Sort")
                            .badge(view.sort.len())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Small)
                            .on_click(cx.listener(|table, _, window, cx| {
                                table.open_sort_editor(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .child(self.trigger(PROPERTIES_TRIGGER, cx))
                    .child(
                        Button::new("properties", "Properties")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Small)
                            .on_click(cx.listener(|table, _, window, cx| {
                                table.open_properties_panel(window, cx)
                            })),
                    ),
            )
            .when(self.can_edit(), |toolbar| {
                toolbar.child(
                    Button::new("new-row", "New")
                        .icon(IconName::Plus)
                        .variant(ButtonVariant::Primary)
                        .size(ButtonSize::Small)
                        .on_click(cx.listener(|table, _, window, cx| {
                            table.perform(Command::AddRow, window, cx)
                        })),
                )
            })
    }

    fn render_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = uniform_list(
            "rows",
            self.model.row_count(),
            cx.processor(|table, range, window, cx| table.render_rows(range, window, cx)),
        )
        .size_full()
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::FitList)
        .track_scroll(self.scroll.clone());
        // Horizontal swipes move the property columns (see `scroll_horizontally`), not the rows.
        rows.style().restrict_scroll_to_axis = Some(true);

        let table = cx.entity().downgrade();
        // Measures the body as laid out this frame, so scroll limits follow container resizes.
        let measure = canvas(
            move |bounds, _, cx| {
                if let Some(table) = table.upgrade() {
                    table.update(cx, |table, cx| table.set_body_width(bounds.size.width, cx));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .on_scroll_wheel(cx.listener(Self::scroll_horizontally))
            .child(measure)
            .child(rows)
            .child(Scrollbar::new("rows-scrollbar", self.scroll.clone()))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(GUTTER_WIDTH + TITLE_COLUMN_WIDTH)
                    .right_0()
                    .child(
                        Scrollbar::new("columns-scrollbar", ColumnScroll(cx.entity().downgrade()))
                            .horizontal(),
                    ),
            )
            .when(self.model.row_count() == 0, |body| {
                let note = match self.is_loading {
                    true => "Loading…",
                    false => "No rows in this view.",
                };
                body.child(div().absolute().size_full().child(EmptyState::new(note)))
            })
    }

    fn render_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.spacing.md)
            .px(theme.spacing.lg)
            .py(theme.spacing.xs)
            .border_t(theme.size.border_width)
            .border_color(theme.colors.border)
            .text_size(theme.text.small.size)
            .line_height(theme.text.small.line_height)
            .text_color(theme.colors.text_muted)
            .when(self.can_edit(), |footer| {
                footer.child(
                    Button::new("add-row", "New row")
                        .icon(IconName::Plus)
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Small)
                        .on_click(cx.listener(|table, _, window, cx| {
                            table.perform(Command::AddRow, window, cx)
                        })),
                )
            })
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(theme.colors.danger)
                    .children(self.error.clone()),
            )
            .child(row_count_label(self.model.row_count()))
    }

    /// During a pointer drag, follows the pointer anywhere in the window until release.
    fn pointer_drag_tracker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let table = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let on_move = table.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && let Some(table) = on_move.upgrade()
                    {
                        table.update(cx, |table, cx| table.drag_to(event.position.x, cx));
                    }
                });
                let on_up = table.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && let Some(table) = on_up.upgrade()
                    {
                        table.update(cx, |table, cx| table.end_drag(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

/// The property columns' horizontal scroll, for the kit's scrollbar.
struct ColumnScroll(WeakEntity<TableView>);

impl ScrollSource for ColumnScroll {
    fn position(&self, cx: &App) -> ScrollPosition {
        let Some(table) = self.0.upgrade() else {
            return ScrollPosition {
                offset: px(0.0),
                max_offset: px(0.0),
                viewport: px(0.0),
            };
        };
        let table = table.read(cx);
        ScrollPosition {
            offset: table.horizontal_offset,
            max_offset: table.columns.max_offset(table.scrolling_width),
            viewport: table.scrolling_width,
        }
    }

    fn scroll_to(&self, offset: Pixels, cx: &mut App) {
        if let Some(table) = self.0.upgrade() {
            table.update(cx, |table, cx| {
                table.horizontal_offset = offset;
                table.clamp_horizontal_offset();
                cx.notify();
            });
        }
    }
}
