//! Views of databases.

use scroll_core::id::ViewId;
use scroll_core::operation::{Record, RecordId, ViewField};
use scroll_core::record::View;
use scroll_core::validation::{ValidationError, check_view, check_view_field};

use super::WorkspaceRecords;
use crate::records::catch_up::{SnapshotSource, stored_record};
use crate::records::{RecordError, decode, encode};

pub(super) const SNAPSHOT: &[SnapshotSource] = &[SnapshotSource {
    sql: "SELECT record FROM views WHERE workspace_id = $1",
    read: |row| stored_record(row, Record::View),
}];

impl WorkspaceRecords<'_> {
    pub(super) async fn create_view(&self, view: &View) -> Result<(), RecordError> {
        let id_is_taken = self
            .exists(
                "SELECT EXISTS (SELECT 1 FROM views WHERE id = $1)",
                view.id.0,
            )
            .await?;
        if id_is_taken {
            return Err(ValidationError::RecordAlreadyExists(RecordId::View(view.id)).into());
        }
        let database = self.require_database(view.database).await?;
        check_view(view, &database)?;
        self.execute(
            "INSERT INTO views (id, workspace_id, database_id, record) VALUES ($1, $2, $3, $4)",
            &[
                &view.id.0,
                &self.workspace_id.0,
                &view.database.0,
                &encode(view)?,
            ],
        )
        .await?;
        Ok(())
    }

    pub(super) async fn set_view_field(
        &self,
        id: ViewId,
        field: &ViewField,
    ) -> Result<(), RecordError> {
        let mut view = self.require_view(id).await?;
        let database = self.require_database(view.database).await?;
        check_view_field(field, &database)?;
        field.apply_to(&mut view);
        self.execute(
            "UPDATE views SET record = $2 WHERE id = $1",
            &[&id.0, &encode(&view)?],
        )
        .await?;
        Ok(())
    }

    pub(super) async fn delete_view(&self, id: ViewId) -> Result<(), RecordError> {
        self.require_view(id).await?;
        self.execute("DELETE FROM views WHERE id = $1", &[&id.0])
            .await?;
        Ok(())
    }
}

impl WorkspaceRecords<'_> {
    async fn require_view(&self, id: ViewId) -> Result<View, RecordError> {
        let row = self
            .query_opt(
                "SELECT record FROM views WHERE id = $1 AND workspace_id = $2",
                &[&id.0, &self.workspace_id.0],
            )
            .await?
            .ok_or(ValidationError::ViewNotFound(id))?;
        decode(row.get("record"))
    }
}
