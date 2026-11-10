//! Column order and widths for the scrolling part of the table (every column after the pinned title).
//! Order is the view's `visible_columns`; widths are the view's `column_widths`, saved when a
//! resize ends.

use std::collections::HashMap;

use gpui::{Pixels, px};
use scroll_core::id::PropertyId;
use scroll_core::record::ColumnWidth;

use crate::sizes::{DEFAULT_COLUMN_WIDTH, MAXIMUM_COLUMN_WIDTH, MINIMUM_COLUMN_WIDTH};

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub property: PropertyId,
    pub width: Pixels,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnLayout {
    columns: Vec<Column>,
}

impl ColumnLayout {
    /// Columns in `order`, keeping the widths of columns this layout already had.
    pub fn arranged(&self, order: &[PropertyId]) -> ColumnLayout {
        let widths: HashMap<PropertyId, Pixels> = self
            .columns
            .iter()
            .map(|column| (column.property, column.width))
            .collect();
        ColumnLayout {
            columns: order
                .iter()
                .map(|property| Column {
                    property: *property,
                    width: widths
                        .get(property)
                        .copied()
                        .unwrap_or(DEFAULT_COLUMN_WIDTH),
                })
                .collect(),
        }
    }

    /// The same columns with the widths a view saved.
    pub fn with_saved_widths(&self, saved: &[ColumnWidth]) -> ColumnLayout {
        let mut layout = self.clone();
        for column in &mut layout.columns {
            if let Some(width) = saved.iter().find(|width| width.property == column.property) {
                column.width = px(width.width as f32).max(MINIMUM_COLUMN_WIDTH);
            }
        }
        layout
    }

