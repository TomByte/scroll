//! Validating and applying operations inside a committing transaction, one module per kind of record.
//!
//! Reads go through caches, and many writes are held back in a `WriteBuffer`. The query helpers at the bottom
//! write what is held back before running anything, so every read sees the writes before it; code that writes
//! the buffer itself goes to the transaction directly.

/// Declares a module per stored kind and builds the snapshot's sources from every one's `SNAPSHOT`, so a kind
/// cannot be added without saying where a snapshot reads it from.
macro_rules! kinds {
    ($($kind:ident),* $(,)?) => {
        $(mod $kind;)*

        /// Where a snapshot reads each kind from, in this order: what is referred to before what refers to it.
        pub(super) const SNAPSHOT_SOURCES: &[&[SnapshotSource]] = &[$($kind::SNAPSHOT),*];
    };
}

kinds!(pages, databases, views, tags, values);

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use deadpool_postgres::Transaction as DbTransaction;
use scroll_core::id::{DatabaseId, PageId, TagId, WorkspaceId};
use scroll_core::operation::{FieldChange, Operation, Record, RecordId, Transaction};
use scroll_core::record::{Database, Page, PageTag, Tag};
use scroll_core::validation::ValidationError;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use super::catch_up::SnapshotSource;
use super::commit::PushedTransaction;
use super::write_buffer::WriteBuffer;
use super::{RecordError, decode};
use crate::db::wire_seq;
use crate::lock;

/// The records of one workspace, read and written inside a committing transaction.
pub struct WorkspaceRecords<'a> {
    tx: &'a DbTransaction<'a>,
    workspace_id: WorkspaceId,
    /// Pages, databases and tags read or written so far in this transaction (`None`: known not to exist), so a
    /// transaction setting many rows' values reads each database and parent page once.
    pages: Mutex<HashMap<PageId, Option<Page>>>,
    databases: Mutex<HashMap<DatabaseId, Option<Database>>>,
    tags: Mutex<HashMap<TagId, Option<Tag>>>,
    buffer: WriteBuffer,
}

impl<'a> WorkspaceRecords<'a> {
    pub(super) fn new(tx: &'a DbTransaction<'a>, workspace_id: WorkspaceId) -> Self {
        Self {
            tx,
            workspace_id,
            pages: Mutex::default(),
            databases: Mutex::default(),
            tags: Mutex::default(),
            buffer: WriteBuffer::default(),
        }
    }

