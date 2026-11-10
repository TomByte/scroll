//! What a cell shows when it is not being edited.

use gpui::{AnyElement, IntoElement, ParentElement, Pixels, Styled, div};
use scroll_core::record::{Property, PropertyValue};
use scroll_ui::{PillRow, Theme};

use crate::values::{chosen_options, display_text};

/// What drawing a cell needs besides its value: the theme and the width available for content
/// inside the cell's padding.
pub struct CellArea<'a> {
    pub theme: &'a Theme,
    pub width: Pixels,
}

/// The cell's content; `None` for an empty cell. Checkbox cells are drawn by the row, which toggles them.
pub fn cell_content(
    value: Option<&PropertyValue>,
    property: &Property,
    area: &CellArea,
) -> AnyElement {
    let theme = area.theme;
    let text_cell = || {
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_size(theme.text.ui.size)
            .line_height(theme.text.ui.line_height)
    };
    match value {
        None => div().into_any_element(),
        Some(number @ PropertyValue::Number(_)) => text_cell()
            .text_right()
            .child(display_text(number, property))
            .into_any_element(),
        Some(PropertyValue::Url(url)) => text_cell()
            .text_color(theme.colors.link)
            .underline()
            .child(url.clone())
            .into_any_element(),
        Some(options @ (PropertyValue::Select(_) | PropertyValue::MultiSelect(_))) => {
            pills(options, property, area)
        }
        Some(other) => text_cell()
            .child(display_text(other, property))
            .into_any_element(),
    }
}

/// Options in the property's colours: as many as fit whole, then a "+N" count for the rest, so a
/// pill is never cut mid-word at the cell's edge. Options missing from the schema are not shown.
fn pills(value: &PropertyValue, property: &Property, area: &CellArea) -> AnyElement {
    chosen_options(property, Some(value))
        .into_iter()
        .fold(PillRow::new(area.width), |row, index| {
            row.pill(property.options[index].name.clone(), index)
        })
        .into_any_element()
}
