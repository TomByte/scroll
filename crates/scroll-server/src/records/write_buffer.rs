//! Writes held back during a commit and done together: a transaction creating thousands of rows or setting
//! thousands of cells costs a few statements rather than thousands of round trips. Nothing checks what is held
//! back, so it is written before the next statement that reads (see `apply`), and at the end of each pushed
//! transaction.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use deadpool_postgres::Transaction as DbTransaction;
use scroll_core::id::{PageId, PropertyId, TagId, WorkspaceId};
use scroll_core::operation::RecordId;
use scroll_core::record::Page;
use scroll_core::validation::ValidationError;
use uuid::Uuid;

use super::{RecordError, encode};
use crate::lock;

/// Publishing creates every page and then sets its parent or database: a statement each would cost a round trip
/// each, hundreds of thousands of them.
#[derive(Default)]
struct UnwrittenPages {
    /// In order, each as it is now.
    created: Vec<Page>,
    /// Where each created page is in `created`.
    positions: HashMap<PageId, usize>,
    /// Existing pages, as they are now.
    changed: HashMap<PageId, Page>,
}

impl UnwrittenPages {
    fn create(&mut self, page: Page) {
        self.positions.insert(page.id, self.created.len());
        self.created.push(page);
    }

    fn change(&mut self, page: Page) {
        match self.positions.get(&page.id) {
            Some(&position) => self.created[position] = page,
            None => {
                self.changed.insert(page.id, page);
            }
        }
    }
}

/// The columns a page's row keeps besides its record, for writing many at once.
struct PageRows {
    ids: Vec<Uuid>,
    records: Vec<Vec<u8>>,
    parents: Vec<Option<Uuid>>,
    rows_of: Vec<Option<Uuid>>,
}

impl PageRows {
    fn of<'p>(pages: impl Iterator<Item = &'p Page>) -> Result<Self, RecordError> {
        let mut rows = PageRows {
            ids: Vec::new(),
            records: Vec::new(),
            parents: Vec::new(),
            rows_of: Vec::new(),
        };
        for page in pages {
            rows.ids.push(page.id.0);
            rows.records.push(encode(page)?);
            rows.parents.push(page.parent.map(|parent| parent.0));
            rows.rows_of.push(page.row_of.map(|database| database.0));
        }
        Ok(rows)
    }
}

/// Encoded property values by row and property; `None` clears the value.
type UnwrittenValues = HashMap<(PageId, PropertyId), Option<Vec<u8>>>;

#[derive(Default)]
pub struct WriteBuffer {
    pages: Mutex<UnwrittenPages>,
    values: Mutex<UnwrittenValues>,
    /// Tag memberships added (`true`) or taken away.
    memberships: Mutex<HashMap<(PageId, TagId), bool>>,
}

impl WriteBuffer {
    pub fn create_page(&self, page: Page) {
        lock(&self.pages).create(page);
    }

    pub fn change_page(&self, page: Page) {
        lock(&self.pages).change(page);
    }

    pub fn set_value(&self, (page, property): (PageId, PropertyId), value: Option<Vec<u8>>) {
        lock(&self.values).insert((page, property), value);
    }

    pub fn set_membership(&self, (page, tag): (PageId, TagId), is_member: bool) {
        lock(&self.memberships).insert((page, tag), is_member);
    }

    /// After a rollback to a savepoint: what was held back belonged to the transaction rolled back.
    pub fn clear(&self) {
        *lock(&self.pages) = UnwrittenPages::default();
        lock(&self.values).clear();
        lock(&self.memberships).clear();
    }

    /// Pages first: values and memberships belong to them.
    pub async fn write(
        &self,
        tx: &DbTransaction<'_>,
        workspace_id: WorkspaceId,
    ) -> Result<(), RecordError> {
        self.write_pages(tx, workspace_id).await?;
        self.write_memberships(tx, workspace_id).await?;
        Ok(self.write_values(tx, workspace_id).await?)
    }

    /// Inserts the pages created so far in one statement and updates the ones changed in another. A created
    /// page's id already in use is accepted only if it is bound to this workspace without a record (see
    /// `claim_page_id`).
    async fn write_pages(
        &self,
        tx: &DbTransaction<'_>,
        workspace_id: WorkspaceId,
    ) -> Result<(), RecordError> {
        let pages = std::mem::take(&mut *lock(&self.pages));
        if !pages.created.is_empty() {
            let rows = PageRows::of(pages.created.iter())?;
            let statement = tx
                .prepare_cached(
                    "INSERT INTO pages (id, workspace_id, record, parent_id, row_of)
                     SELECT id, $1, record, parent_id, row_of
                     FROM UNNEST($2::uuid[], $3::bytea[], $4::uuid[], $5::uuid[]) AS new (id, record, parent_id, row_of)
                     ON CONFLICT (id) DO NOTHING RETURNING id",
                )
                .await?;
            let inserted: HashSet<Uuid> = tx
                .query(
                    &statement,
                    &[
                        &workspace_id.0,
                        &rows.ids,
                        &rows.records,
                        &rows.parents,
                        &rows.rows_of,
                    ],
                )
                .await?
                .iter()
                .map(|row| row.get("id"))
                .collect();
            let taken: Vec<&Page> = pages
                .created
                .iter()
                .filter(|page| !inserted.contains(&page.id.0))
                .collect();
            for page in &taken {
                claim_page_id(tx, workspace_id, page.id).await?;
            }
            update_pages(tx, taken.into_iter()).await?;
        }
        update_pages(tx, pages.changed.values()).await
    }

