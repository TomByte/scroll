//! When an edit can move rows, so the table re-runs the query to put them in place.

use scroll_core::operation::{FieldChange, Operation, PageField, Transaction};
use scroll_core::record::{Database, View};
use scroll_core::view_query::places_rows_by;

/// Whether the transaction changes a value the view sorts or filters on, or a row's position, so
/// rows may need to move.
pub fn changes_row_placement(view: &View, database: &Database, transaction: &Transaction) -> bool {
    transaction
        .operations
        .iter()
        .any(|operation| match operation {
            Operation::Set(FieldChange::PropertyValue { property, .. }) => {
                places_rows_by(view, database, *property)
            }
            Operation::Set(FieldChange::Page {
                field: PageField::Position(_),
                ..
            }) => true,
            _ => false,
        })
}
