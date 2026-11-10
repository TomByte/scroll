//! The database table view: one `View` of a database, shown as a virtualised, editable grid.
//!
//! Call `scroll_ui::init` and then `init` once at startup, and serve `scroll_ui::Assets` as the
//! application's assets (the table's icons are the kit's).

mod actions;
mod cell_content;
mod columns;
mod conditions;
mod date_picker;
mod filter_editor;
mod grid_text;
mod properties_panel;
mod selection;
mod sizes;
mod sort_editor;
mod sort_keys;
mod table_model;
mod table_view;
mod transactions;
pub mod values;

pub use actions::{DeleteRows, OpenRow, init};
pub use table_view::{TableAccess, TableEvent, TableView};

/// The key context the table's bindings apply in.
pub const KEY_CONTEXT: &str = "TableView";
