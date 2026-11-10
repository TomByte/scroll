//! Property values of database rows.

use scroll_core::id::{PageId, PropertyId};
use scroll_core::record::PropertyValue;
use scroll_core::sync::PropertyValueEntry;
use scroll_core::validation::{ValidationError, check_property_value, find_property};

use super::WorkspaceRecords;
use crate::records::catch_up::{ID_PAIR_BYTES, Item, SnapshotSource};
use crate::records::{RecordError, decode, encode};

pub(super) const SNAPSHOT: &[SnapshotSource] = &[SnapshotSource {
    sql: "SELECT page_id, property_id, value FROM property_values WHERE workspace_id = $1",
    read: |row| {
        let bytes: &[u8] = row.get("value");
        let entry = PropertyValueEntry {
            page: PageId(row.get("page_id")),
            property: PropertyId(row.get("property_id")),
            value: decode(bytes)?,
        };
        Ok((Item::Value(entry), bytes.len() + ID_PAIR_BYTES))
    },
}];

impl WorkspaceRecords<'_> {
    pub(super) async fn set_property_value(
        &self,
        page_id: PageId,
        property_id: PropertyId,
        value: Option<&PropertyValue>,
    ) -> Result<(), RecordError> {
        let page = self.require_page(page_id).await?;
        let database_id = page
            .row_of
            .ok_or(ValidationError::PageIsNotADatabaseRow(page_id))?;
        let database = self.require_database(database_id).await?;
        let property = find_property(&database, property_id)?;
        let Some(value) = value else {
            self.buffer.set_value((page_id, property_id), None);
            return Ok(());
        };
        check_property_value(property, value)?;
        self.buffer
            .set_value((page_id, property_id), Some(encode(value)?));
        Ok(())
    }
}
