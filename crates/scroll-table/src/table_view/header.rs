//! The header row: the pinned title column's header and one cell per property column. A property
//! header opens its menu on click, reorders by dragging, resizes by its right edge, and is renamed
//! in place.

use gpui::{
    AnyElement, AppContext, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    MouseDownEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, div,
};
use scroll_core::id::PropertyId;
use scroll_core::record::Property;
use scroll_ui::{DragPreview, Icon, IconName, IconSize, ResizeHandle, Theme};

use super::popups::column_trigger;
use super::{EditTarget, TableView};
use crate::sizes::{CELL_GAP, CELL_PADDING, GUTTER_WIDTH, HEADER_HEIGHT, TITLE_COLUMN_WIDTH};
use crate::values::type_icon;

/// What is carried while a column header is dragged.
#[derive(Clone)]
pub(super) struct DraggedColumn {
    property: PropertyId,
    name: SharedString,
}

impl TableView {
    pub(super) fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let cells: Vec<AnyElement> = self
            .columns
            .columns()
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                let property = self.model.property(column.property)?;
                Some(self.render_header_cell(index, property, theme, cx))
            })
            .collect();
        div()
            .flex()
            .flex_none()
            .h(HEADER_HEIGHT)
            .border_b(theme.size.border_width)
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_secondary)
            .text_size(theme.text.small.size)
            .text_color(theme.colors.text_muted)
            .child(div().flex_none().w(GUTTER_WIDTH))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(CELL_GAP)
                    .w(TITLE_COLUMN_WIDTH)
                    .px(CELL_PADDING)
                    .border_r(theme.size.border_width)
                    .border_color(theme.colors.border)
                    .child(Icon::new(IconName::Page).size(IconSize::Small))
                    .child("Name"),
            )
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
    }

    fn render_header_cell(
        &self,
        index: usize,
        property: &Property,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = property.id;
        let width = self.columns.columns()[index].width;
        let dragged = DraggedColumn {
            property: id,
            name: property.name.clone().into(),
        };
        let drop_target = theme.colors.selected;
        let label: AnyElement = match &self.text_edit {
            Some(edit) if edit.target == EditTarget::PropertyName(id) => {
                edit.input.clone().into_any_element()
            }
            _ => div()
                .flex_1()
                .truncate()
                .child(property.name.clone())
                .into_any_element(),
        };
        div()
            .id(ElementId::Name(format!("column-{}", id.0).into()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(CELL_GAP)
            .w(width)
            .h_full()
            .px(CELL_PADDING)
            .border_r(theme.size.border_width)
            .border_color(theme.colors.border)
            .cursor_pointer()
            .hover(|cell| cell.bg(theme.colors.hover))
            .child(Icon::new(type_icon(property.property_type)).size(IconSize::Small))
            .child(label)
            .child(self.trigger(column_trigger(id), cx))
            .on_click(cx.listener(move |table, _: &ClickEvent, window, cx| {
                table.open_column_menu(id, window, cx)
            }))
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| DragPreview::new(dragged.name.clone()))
            })
            .drag_over::<DraggedColumn>(move |style, _, _, _| style.bg(drop_target))
            .on_drop(cx.listener(move |table, dragged: &DraggedColumn, _, cx| {
                table.move_column(dragged.property, id, cx)
            }))
            .child(
                ResizeHandle::new(ElementId::Name(format!("resize-{}", id.0).into())).on_press(
                    cx.listener(move |table, event: &MouseDownEvent, _, cx| {
                        table.start_resize(id, event, cx)
                    }),
                ),
            )
            .into_any_element()
    }
}
