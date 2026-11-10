//! Property values, keyed by property first so a view reads one property in one range.

use rusqlite::{Connection, params};

use crate::id::{PageId, PropertyId};
use crate::record::PropertyValue;
use crate::store::{StoreError, WorkspaceStore, decode, encode};

impl WorkspaceStore {
    /// Values may name properties since removed from the schema, or hold a type the property
    /// no longer has; callers show only those that match the current schema.
    pub fn property_values(
        &self,
        page: PageId,
    ) -> Result<Vec<(PropertyId, PropertyValue)>, StoreError> {
        read_property_values(&self.conn, page)
    }
}

/// In property id order.
pub(in crate::store) fn read_property_values(
    conn: &Connection,
    page: PageId,
) -> Result<Vec<(PropertyId, PropertyValue)>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT property_id, value FROM property_value WHERE page_id = ?1 ORDER BY property_id",
    )?;
    let rows = statement.query_map([page], |row| {
        Ok((row.get::<_, PropertyId>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    rows.map(|row| {
        let (property, bytes) = row?;
        Ok((property, decode(&bytes)?))
    })
    .collect()
}

pub(in crate::store) fn replace_property_values(
    conn: &Connection,
    page: PageId,
    values: &[(PropertyId, PropertyValue)],
) -> Result<(), StoreError> {
    delete_values_of_page(conn, page)?;
    for (property, value) in values {
        write_property_value(conn, page, *property, Some(value))?;
    }
    Ok(())
}

/// Calls `visit` with every stored property value in (page, property) order, without holding
/// them all.
pub(in crate::store) fn for_each_property_value_by_page(
    conn: &Connection,
    mut visit: impl FnMut(PageId, PropertyId, PropertyValue) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "SELECT page_id, property_id, value FROM property_value INDEXED BY property_value_by_page
         ORDER BY page_id, property_id",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let bytes = row.get_ref(2)?.as_blob().map_err(rusqlite::Error::from)?;
        visit(row.get(0)?, row.get(1)?, decode(bytes)?)?;
    }
    Ok(())
}

/// Every stored property value as (page, property, value), in storage order (by property).
pub(in crate::store) fn all_property_values(
    conn: &Connection,
) -> Result<Vec<(PageId, PropertyId, PropertyValue)>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT page_id, property_id, value FROM property_value ORDER BY property_id, page_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, PageId>(0)?,
            row.get::<_, PropertyId>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (page, property, bytes) = row?;
        Ok((page, property, decode(&bytes)?))
    })
    .collect()
}

pub(in crate::store) fn write_property_value(
    conn: &Connection,
    page: PageId,
    property: PropertyId,
    value: Option<&PropertyValue>,
) -> Result<(), StoreError> {
    match value {
        Some(value) => conn
            .prepare_cached(
                "INSERT INTO property_value (page_id, property_id, value)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM page WHERE id = ?1)
             ON CONFLICT (page_id, property_id) DO UPDATE SET value = excluded.value",
            )?
            .execute(params![page, property, encode(value)?])?,
        None => conn
            .prepare_cached("DELETE FROM property_value WHERE page_id = ?1 AND property_id = ?2")?
            .execute(params![page, property])?,
    };
    Ok(())
}

pub(in crate::store) fn delete_values_of_page(
    conn: &Connection,
    page: PageId,
) -> Result<(), StoreError> {
    conn.prepare_cached("DELETE FROM property_value WHERE page_id = ?1")?
        .execute([page])?;
    Ok(())
}
