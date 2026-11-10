//! Databases: their schemas, and the checks before one goes.

use scroll_core::id::DatabaseId;
use scroll_core::operation::{DatabaseField, Record, RecordId};
use scroll_core::record::Database;
use scroll_core::validation::{
    ValidationError, check_database, check_database_has_no_rows, check_database_has_no_views,
    check_host_page_is_free, check_property, find_property,
};

use super::WorkspaceRecords;
use crate::lock;
use crate::records::catch_up::{SnapshotSource, stored_record};
use crate::records::{RecordError, encode};

pub(super) const SNAPSHOT: &[SnapshotSource] = &[SnapshotSource {
    sql: "SELECT record FROM databases WHERE workspace_id = $1",
    read: |row| stored_record(row, Record::Database),
}];

impl WorkspaceRecords<'_> {
    pub(super) async fn create_database(&self, database: &Database) -> Result<(), RecordError> {
        let id_is_taken = self
            .exists(
                "SELECT EXISTS (SELECT 1 FROM databases WHERE id = $1)",
                database.id.0,
            )
            .await?;
        if id_is_taken {
            return Err(
                ValidationError::RecordAlreadyExists(RecordId::Database(database.id)).into(),
            );
        }
        self.require_page(database.host_page).await?;
        check_host_page_is_free(
            database.host_page,
            self.database_hosted_on(database.host_page).await?,
        )?;
        check_database(database)?;
        self.execute(
            "INSERT INTO databases (id, workspace_id, host_page, record) VALUES ($1, $2, $3, $4)",
            &[
                &database.id.0,
                &self.workspace_id.0,
                &database.host_page.0,
                &encode(database)?,
            ],
        )
        .await?;
        lock(&self.databases).insert(database.id, Some(database.clone()));
        Ok(())
    }

    pub(super) async fn set_database_field(
        &self,
        id: DatabaseId,
        field: &DatabaseField,
    ) -> Result<(), RecordError> {
        let mut database = self.require_database(id).await?;
        match field {
            DatabaseField::Property(property) => check_property(property)?,
            DatabaseField::RemovedProperty(property) => {
                find_property(&database, *property)?;
            }
        }
        field.apply_to(&mut database);
        self.execute(
            "UPDATE databases SET record = $2 WHERE id = $1",
            &[&id.0, &encode(&database)?],
        )
        .await?;
        lock(&self.databases).insert(id, Some(database));
        Ok(())
    }

    pub(super) async fn delete_database(&self, id: DatabaseId) -> Result<(), RecordError> {
        self.require_database(id).await?;
        let row_count = self
            .count(
                "SELECT count(*) FROM (SELECT 1 FROM page_records WHERE row_of = $1 LIMIT 1) AS any_row",
                id.0,
            )
            .await?;
        check_database_has_no_rows(id, row_count)?;
        let view_count = self
            .count(
                "SELECT count(*) FROM (SELECT 1 FROM views WHERE database_id = $1 LIMIT 1) AS any_view",
                id.0,
            )
            .await?;
        check_database_has_no_views(id, view_count)?;
        self.execute("DELETE FROM databases WHERE id = $1", &[&id.0])
            .await?;
        lock(&self.databases).insert(id, None);
        Ok(())
    }
}