    /// Takes the workspace lock and returns the last sequence number and which of the group's transactions
    /// were committed before, with their sequence numbers. Also reads the pages the group refers to. The
    /// queries go out together; the server runs them in order, so the reads happen under the lock.
    pub(super) async fn lock_workspace(
        &self,
        group: &[PushedTransaction<'_>],
    ) -> Result<(u64, HashMap<Uuid, u64>), RecordError> {
        let transaction_ids: Vec<Uuid> =
            group.iter().map(|pushed| pushed.transaction.id.0).collect();
        let page_ids: Vec<Uuid> = group
            .iter()
            .flat_map(|pushed| &pushed.transaction.operations)
            .filter_map(page_read_by)
            .map(|page| page.0)
            .collect::<HashSet<Uuid>>()
            .into_iter()
            .collect();
        let lock_statement = self
            .tx
            .prepare_cached("SELECT last_record_seq FROM workspaces WHERE id = $1 FOR UPDATE")
            .await?;
        let earlier_statement = self
            .tx
            .prepare_cached(
                "SELECT transaction_id, seq FROM record_transactions
                 WHERE workspace_id = $1 AND transaction_id = ANY($2)",
            )
            .await?;
        let pages_statement = self
            .tx
            .prepare_cached(
                "SELECT id, record FROM page_records WHERE id = ANY($1) AND workspace_id = $2",
            )
            .await?;
        let workspace = &self.workspace_id.0;
        let lock_params: [&(dyn ToSql + Sync); 1] = [workspace];
        let earlier_params: [&(dyn ToSql + Sync); 2] = [workspace, &transaction_ids];
        let pages_params: [&(dyn ToSql + Sync); 2] = [&page_ids, workspace];
        let (locked, earlier, pages) = tokio::try_join!(
            self.tx.query_one(&lock_statement, &lock_params),
            self.tx.query(&earlier_statement, &earlier_params),
            self.tx.query(&pages_statement, &pages_params),
        )?;
        let committed = earlier
            .iter()
            .map(|row| (row.get("transaction_id"), wire_seq(row.get("seq"))))
            .collect();
        let mut known = lock(&self.pages);
        for id in page_ids {
            known.insert(PageId(id), None);
        }
        for row in &pages {
            known.insert(PageId(row.get("id")), Some(decode(row.get("record"))?));
        }
        drop(known);
        Ok((wire_seq(locked.get(0)), committed))
    }

    /// Applies every operation in order; returns the database whose rows are all it changed, if that is all.
    /// What it writes may be left in the buffer: see `write_unwritten`.
    pub(super) async fn apply_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<Option<DatabaseId>, RecordError> {
        let mut rows_of: Option<Option<DatabaseId>> = None;
        for operation in &transaction.operations {
            let database = self.row_database(operation).await?;
            rows_of = match rows_of {
                Some(previous) if previous != database => Some(None),
                _ => Some(database),
            };
            self.apply(operation).await?;
        }
        Ok(rows_of.flatten())
    }

    /// After a rollback to a savepoint: what was read or held may no longer be so.
    pub(super) fn forget_unwritten(&self) {
        lock(&self.pages).clear();
        lock(&self.databases).clear();
        lock(&self.tags).clear();
        self.buffer.clear();
    }

    /// Writes the pages created and the property values set so far.
    pub(super) async fn write_unwritten(&self) -> Result<(), RecordError> {
        self.buffer.write(self.tx, self.workspace_id).await
    }

    async fn apply(&self, operation: &Operation) -> Result<(), RecordError> {
        match operation {
            Operation::Create(Record::Page(page)) => self.create_page(page).await,
            Operation::Create(Record::Database(database)) => self.create_database(database).await,
            Operation::Create(Record::View(view)) => self.create_view(view).await,
            Operation::Set(FieldChange::Page { id, field }) => {
                self.set_page_field(*id, field).await
            }
            Operation::Set(FieldChange::Database { id, field }) => {
                self.set_database_field(*id, field).await
            }
            Operation::Set(FieldChange::PropertyValue {
                page,
                property,
                value,
            }) => {
                self.set_property_value(*page, *property, value.as_ref())
                    .await
            }
            Operation::Set(FieldChange::View { id, field }) => {
                self.set_view_field(*id, field).await
            }
            Operation::Delete(RecordId::Page(id)) => self.delete_page(*id).await,
            Operation::Delete(RecordId::Database(id)) => self.delete_database(*id).await,
            Operation::Delete(RecordId::View(id)) => self.delete_view(*id).await,
            Operation::Create(Record::Tag(tag)) => self.create_tag(tag).await,
            Operation::Set(FieldChange::Tag { id, field }) => self.set_tag_field(*id, field).await,
            Operation::Delete(RecordId::Tag(id)) => self.delete_tag(*id).await,
            Operation::Create(Record::PageTag(membership)) => self.tag_page(*membership).await,
            Operation::Delete(RecordId::PageTag { page, tag }) => {
                self.untag_page(PageTag {
                    page: *page,
                    tag: *tag,
                });
                Ok(())
            }
        }
    }

    async fn require_page(&self, id: PageId) -> Result<Page, RecordError> {
        Ok(self
            .known_page(id)
            .await?
            .ok_or(ValidationError::PageNotFound(id))?)
    }

    /// The page, or `None` if this workspace has no page with that id.
    pub(super) async fn known_page(&self, id: PageId) -> Result<Option<Page>, RecordError> {
        let known = lock(&self.pages).get(&id).cloned();
        let page = match known {
            Some(page) => page,
            None => {
                let row = self
                    .query_opt(
                        "SELECT record FROM page_records WHERE id = $1 AND workspace_id = $2",
                        &[&id.0, &self.workspace_id.0],
                    )
                    .await?;
                let page: Option<Page> = row.map(|row| decode(row.get("record"))).transpose()?;
                lock(&self.pages).insert(id, page.clone());
                page
            }
        };
        Ok(page)
    }

    async fn require_database(&self, id: DatabaseId) -> Result<Database, RecordError> {
        let known = lock(&self.databases).get(&id).cloned();
        let database = match known {
            Some(database) => database,
            None => {
                let row = self
                    .query_opt(
                        "SELECT record FROM databases WHERE id = $1 AND workspace_id = $2",
                        &[&id.0, &self.workspace_id.0],
                    )
                    .await?;
                let database: Option<Database> =
                    row.map(|row| decode(row.get("record"))).transpose()?;
                lock(&self.databases).insert(id, database.clone());
                database
            }
        };
        Ok(database.ok_or(ValidationError::DatabaseNotFound(id))?)
    }

    async fn database_hosted_on(&self, page: PageId) -> Result<Option<DatabaseId>, RecordError> {
        let row = self
            .query_opt(
                "SELECT id FROM databases WHERE host_page = $1 AND workspace_id = $2",
                &[&page.0, &self.workspace_id.0],
            )
            .await?;
        Ok(row.map(|row| DatabaseId(row.get("id"))))
    }

    /// Runs a `SELECT EXISTS (...)` query that takes one id.
    async fn exists(&self, sql: &str, id: Uuid) -> Result<bool, RecordError> {
        Ok(self.query_one(sql, &[&id]).await?.get(0))
    }

    // Statements are prepared once per database connection and reused: a transaction runs the same few
    // queries thousands of times, and preparing each would double its round trips. Each first writes the
    // pages and property values held back so far, so it sees them.

    async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64, RecordError> {
        self.write_unwritten().await?;
        let statement = self.tx.prepare_cached(sql).await?;
        Ok(self.tx.execute(&statement, params).await?)
    }

    async fn query_opt(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, RecordError> {
        self.write_unwritten().await?;
        let statement = self.tx.prepare_cached(sql).await?;
        Ok(self.tx.query_opt(&statement, params).await?)
    }

    async fn query_one(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, RecordError> {
        self.write_unwritten().await?;
        let statement = self.tx.prepare_cached(sql).await?;
        Ok(self.tx.query_one(&statement, params).await?)
    }

    /// Runs a `SELECT count(*)` query that takes one id. The deletion checks only ask whether there are any, so
    /// those queries stop at the first match and count at most one.
    async fn count(&self, sql: &str, id: Uuid) -> Result<usize, RecordError> {
        let count: i64 = self.query_one(sql, &[&id]).await?.get(0);
        Ok(usize::try_from(count).expect("a row count is never negative"))
    }
}

/// The existing page an operation reads, for reading the pages of a whole group at once.
fn page_read_by(operation: &Operation) -> Option<PageId> {
    match operation {
        Operation::Create(Record::Page(page)) => page.parent,
        Operation::Create(Record::Database(database)) => Some(database.host_page),
        _ => page_changed_by(operation),
    }
}

/// The existing page whose record, values or tags an operation changes.
pub(super) fn page_changed_by(operation: &Operation) -> Option<PageId> {
    match operation {
        Operation::Set(FieldChange::Page { id, .. }) | Operation::Delete(RecordId::Page(id)) => {
            Some(*id)
        }
        Operation::Set(FieldChange::PropertyValue { page, .. })
        | Operation::Create(Record::PageTag(PageTag { page, .. }))
        | Operation::Delete(RecordId::PageTag { page, .. }) => Some(*page),
        _ => None,
    }
}
