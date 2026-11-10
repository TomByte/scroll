//! Databases and their schema properties.

use rusqlite::{Connection, OptionalExtension, params};

use crate::id::{DatabaseId, PageId};
use crate::operation::DatabaseField;
use crate::record::{Database, Property};
use crate::store::{StoreError, WorkspaceStore, decode, encode};

impl WorkspaceStore {
    pub fn database(&self, id: DatabaseId) -> Result<Option<Database>, StoreError> {
        read_database(&self.conn, id)
    }

    pub fn database_hosted_by(&self, page: PageId) -> Result<Option<Database>, StoreError> {
        match database_hosted_by(&self.conn, page)? {
            Some(id) => read_database(&self.conn, id),
            None => Ok(None),
        }
    }
}

pub(in crate::store) fn read_database(
    conn: &Connection,
    id: DatabaseId,
) -> Result<Option<Database>, StoreError> {
    let host_page: Option<PageId> = conn
        .query_row(
            "SELECT host_page_id FROM database WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(host_page) = host_page else {
        return Ok(None);
    };
    let mut statement = conn.prepare_cached(
        "SELECT definition FROM database_property WHERE database_id = ?1 ORDER BY property_id",
    )?;
    let definitions = statement.query_map([id], |row| row.get::<_, Vec<u8>>(0))?;
    let properties = definitions
        .map(|bytes| decode::<Property>(&bytes?))
        .collect::<Result<_, _>>()?;
    Ok(Some(Database {
        id,
        host_page,
        properties,
    }))
}

pub(in crate::store) fn database_hosted_by(
    conn: &Connection,
    page: PageId,
) -> Result<Option<DatabaseId>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT id FROM database WHERE host_page_id = ?1",
            [page],
            |row| row.get(0),
        )
        .optional()?)
}

pub(in crate::store) fn row_count(
    conn: &Connection,
    database: DatabaseId,
) -> Result<usize, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM page WHERE row_of_database_id = ?1",
        [database],
        |row| row.get(0),
    )?)
}

pub(in crate::store) fn upsert_database(
    conn: &Connection,
    database: &Database,
) -> Result<(), StoreError> {
    conn.prepare_cached(
        "INSERT INTO database (id, host_page_id) VALUES (?1, ?2)
         ON CONFLICT (id) DO UPDATE SET host_page_id = excluded.host_page_id",
    )?
    .execute(params![database.id, database.host_page])?;
    conn.prepare_cached("DELETE FROM database_property WHERE database_id = ?1")?
        .execute([database.id])?;
    for property in &database.properties {
        upsert_property(conn, database.id, property)?;
    }
    Ok(())
}

pub(in crate::store) fn all_databases(conn: &Connection) -> Result<Vec<Database>, StoreError> {
    let ids: Vec<DatabaseId> = conn
        .prepare_cached("SELECT id FROM database ORDER BY id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let databases = ids.into_iter().map(|id| read_database(conn, id));
    databases.filter_map(Result::transpose).collect()
}

pub(in crate::store) fn insert_database(
    conn: &Connection,
    database: &Database,
) -> Result<(), StoreError> {
    let inserted_count = conn
        .prepare_cached("INSERT OR IGNORE INTO database (id, host_page_id) VALUES (?1, ?2)")?
        .execute(params![database.id, database.host_page])?;
    if inserted_count == 0 {
        return Ok(());
    }
    for property in &database.properties {
        upsert_property(conn, database.id, property)?;
    }
    Ok(())
}

pub(in crate::store) fn write_database_field(
    conn: &Connection,
    id: DatabaseId,
    field: &DatabaseField,
) -> Result<(), StoreError> {
    match field {
        DatabaseField::Property(property) => upsert_property(conn, id, property),
        DatabaseField::RemovedProperty(property) => {
            conn.prepare_cached(
                "DELETE FROM database_property WHERE database_id = ?1 AND property_id = ?2",
            )?
            .execute(params![id, property])?;
            Ok(())
        }
    }
}

pub(in crate::store) fn upsert_property(
    conn: &Connection,
    database: DatabaseId,
    property: &Property,
) -> Result<(), StoreError> {
    conn.prepare_cached(
        "INSERT INTO database_property (database_id, property_id, definition)
         SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM database WHERE id = ?1)
         ON CONFLICT (database_id, property_id) DO UPDATE SET definition = excluded.definition",
    )?
    .execute(params![database, property.id, encode(property)?])?;
    Ok(())
}

pub(in crate::store) fn database_exists(
    conn: &Connection,
    id: DatabaseId,
) -> Result<bool, StoreError> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM database WHERE id = ?1)")?
        .query_row([id], |row| row.get(0))?)
}

pub(in crate::store) fn delete_database(
    conn: &Connection,
    id: DatabaseId,
) -> Result<(), StoreError> {
    conn.prepare_cached("DELETE FROM database_property WHERE database_id = ?1")?
        .execute([id])?;
    conn.prepare_cached("DELETE FROM database WHERE id = ?1")?
        .execute([id])?;
    Ok(())
}
