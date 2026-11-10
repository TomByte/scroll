//! Pages: their rows, their place in the tree, and what a delete leaves behind.

use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::id::{DatabaseId, PageId};
use crate::operation::PageField;
use crate::position::Position;
use crate::record::Page;
use crate::store::content;
use crate::store::kinds::{page_tag, value};
use crate::store::{StoreError, WorkspaceStore, search};

impl WorkspaceStore {
    pub fn page(&self, id: PageId) -> Result<Option<Page>, StoreError> {
        read_page(&self.conn, id)
    }

    /// The position of the database's last row, to place a new row after it.
    pub fn last_row_position(&self, database: DatabaseId) -> Result<Option<Position>, StoreError> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT position FROM page WHERE row_of_database_id = ?1
                 ORDER BY position DESC, id DESC LIMIT 1",
            )?
            .query_row([database], |row| row.get(0))
            .optional()?)
    }
}

/// The SQL form of `Page::is_in_tree`, for a table or alias. The partial index `page_in_tree`
/// is built with exactly this condition, and SQLite only uses that index for queries whose
/// condition matches it.
macro_rules! in_tree {
    () => {
        "row_of_database_id IS NULL AND NOT is_archived"
    };
    ($table:ident) => {
        concat!(
            stringify!($table),
            ".row_of_database_id IS NULL AND NOT ",
            stringify!($table),
            ".is_archived"
        )
    };
}
pub(in crate::store) use in_tree;

pub(in crate::store) const PAGE_COLUMNS: &str =
    "id, parent_id, title, icon, position, row_of_database_id, is_archived";

pub(in crate::store) fn read_page(
    conn: &Connection,
    id: PageId,
) -> Result<Option<Page>, StoreError> {
    let sql = format!("SELECT {PAGE_COLUMNS} FROM page WHERE id = ?1");
    Ok(conn
        .prepare_cached(&sql)?
        .query_row([id], page_from_row)
        .optional()?)
}

pub(in crate::store) fn child_page_count(
    conn: &Connection,
    page: PageId,
) -> Result<usize, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM page WHERE parent_id = ?1",
        [page],
        |row| row.get(0),
    )?)
}

/// The page followed by its ancestors. `UNION` stops at a repeat, so a cycle (possible
/// transiently from concurrent remote moves) cannot loop forever.
pub(in crate::store) fn page_lineage(
    conn: &Connection,
    page: PageId,
) -> Result<Vec<PageId>, StoreError> {
    let mut statement = conn.prepare_cached(
        "WITH RECURSIVE lineage (id) AS (
            SELECT ?1
            UNION
            SELECT page.parent_id FROM page JOIN lineage ON page.id = lineage.id WHERE page.parent_id IS NOT NULL
        )
        SELECT id FROM lineage",
    )?;
    let lineage = statement
        .query_map([page], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(lineage)
}

/// Writes every column, inserting the page if it is missing. Keeps `local_key`, so the search entry survives.
pub(in crate::store) fn upsert_page(conn: &Connection, page: &Page) -> Result<(), StoreError> {
    let updated_count = conn.prepare_cached("UPDATE page SET parent_id = ?2, title = ?3, icon = ?4, position = ?5, row_of_database_id = ?6,
         is_archived = ?7 WHERE id = ?1")?
        .execute(
        params![page.id, page.parent, page.title, page.icon, page.position, page.row_of, page.is_archived],
    )?;
    if updated_count == 0 {
        return insert_page(conn, page);
    }
    search::update_title(conn, page.id, &page.title)
}

pub(in crate::store) fn all_pages(conn: &Connection) -> Result<Vec<Page>, StoreError> {
    let sql = format!("SELECT {PAGE_COLUMNS} FROM page ORDER BY local_key");
    let pages = conn
        .prepare_cached(&sql)?
        .query_map([], page_from_row)?
        .collect::<Result<_, _>>()?;
    Ok(pages)
}

/// The ids of the pages directly under `parent`, rows included, in sibling order.
pub(in crate::store) fn child_page_ids(
    conn: &Connection,
    parent: Option<PageId>,
) -> Result<Vec<PageId>, StoreError> {
    let mut statement =
        conn.prepare_cached("SELECT id FROM page WHERE parent_id IS ?1 ORDER BY position, id")?;
    let ids = statement.query_map([parent], |row| row.get(0))?;
    Ok(ids.collect::<Result<_, _>>()?)
}

pub(in crate::store) fn insert_page(conn: &Connection, page: &Page) -> Result<(), StoreError> {
    let inserted_count = conn.prepare_cached("INSERT OR IGNORE INTO page (id, parent_id, title, icon, position, row_of_database_id, is_archived)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?
        .execute(
        params![page.id, page.parent, page.title, page.icon, page.position, page.row_of, page.is_archived],
    )?;
    if inserted_count == 0 {
        return Ok(());
    }
    search::insert_entry(conn, conn.last_insert_rowid(), &page.title)?;
    page_tag::mark_memberships_of_returned_page(conn, page.id)
}

pub(in crate::store) fn write_page_field(
    conn: &Connection,
    id: PageId,
    field: &PageField,
) -> Result<(), StoreError> {
    match field {
        PageField::Parent(parent) => update_page_column(conn, id, "parent_id", parent),
        PageField::Title(title) => {
            update_page_column(conn, id, "title", title)?;
            search::update_title(conn, id, title)
        }
        PageField::Icon(icon) => update_page_column(conn, id, "icon", icon),
        PageField::Position(position) => update_page_column(conn, id, "position", position),
        PageField::RowOf(database) => update_page_column(conn, id, "row_of_database_id", database),
        PageField::IsArchived(is_archived) => {
            update_page_column(conn, id, "is_archived", is_archived)
        }
    }
}

/// `column` always comes from a literal in `write_page_field`, never from input.
fn update_page_column(
    conn: &Connection,
    id: PageId,
    column: &str,
    value: &dyn ToSql,
) -> Result<(), StoreError> {
    conn.prepare_cached(&format!("UPDATE page SET {column} = ?2 WHERE id = ?1"))?
        .execute(params![id, value])?;
    Ok(())
}

/// Reads the `PAGE_COLUMNS` of a result row by name, so queries may select more columns.
pub(in crate::store) fn page_from_row(row: &Row<'_>) -> rusqlite::Result<Page> {
    Ok(Page {
        id: row.get("id")?,
        parent: row.get("parent_id")?,
        title: row.get("title")?,
        icon: row.get("icon")?,
        position: row.get("position")?,
        row_of: row.get("row_of_database_id")?,
        is_archived: row.get("is_archived")?,
    })
}

pub(in crate::store) fn page_exists(conn: &Connection, id: PageId) -> Result<bool, StoreError> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM page WHERE id = ?1)")?
        .query_row([id], |row| row.get(0))?)
}

/// Removes the page and its values, leaving its content and tag memberships for
/// `delete_dependants`.
pub(in crate::store) fn delete_page(conn: &Connection, id: PageId) -> Result<(), StoreError> {
    search::delete_entry(conn, id)?;
    page_tag::mark_memberships_of_gone_page(conn, id)?;
    value::delete_values_of_page(conn, id)?;
    conn.prepare_cached("DELETE FROM page WHERE id = ?1")?
        .execute([id])?;
    Ok(())
}

pub(in crate::store) fn delete_dependants(conn: &Connection, id: PageId) -> Result<(), StoreError> {
    content::log::delete_content(conn, id)?;
    page_tag::delete_memberships_of_page(conn, id)
}
