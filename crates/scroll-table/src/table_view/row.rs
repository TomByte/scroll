//! One row: the selection checkbox, the pinned title cell, and the property cells that scroll
//! horizontally. Only rows in view are built, by the virtualised list.

use std::ops::Range;

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Pixels, SharedString, Stateful, Styled, Window, div,
    prelude::FluentBuilder,
};
use scroll_core::record::{Property, PropertyType, PropertyValue};
use scroll_ui::{Button, ButtonSize, ButtonVariant, Checkbox, ErrorBubble, Theme};

use super::{Command, EditTarget, TableView};
use crate::TableEvent;
use crate::cell_content::{CellArea, cell_content};
use crate::selection::CellPosition;
use crate::sizes::{
    CELL_GAP, CELL_PADDING, GUTTER_WIDTH, PLACEHOLDER_BAR_HEIGHT, PLACEHOLDER_BAR_RADIUS,
    PLACEHOLDER_BAR_WIDTH, ROW_HEIGHT, SELECTION_OUTLINE_WIDTH, TITLE_COLUMN_WIDTH,
};

/// What every row drawn in one frame shares.
struct RowFrame<'a> {
    theme: &'a Theme,
    /// The visible properties, in column order.
    properties: &'a [Property],
}

impl TableView {
    pub(super) fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = Theme::current(window, cx);
        // Cloned once per frame for the rows in view, not once per cell.
        let properties: Vec<Property> = self
            .columns
            .columns()
            .iter()
            .filter_map(|column| self.model.property(column.property).cloned())
            .collect();
        self.ensure_rows_loaded(range.clone(), cx);
        let frame = RowFrame {
            theme: &theme,
            properties: &properties,
        };
        range.map(|row| self.render_row(row, &frame, cx)).collect()
    }

    fn render_row(&self, row: usize, frame: &RowFrame, cx: &mut Context<Self>) -> AnyElement {
        let theme = frame.theme;
        let Some(table_row) = self.model.row(row) else {
            return placeholder_row(row, theme);
        };
        let page = table_row.page.id;
        let is_checked = self.selected_rows.contains(&page);
        let group = SharedString::from(format!("row-{row}"));
        let cells: Vec<AnyElement> = frame
            .properties
            .iter()
            .enumerate()
            .map(|(index, property)| {
                let position = CellPosition {
                    row,
                    column: index + 1,
                };
                self.render_property_cell(position, property, frame, cx)
            })
            .collect();

        div()
            .id(ElementId::Name(format!("row-{row}").into()))
            .group(group.clone())
            .flex()
            .w_full()
            .h(ROW_HEIGHT)
            .border_b(theme.size.border_width)
            .border_color(theme.colors.border)
            .when(is_checked, |row| row.bg(theme.colors.selected))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(GUTTER_WIDTH)
                    .when(!is_checked, |gutter| {
                        gutter
                            .opacity(0.0)
                            .group_hover(group.clone(), |gutter| gutter.opacity(1.0))
                    })
                    .when(self.can_edit(), |gutter| {
                        gutter.child(
                            Checkbox::new(ElementId::Name(format!("check-{row}").into()))
                                .when(is_checked, Checkbox::checked)
                                .on_toggle(cx.listener(move |table, _: &bool, window, cx| {
                                    table.perform(Command::ToggleRow(page), window, cx)
                                })),
                        )
                    }),
            )
            .child(self.render_title_cell(row, group, theme, cx))
            .child(
                div().flex_1().h_full().overflow_hidden().relative().child(
                    div()
                        .absolute()
                        .top_0()
                        .left(-self.horizontal_offset)
                        .h_full()
                        .flex()
                        .children(cells),
                ),
            )
            .into_any_element()
    }

    fn render_title_cell(
        &self,
        row: usize,
        group: SharedString,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(page) = self.model.row(row).map(|row| &row.page) else {
            return div().into_any_element();
        };
        let page_id = page.id;
        let position = CellPosition { row, column: 0 };
        let content: AnyElement = match &self.text_edit {
            Some(edit) if edit.target == EditTarget::Title(page_id) => {
                edit.input.clone().into_any_element()
            }
            _ if page.title.is_empty() => div()
                .flex_1()
                .text_color(theme.colors.text_placeholder)
                .child("Untitled")
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(page.title.clone())
                .into_any_element(),
        };
        self.cell_frame(position, TITLE_COLUMN_WIDTH, theme, cx)
            .child(content)
            .child(
                div()
                    .flex_none()
                    .opacity(0.0)
                    .group_hover(group, |button| button.opacity(1.0))
                    .child(
                        Button::new(ElementId::Name(format!("open-{row}").into()), "Open")
                            .variant(ButtonVariant::Secondary)
                            .size(ButtonSize::Small)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(TableEvent::OpenPage(page_id))
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_property_cell(
        &self,
        position: CellPosition,
        property: &Property,
        frame: &RowFrame,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = frame.theme;
        let width = self.columns.columns()[position.column - 1].width;
        let Some(page) = self.model.row_id(position.row) else {
            return div().into_any_element();
        };
        let value = self.model.value(position.row, property);
        let content: AnyElement = match (&self.text_edit, property.property_type) {
            (Some(edit), _) if edit.target.is_value_of(page, property.id) => {
                edit.input.clone().into_any_element()
            }
            (_, PropertyType::Checkbox) => {
                let is_checked = value == Some(&PropertyValue::Checkbox(true));
                let property_id = property.id;
                Checkbox::new(ElementId::Name(
                    format!("cell-check-{}-{}", position.row, position.column).into(),
                ))
                .when(is_checked, Checkbox::checked)
                .on_toggle(cx.listener(move |table, _: &bool, window, cx| {
                    let command = Command::ToggleCheckbox {
                        page,
                        property: property_id,
                    };
                    table.perform(command, window, cx)
                }))
                .into_any_element()
            }
            _ => {
                let area = CellArea {
                    theme,
                    width: width - CELL_PADDING * 2.0,
                };
                cell_content(value, property, &area)
            }
        };
        self.cell_frame(position, width, theme, cx)
            .child(content)
            .into_any_element()
    }

    /// The box every cell shares: size, borders, selection outline and mouse handling.
    fn cell_frame(
        &self,
        position: CellPosition,
        width: Pixels,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui::Div> {
        let is_head = self.is_selection_shown && self.selection == position;
        let is_in_block = self.is_selection_shown
            && !self.selected_range().is_single()
            && self.selected_range().contains(position);
        div()
            .id(ElementId::Name(
                format!("cell-{}-{}", position.row, position.column).into(),
            ))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(CELL_GAP)
            .w(width)
            .h_full()
            .px(CELL_PADDING)
            .overflow_hidden()
            .border_r(theme.size.border_width)
            .border_color(theme.colors.border)
            .when(is_in_block, |cell| cell.bg(theme.colors.text_selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    // The press is the cell's: on the body behind, it would deselect.
                    cx.stop_propagation();
                    table.press_cell(position, event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
                if table.is_drag_selecting
                    && event.pressed_button == Some(MouseButton::Left)
                    && table.selection != position
                {
                    table.extend_to(position, cx);
                }
            }))
            .when(is_head, |cell| {
                cell.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .border(SELECTION_OUTLINE_WIDTH)
                        .border_color(theme.colors.accent)
                        .rounded(theme.radius.sm),
                )
                .children(self.edit_error.clone().map(ErrorBubble::new))
            })
    }

    /// Selects the cell; a second click or a double click edits it, and Cmd+click opens a URL.
    fn press_cell(
        &mut self,
        position: CellPosition,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text_edit.is_some() && !self.commit_text_edit(window, cx) {
            return;
        }
        window.focus(&self.focus_handle);
        if event.modifiers.shift {
            self.extend_to(position, cx);
            return;
        }
        let was_selected = self.selection == position && self.selected_range().is_single();
        self.select(position, cx);
        self.is_drag_selecting = true;
        let property = self.property_at(position.column).cloned();
        if event.modifiers.platform
            && let Some(property) = &property
            && let Some(PropertyValue::Url(url)) = self.model.value(position.row, property)
        {
            cx.open_url(url);
            return;
        }
        let is_checkbox = property
            .as_ref()
            .is_some_and(|property| property.property_type == PropertyType::Checkbox);
        if !is_checkbox && (event.click_count >= 2 || was_selected) {
            self.perform(Command::StartEditing, window, cx);
        }
    }
}

/// A row whose contents are still loading: its height and a faint bar where the title goes, so
/// scrolling quickly through unloaded rows stays smooth and readable.
fn placeholder_row(row: usize, theme: &Theme) -> AnyElement {
    div()
        .id(ElementId::Name(format!("row-{row}").into()))
        .flex()
        .items_center()
        .w_full()
        .h(ROW_HEIGHT)
        .border_b(theme.size.border_width)
        .border_color(theme.colors.border)
        .pl(GUTTER_WIDTH + CELL_PADDING)
        .child(
            div()
                .w(PLACEHOLDER_BAR_WIDTH)
                .h(PLACEHOLDER_BAR_HEIGHT)
                .rounded(PLACEHOLDER_BAR_RADIUS)
                .bg(theme.colors.code_background),
        )
        .into_any_element()
}
