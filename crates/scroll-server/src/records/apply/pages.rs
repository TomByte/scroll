//! Pages: the tree and database rows, as records. Their content is `pages::store`'s.

use scroll_core::id::PageId;
use scroll_core::operation::{PageField, Record, RecordId};
use scroll_core::record::Page;
use scroll_core::validation::{
    ValidationError, check_new_parent, check_page_has_no_children, check_page_hosts_no_database,
    check_position, check_row_of,
};

use super::WorkspaceRecords;
use crate::lock;
use crate::records::RecordError;
use crate::records::catch_up::{SnapshotSource, stored_record};

pub(super) const SNAPSHOT: &[SnapshotSource] = &[SnapshotSource {
    sql: "SELECT record FROM page_records WHERE workspace_id = $1",
    read: |row| stored_record(row, Record::Page),
}];

impl WorkspaceRecords<'_> {
    pub(super) async fn create_page(&self, page: &Page) -> Result<(), RecordError> {
        if let Some(Some(_)) = lock(&self.pages).get(&page.id) {
            return Err(ValidationError::RecordAlreadyExists(RecordId::Page(page.id)).into());
        }
        check_position(&page.position)?;
        if let Some(parent) = page.parent {
            self.require_page(parent).await?;
        }
        if let Some(database) = page.row_of {
            self.require_database(database).await?;
        }
        self.buffer.create_page(page.clone());
        lock(&self.pages).insert(page.id, Some(page.clone()));
        Ok(())
    }

    pub(super) async fn set_page_field(
        &self,
        id: PageId,
        field: &PageField,
    ) -> Result<(), RecordError> {
        let mut page = self.require_page(id).await?;
        match field {
            PageField::Parent(Some(parent)) => {
                self.require_page(*parent).await?;
                check_new_parent(id, &self.lineage(*parent).await?)?;
            }
            PageField::Position(position) => check_position(position)?,
            PageField::RowOf(Some(database)) => {
                check_row_of(id, &self.require_database(*database).await?)?;
            }
            _ => {}
        }
        field.apply_to(&mut page);
        self.write_page(&page);
        Ok(())
    }

    /// Drops the page's content too. The page row stays, so its id remains bound to this workspace.
    pub(super) async fn delete_page(&self, id: PageId) -> Result<(), RecordError> {
        self.require_page(id).await?;
        let child_count = self
            .count(
                "SELECT count(*) FROM (SELECT 1 FROM page_records WHERE parent_id = $1 LIMIT 1) AS any_child",
                id.0,
            )
            .await?;
        check_page_has_no_children(id, child_count)?;
        check_page_hosts_no_database(id, self.database_hosted_on(id).await?)?;
        self.execute(
            "UPDATE pages SET record = NULL, parent_id = NULL, row_of = NULL WHERE id = $1",
            &[&id.0],
        )
        .await?;
        crate::pages::drop_content(self.tx, id).await?;
        self.execute("DELETE FROM property_values WHERE page_id = $1", &[&id.0])
            .await?;
        self.execute("DELETE FROM page_tags WHERE page_id = $1", &[&id.0])
            .await?;
        lock(&self.pages).insert(id, None);
        Ok(())
    }
}

impl WorkspaceRecords<'_> {
    /// Held back with the other page writes; readers here go through the cache, and the database sees it before
    /// the next statement.
    fn write_page(&self, page: &Page) {
        self.buffer.change_page(page.clone());
        lock(&self.pages).insert(page.id, Some(page.clone()));
    }

    /// `page` followed by its ancestors, read through the cache: nearby pages share ancestors, so a transaction
    /// moving many reads each once. Stops at a repeat, so even a corrupt cycle cannot loop forever.
    async fn lineage(&self, page: PageId) -> Result<Vec<PageId>, RecordError> {
        let mut lineage = vec![page];
        let mut next = self.known_page(page).await?.and_then(|page| page.parent);
        while let Some(ancestor) = next {
            if lineage.contains(&ancestor) {
                break;
            }
            lineage.push(ancestor);
            next = self
                .known_page(ancestor)
                .await?
                .and_then(|page| page.parent);
        }
        Ok(lineage)
    }
}
