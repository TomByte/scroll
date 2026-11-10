//! Table views.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::id::{DatabaseId, ViewId};
use crate::operation::ViewField;
use crate::record::View;
use crate::store::{StoreError, WorkspaceStore, decode, encode};

impl WorkspaceStore {
    pub fn view(&self, id: ViewId) -> Result<Option<View>, StoreError> {
        read_view(&self.conn, id)
    }

    pub fn views(&self, database: DatabaseId) -> Result<Vec<View>, StoreError> {
        let sql = format!("SELECT {VIEW_COLUMNS} FROM view WHERE database_id = ?1 ORDER BY id");
        let mut statement = self.conn.prepare_cached(&sql)?;
        let rows = statement.query_map([database], encoded_view_from_row)?;
        rows.map(|row| row?.decode()).collect()
    }
}

const VIEW_COLUMNS: &str =
    "id, database_id, name, view_type, filter, sort, visible_columns, column_widths";

pub(in crate::store) fn read_view(
    conn: &Connection,
    id: ViewId,
) -> Result<Option<View>, StoreError> {
    let sql = format!("SELECT {VIEW_COLUMNS} FROM view WHERE id = ?1");
    let encoded = conn
        .prepare_cached(&sql)?
        .query_row([id], encoded_view_from_row)
        .optional()?;
    encoded.map(EncodedView::decode).transpose()
}

pub(in crate::store) fn view_count(
    conn: &Connection,
    database: DatabaseId,
) -> Result<usize, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM view WHERE database_id = ?1",
        [database],
        |row| row.get(0),
    )?)
}

pub(in crate::store) fn upsert_view(conn: &Connection, view: &View) -> Result<(), StoreError> {
    conn.execute("DELETE FROM view WHERE id = ?1", [view.id])?;
    insert_view(conn, view)
}

pub(in crate::store) fn all_views(conn: &Connection) -> Result<Vec<View>, StoreError> {
    let sql = format!("SELECT {VIEW_COLUMNS} FROM view ORDER BY id");
    let mut statement = conn.prepare_cached(&sql)?;
    let rows = statement.query_map([], encoded_view_from_row)?;
    rows.map(|row| row?.decode()).collect()
}

pub(in crate::store) fn insert_view(conn: &Connection, view: &View) -> Result<(), StoreError> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO view
         (id, database_id, name, view_type, filter, sort, visible_columns, column_widths)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![
        view.id,
        view.database,
        view.name,
        encode(&view.view_type)?,
        encode(&view.filter)?,
        encode(&view.sort)?,
        encode(&view.visible_columns)?,
        encode(&view.column_widths)?
    ])?;
    Ok(())
}

pub(in crate::store) fn write_view_field(
    conn: &Connection,
    id: ViewId,
    field: &ViewField,
) -> Result<(), StoreError> {
    let (column, bytes) = match field {
        ViewField::Name(name) => {
            conn.execute("UPDATE view SET name = ?2 WHERE id = ?1", params![id, name])?;
            return Ok(());
        }
        ViewField::ViewType(view_type) => ("view_type", encode(view_type)?),
        ViewField::Filter(filter) => ("filter", encode(filter)?),
        ViewField::Sort(sort) => ("sort", encode(sort)?),
        ViewField::VisibleColumns(columns) => ("visible_columns", encode(columns)?),
        ViewField::ColumnWidths(widths) => ("column_widths", encode(widths)?),
    };
    conn.prepare_cached(&format!("UPDATE view SET {column} = ?2 WHERE id = ?1"))?
        .execute(params![id, bytes])?;
    Ok(())
}

/// A view row before its postcard columns are decoded; decoding errors are not SQLite errors.
struct EncodedView {
    id: ViewId,
    database: DatabaseId,
    name: String,
    view_type: Vec<u8>,
    filter: Vec<u8>,
    sort: Vec<u8>,
    visible_columns: Vec<u8>,
    column_widths: Vec<u8>,
}

impl EncodedView {
    fn decode(self) -> Result<View, StoreError> {
        Ok(View {
            id: self.id,
            database: self.database,
            name: self.name,
            view_type: decode(&self.view_type)?,
            filter: decode(&self.filter)?,
            sort: decode(&self.sort)?,
            visible_columns: decode(&self.visible_columns)?,
            column_widths: decode(&self.column_widths)?,
        })
    }
}

fn encoded_view_from_row(row: &Row<'_>) -> rusqlite::Result<EncodedView> {
    Ok(EncodedView {
        id: row.get(0)?,
        database: row.get(1)?,
        name: row.get(2)?,
        view_type: row.get(3)?,
        filter: row.get(4)?,
        sort: row.get(5)?,
        visible_columns: row.get(6)?,
        column_widths: row.get(7)?,
    })
}

pub(in crate::store) fn view_exists(conn: &Connection, id: ViewId) -> Result<bool, StoreError> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM view WHERE id = ?1)")?
        .query_row([id], |row| row.get(0))?)
}

pub(in crate::store) fn delete_view(conn: &Connection, id: ViewId) -> Result<(), StoreError> {
    conn.prepare_cached("DELETE FROM view WHERE id = ?1")?
        .execute([id])?;
    Ok(())
}