    /// The widths to save with the view: every column not at the default width, in whole points.
    pub fn widths_to_save(&self) -> Vec<ColumnWidth> {
        self.columns
            .iter()
            .filter(|column| column.width != DEFAULT_COLUMN_WIDTH)
            .map(|column| ColumnWidth {
                property: column.property,
                width: f32::from(column.width).round() as u32,
            })
            .collect()
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn order(&self) -> Vec<PropertyId> {
        self.columns.iter().map(|column| column.property).collect()
    }

    pub fn total_width(&self) -> Pixels {
        self.columns
            .iter()
            .fold(px(0.0), |total, column| total + column.width)
    }

    /// Left edge of the column at `index`, measured from the first scrolling column.
    pub fn left_of(&self, index: usize) -> Pixels {
        self.columns
            .iter()
            .take(index)
            .fold(px(0.0), |total, column| total + column.width)
    }

    pub fn resized(&self, property: PropertyId, width: Pixels) -> ColumnLayout {
        let mut layout = self.clone();
        if let Some(column) = layout
            .columns
            .iter_mut()
            .find(|column| column.property == property)
        {
            column.width = width.clamp(MINIMUM_COLUMN_WIDTH, MAXIMUM_COLUMN_WIDTH);
        }
        layout
    }

    /// The order after moving `property` to where `target` is now.
    pub fn order_after_move(&self, property: PropertyId, target: PropertyId) -> Vec<PropertyId> {
        let mut order = self.order();
        let (Some(from), Some(to)) = (
            order.iter().position(|candidate| *candidate == property),
            order.iter().position(|candidate| *candidate == target),
        ) else {
            return order;
        };
        let moved = order.remove(from);
        order.insert(to, moved);
        order
    }

    /// How far to scroll horizontally so the column at `index` is fully visible, given the current
    /// offset and the width of the scrolling area.
    pub fn offset_revealing(&self, index: usize, offset: Pixels, viewport_width: Pixels) -> Pixels {
        let Some(column) = self.columns.get(index) else {
            return offset;
        };
        let left = self.left_of(index);
        let right = left + column.width;
        if left < offset {
            return left;
        }
        if right > offset + viewport_width {
            return (right - viewport_width).max(px(0.0)).min(left);
        }
        offset
    }

    /// The largest useful horizontal offset for a scrolling area of `viewport_width`.
    pub fn max_offset(&self, viewport_width: Pixels) -> Pixels {
        (self.total_width() - viewport_width).max(px(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(count: usize) -> (ColumnLayout, Vec<PropertyId>) {
        let ids: Vec<PropertyId> = (0..count).map(|_| PropertyId::generate()).collect();
        (ColumnLayout::default().arranged(&ids), ids)
    }

    #[test]
    fn widths_survive_rearranging_and_new_columns_get_the_default() {
        let (layout, ids) = layout(2);
        let resized = layout.resized(ids[0], px(300.0));
        let added = PropertyId::generate();
        let rearranged = resized.arranged(&[added, ids[0]]);
        assert_eq!(
            rearranged.columns(),
            [
                Column {
                    property: added,
                    width: DEFAULT_COLUMN_WIDTH
                },
                Column {
                    property: ids[0],
                    width: px(300.0)
                },
            ]
        );
        assert_eq!(rearranged.total_width(), DEFAULT_COLUMN_WIDTH + px(300.0));
        assert_eq!(rearranged.left_of(1), DEFAULT_COLUMN_WIDTH);
    }

    #[test]
    fn columns_cannot_shrink_below_the_minimum() {
        let (layout, ids) = layout(1);
        assert_eq!(
            layout.resized(ids[0], px(10.0)).columns()[0].width,
            MINIMUM_COLUMN_WIDTH
        );
    }

    #[test]
    fn columns_cannot_grow_past_the_widest_the_store_accepts() {
        let (layout, ids) = layout(1);
        let resized = layout.resized(ids[0], px(1_000_000.0));
        assert_eq!(resized.columns()[0].width, MAXIMUM_COLUMN_WIDTH);
        let saved = resized.widths_to_save();
        assert_eq!(
            saved[0].width,
            scroll_core::validation::MAXIMUM_COLUMN_WIDTH
        );
    }

    #[test]
    fn moving_a_column_takes_the_target_place() {
        let (layout, ids) = layout(4);
        assert_eq!(
            layout.order_after_move(ids[0], ids[2]),
            vec![ids[1], ids[2], ids[0], ids[3]]
        );
        assert_eq!(
            layout.order_after_move(ids[3], ids[1]),
            vec![ids[0], ids[3], ids[1], ids[2]]
        );
        assert_eq!(layout.order_after_move(ids[1], PropertyId::generate()), ids);
    }

    #[test]
    fn scrolling_reveals_a_column_with_the_least_movement() {
        let (layout, _) = layout(5);
        let width = DEFAULT_COLUMN_WIDTH;
        let viewport = width * 2.0;
        assert_eq!(layout.offset_revealing(0, px(0.0), viewport), px(0.0));
        assert_eq!(layout.offset_revealing(3, px(0.0), viewport), width * 2.0);
        assert_eq!(layout.offset_revealing(1, width * 3.0, viewport), width);
        assert_eq!(
            layout.offset_revealing(2, width * 2.0, viewport),
            width * 2.0
        );
        assert_eq!(layout.max_offset(viewport), width * 3.0);
        assert_eq!(layout.max_offset(width * 9.0), px(0.0));
    }

    #[test]
    fn saved_widths_apply_and_only_changed_widths_are_saved() {
        let (layout, ids) = layout(3);
        let saved = [ColumnWidth {
            property: ids[1],
            width: 250,
        }];
        let restored = layout.with_saved_widths(&saved);
        assert_eq!(restored.columns()[1].width, px(250.0));
        assert_eq!(restored.columns()[0].width, DEFAULT_COLUMN_WIDTH);
        assert_eq!(restored.widths_to_save(), saved);
        let tiny = [ColumnWidth {
            property: ids[0],
            width: 3,
        }];
        assert_eq!(
            layout.with_saved_widths(&tiny).columns()[0].width,
            MINIMUM_COLUMN_WIDTH
        );
    }
}
