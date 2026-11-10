//! The table's own sizes: the grid of rows and property columns.

use gpui::{Pixels, px};

pub(crate) const ROW_HEIGHT: Pixels = px(28.0);
pub(crate) const HEADER_HEIGHT: Pixels = px(28.0);
/// Of a newly added property column.
pub(crate) const DEFAULT_COLUMN_WIDTH: Pixels = px(180.0);
/// The narrowest a property column can be resized to.
pub(crate) const MINIMUM_COLUMN_WIDTH: Pixels = px(64.0);
/// The widest a property column can be resized to: the widest the store accepts.
pub(crate) const MAXIMUM_COLUMN_WIDTH: Pixels =
    px(scroll_core::validation::MAXIMUM_COLUMN_WIDTH as f32);
/// Of the pinned column holding each row's page title.
pub(crate) const TITLE_COLUMN_WIDTH: Pixels = px(260.0);
/// Of the column left of the title holding each row's selection checkbox.
pub(crate) const GUTTER_WIDTH: Pixels = px(40.0);
/// Between a cell's (or header cell's) edges and its content.
pub(crate) const CELL_PADDING: Pixels = px(8.0);
/// Between the pieces of a cell: an icon, its text, pills.
pub(crate) const CELL_GAP: Pixels = px(4.0);
/// Of the outline around the selected cell.
pub(crate) const SELECTION_OUTLINE_WIDTH: Pixels = px(2.0);
/// The faint bar standing in for the title of a row still loading.
pub(crate) const PLACEHOLDER_BAR_WIDTH: Pixels = px(130.0);
pub(crate) const PLACEHOLDER_BAR_HEIGHT: Pixels = px(6.0);
pub(crate) const PLACEHOLDER_BAR_RADIUS: Pixels = px(4.0);