    async fn write_memberships(
        &self,
        tx: &DbTransaction<'_>,
        workspace_id: WorkspaceId,
    ) -> Result<(), tokio_postgres::Error> {
        let unwritten = std::mem::take(&mut *lock(&self.memberships));
        let mut added = (Vec::new(), Vec::new());
        let mut removed = (Vec::new(), Vec::new());
        for ((page, tag), is_added) in unwritten {
            let list = if is_added { &mut added } else { &mut removed };
            list.0.push(page.0);
            list.1.push(tag.0);
        }
        if !added.0.is_empty() {
            let statement = tx
                .prepare_cached(
                    "INSERT INTO page_tags (workspace_id, page_id, tag_id)
                     SELECT $1, * FROM UNNEST($2::uuid[], $3::uuid[])
                     ON CONFLICT (page_id, tag_id) DO NOTHING",
                )
                .await?;
            tx.execute(&statement, &[&workspace_id.0, &added.0, &added.1])
                .await?;
        }
        if !removed.0.is_empty() {
            let statement = tx
                .prepare_cached(
                    "DELETE FROM page_tags WHERE workspace_id = $1 AND (page_id, tag_id) IN
                     (SELECT * FROM UNNEST($2::uuid[], $3::uuid[]))",
                )
                .await?;
            tx.execute(&statement, &[&workspace_id.0, &removed.0, &removed.1])
                .await?;
        }
        Ok(())
    }

    async fn write_values(
        &self,
        tx: &DbTransaction<'_>,
        workspace_id: WorkspaceId,
    ) -> Result<(), tokio_postgres::Error> {
        let unwritten = std::mem::take(&mut *lock(&self.values));
        if unwritten.is_empty() {
            return Ok(());
        }
        let mut set = (Vec::new(), Vec::new(), Vec::new());
        let mut cleared = (Vec::new(), Vec::new());
        // In key order, so the index is walked once rather than at random.
        let mut unwritten: Vec<_> = unwritten.into_iter().collect();
        unwritten.sort_unstable_by_key(|(key, _)| *key);
        for ((page, property), value) in unwritten {
            match value {
                Some(value) => {
                    set.0.push(page.0);
                    set.1.push(property.0);
                    set.2.push(value);
                }
                None => {
                    cleared.0.push(page.0);
                    cleared.1.push(property.0);
                }
            }
        }
        if !set.0.is_empty() {
            let statement = tx
                .prepare_cached(
                    "INSERT INTO property_values (workspace_id, page_id, property_id, value)
                     SELECT $1, * FROM UNNEST($2::uuid[], $3::uuid[], $4::bytea[])
                     ON CONFLICT (page_id, property_id) DO UPDATE SET value = excluded.value",
                )
                .await?;
            tx.execute(&statement, &[&workspace_id.0, &set.0, &set.1, &set.2])
                .await?;
        }
        if !cleared.0.is_empty() {
            let statement = tx
                .prepare_cached(
                    "DELETE FROM property_values WHERE (page_id, property_id) IN
                     (SELECT * FROM UNNEST($1::uuid[], $2::uuid[]))",
                )
                .await?;
            tx.execute(&statement, &[&cleared.0, &cleared.1]).await?;
        }
        Ok(())
    }
}

async fn update_pages<'p>(
    tx: &DbTransaction<'_>,
    pages: impl Iterator<Item = &'p Page>,
) -> Result<(), RecordError> {
    let rows = PageRows::of(pages)?;
    if rows.ids.is_empty() {
        return Ok(());
    }
    let statement = tx
        .prepare_cached(
            "UPDATE pages SET record = changed.record, parent_id = changed.parent_id, row_of = changed.row_of
             FROM UNNEST($1::uuid[], $2::bytea[], $3::uuid[], $4::uuid[]) AS changed (id, record, parent_id, row_of)
             WHERE pages.id = changed.id",
        )
        .await?;
    tx.execute(
        &statement,
        &[&rows.ids, &rows.records, &rows.parents, &rows.rows_of],
    )
    .await?;
    Ok(())
}

/// For a page id already in use: succeeds if it is bound to this workspace without a record. A page id with
/// content in this workspace may gain its record later; one used in another workspace never can.
/// Reads through the transaction directly: it runs while unwritten pages are being written.
async fn claim_page_id(
    tx: &DbTransaction<'_>,
    workspace_id: WorkspaceId,
    id: PageId,
) -> Result<(), RecordError> {
    let statement = tx
        .prepare_cached(
            "SELECT workspace_id, record IS NOT NULL AS has_record FROM pages WHERE id = $1 FOR UPDATE",
        )
        .await?;
    let row = tx.query_one(&statement, &[&id.0]).await?;
    let is_free_here =
        WorkspaceId(row.get("workspace_id")) == workspace_id && !row.get::<_, bool>("has_record");
    if !is_free_here {
        return Err(ValidationError::RecordAlreadyExists(RecordId::Page(id)).into());
    }
    Ok(())
}
