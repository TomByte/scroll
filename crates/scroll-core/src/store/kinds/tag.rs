//! Tags.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::id::TagId;
use crate::operation::TagField;
use crate::record::Tag;
use crate::store::{StoreError, WorkspaceStore, decode, encode, search};
use crate::validation::tag_name_key;

impl WorkspaceStore {
    pub fn tag(&self, id: TagId) -> Result<Option<Tag>, StoreError> {
        read_tag(&self.conn, id)
    }

    /// The tag whose name equals `name` ignoring case and surrounding spaces. Use it before
    /// creating a tag, so typing an existing name reuses that tag.
    pub fn tag_named(&self, name: &str) -> Result<Option<Tag>, StoreError> {
        match tag_holding_name(&self.conn, name)? {
            Some(id) => read_tag(&self.conn, id),
            None => Ok(None),
        }
    }
}

pub(in crate::store) fn read_tag(conn: &Connection, id: TagId) -> Result<Option<Tag>, StoreError> {
    let encoded = conn
        .prepare_cached("SELECT id, name, color FROM tag WHERE id = ?1")?
        .query_row([id], encoded_tag)
        .optional()?;
    encoded.map(EncodedTag::decode).transpose()
}

pub(in crate::store) fn tag_holding_name(
    conn: &Connection,
    name: &str,
) -> Result<Option<TagId>, StoreError> {
    Ok(conn
        .prepare_cached("SELECT id FROM tag WHERE name_key = ?1 ORDER BY id LIMIT 1")?
        .query_row([tag_name_key(name)], |row| row.get(0))
        .optional()?)
}

pub(in crate::store) fn tag_exists(conn: &Connection, id: TagId) -> Result<bool, StoreError> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM tag WHERE id = ?1)")?
        .query_row([id], |row| row.get(0))?)
}

pub(in crate::store) fn insert_tag(conn: &Connection, tag: &Tag) -> Result<(), StoreError> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO tag (id, name, name_key, color) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![
        tag.id,
        tag.name,
        tag_name_key(&tag.name),
        encode(&tag.color)?
    ])?;
    Ok(())
}

/// Writes every column, inserting the tag if it is missing. Pages carrying it are re-indexed
/// when its name changed or it reappeared.
pub(in crate::store) fn upsert_tag(conn: &Connection, tag: &Tag) -> Result<(), StoreError> {
    let previous = read_tag(conn, tag.id)?;
    conn.prepare_cached(
        "INSERT INTO tag (id, name, name_key, color) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, name_key = excluded.name_key,
         color = excluded.color",
    )?
    .execute(params![
        tag.id,
        tag.name,
        tag_name_key(&tag.name),
        encode(&tag.color)?
    ])?;
    if previous.is_none_or(|previous| previous.name != tag.name) {
        search::refresh_tags_of_pages_tagged(conn, tag.id)?;
    }
    Ok(())
}

pub(in crate::store) fn write_tag_field(
    conn: &Connection,
    id: TagId,
    field: &TagField,
) -> Result<(), StoreError> {
    match field {
        TagField::Name(name) => {
            let updated_count = conn
                .prepare_cached("UPDATE tag SET name = ?2, name_key = ?3 WHERE id = ?1")?
                .execute(params![id, name, tag_name_key(name)])?;
            if updated_count > 0 {
                search::refresh_tags_of_pages_tagged(conn, id)?;
            }
        }
        TagField::Color(color) => {
            conn.prepare_cached("UPDATE tag SET color = ?2 WHERE id = ?1")?
                .execute(params![id, encode(color)?])?;
        }
    }
    Ok(())
}

/// Removes the tag but keeps its memberships, which the join on `tag` hides from readers.
pub(in crate::store) fn delete_tag(conn: &Connection, id: TagId) -> Result<(), StoreError> {
    let deleted_count = conn.execute("DELETE FROM tag WHERE id = ?1", [id])?;
    if deleted_count > 0 {
        search::refresh_tags_of_pages_tagged(conn, id)?;
    }
    Ok(())
}

pub(in crate::store) fn all_tags(conn: &Connection) -> Result<Vec<Tag>, StoreError> {
    let mut statement = conn.prepare_cached("SELECT id, name, color FROM tag ORDER BY id")?;
    let rows = statement.query_map([], encoded_tag)?;
    rows.map(|row| row?.decode()).collect()
}

/// A tag row before its colour is decoded; decoding errors are not SQLite errors.
pub(in crate::store) struct EncodedTag {
    id: TagId,
    name: String,
    color: Vec<u8>,
}

impl EncodedTag {
    pub(in crate::store) fn decode(self) -> Result<Tag, StoreError> {
        Ok(Tag {
            id: self.id,
            name: self.name,
            color: decode(&self.color)?,
        })
    }
}

pub(in crate::store) fn encoded_tag(row: &Row<'_>) -> rusqlite::Result<EncodedTag> {
    Ok(EncodedTag {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
    })
}
