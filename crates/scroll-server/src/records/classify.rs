//! Which transactions only change rows of one database: a hint for delivery (see `realtime::feed`), never for
//! correctness, since every connection still receives every transaction in order.

use scroll_core::id::DatabaseId;
use scroll_core::operation::{FieldChange, Operation, PageField, Record};

use super::RecordError;
use super::apply::{WorkspaceRecords, page_changed_by};

impl WorkspaceRecords<'_> {
    /// The database whose row the operation changes, if it changes a row and leaves it a row of the same
    /// database. Read before the operation applies, so a deleted row still counts.
    pub(super) async fn row_database(
        &self,
        operation: &Operation,
    ) -> Result<Option<DatabaseId>, RecordError> {
        let page = match operation {
            Operation::Create(Record::Page(page)) => return Ok(page.row_of),
            Operation::Set(FieldChange::Page {
                field: PageField::Parent(_) | PageField::RowOf(_),
                ..
            }) => return Ok(None),
            _ => page_changed_by(operation),
        };
        let Some(page) = page else {
            return Ok(None);
        };
        Ok(self.known_page(page).await?.and_then(|page| page.row_of))
    }
}
