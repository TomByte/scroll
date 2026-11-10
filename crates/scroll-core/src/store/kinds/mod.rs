//! Everything the store does with each kind of record, one module per kind, and the dispatch
//! from a `Record`, `RecordId` or `FieldChange` to the right one. Writes do no validation and no
//! conflict resolution: `sync::apply` decides whether a write happens. Inserts ignore records
//! that already exist, so replaying a transaction is harmless. Deleting a page leaves its content
//! and tag memberships, and deleting a tag leaves its memberships, because a pending local delete
//! can still be undone by a rejection; `delete_dependants` removes them once the delete is final.

pub(in crate::store) mod database;
pub(in crate::store) mod page;
pub(in crate::store) mod page_tag;
pub(in crate::store) mod tag;
pub(in crate::store) mod value;
pub(in crate::store) mod view;

use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::id::{DatabaseId, PageId, TagId, ViewId};
use crate::operation::{FieldChange, Record, RecordId};
use crate::record::PageTag;
use crate::store::StoreError;

pub(in crate::store) fn insert_record(
    conn: &Connection,
    record: &Record,
) -> Result<(), StoreError> {
    match record {
        Record::Page(page) => page::insert_page(conn, page),
        Record::Database(database) => database::insert_database(conn, database),
        Record::View(view) => view::insert_view(conn, view),
        Record::Tag(tag) => tag::insert_tag(conn, tag),
        Record::PageTag(membership) => page_tag::insert_page_tag(conn, *membership),
    }
}

/// Writes nothing if the record the field belongs to does not exist.
pub(in crate::store) fn write_field(
    conn: &Connection,
    change: &FieldChange,
) -> Result<(), StoreError> {
    match change {
        FieldChange::Page { id, field } => page::write_page_field(conn, *id, field),
        FieldChange::Database { id, field } => database::write_database_field(conn, *id, field),
        FieldChange::PropertyValue {
            page,
            property,
            value,
        } => value::write_property_value(conn, *page, *property, value.as_ref()),
        FieldChange::View { id, field } => view::write_view_field(conn, *id, field),
        FieldChange::Tag { id, field } => tag::write_tag_field(conn, *id, field),
    }
}

pub(in crate::store) fn delete_record(conn: &Connection, id: RecordId) -> Result<(), StoreError> {
    match id {
        RecordId::Page(id) => page::delete_page(conn, id),
        RecordId::Database(id) => database::delete_database(conn, id),
        RecordId::View(id) => view::delete_view(conn, id),
        RecordId::Tag(id) => tag::delete_tag(conn, id),
        RecordId::PageTag { page, tag } => page_tag::delete_page_tag(conn, PageTag { page, tag }),
    }
}

pub(in crate::store) fn record_exists(conn: &Connection, id: RecordId) -> Result<bool, StoreError> {
    match id {
        RecordId::Page(id) => page::page_exists(conn, id),
        RecordId::Database(id) => database::database_exists(conn, id),
        RecordId::View(id) => view::view_exists(conn, id),
        RecordId::Tag(id) => tag::tag_exists(conn, id),
        RecordId::PageTag { page, tag } => page_tag::page_has_tag(conn, PageTag { page, tag }),
    }
}

/// What a record the server no longer has leaves behind: a page's content and tag memberships,
/// and a tag's memberships. Kept until then, so a rejected local delete can bring them back.
pub(in crate::store) fn delete_dependants(
    conn: &Connection,
    id: RecordId,
) -> Result<(), StoreError> {
    match id {
        RecordId::Page(id) => page::delete_dependants(conn, id),
        RecordId::Tag(id) => page_tag::delete_memberships_of_tag(conn, id),
        RecordId::Database(_) | RecordId::View(_) | RecordId::PageTag { .. } => Ok(()),
    }
}

/// The kinds of record. `KINDS` lists them in the order they are applied: each after the kinds
/// it refers to (a view after its database, a membership after its page and tag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum Kind {
    Page,
    Database,
    View,
    Tag,
    PageTag,
}

pub(in crate::store) const KINDS: [Kind; 5] = [
    Kind::Page,
    Kind::Database,
    Kind::View,
    Kind::Tag,
    Kind::PageTag,
];

impl Kind {
    pub(in crate::store) fn of(id: RecordId) -> Kind {
        match id {
            RecordId::Page(_) => Kind::Page,
            RecordId::Database(_) => Kind::Database,
            RecordId::View(_) => Kind::View,
            RecordId::Tag(_) => Kind::Tag,
            RecordId::PageTag { .. } => Kind::PageTag,
        }
    }

    /// The kind's number in `staged_record.kind`. Stored, so a number never changes; staged
    /// records settle in number order, so numbers follow `KINDS`.
    fn staging_number(self) -> u8 {
        match self {
            Kind::Page => 0,
            Kind::Database => 1,
            Kind::View => 2,
            Kind::Tag => 3,
            Kind::PageTag => 4,
        }
    }

    pub(in crate::store) fn all_records(
        self,
        conn: &Connection,
    ) -> Result<Vec<Record>, StoreError> {
        Ok(match self {
            Kind::Page => page::all_pages(conn)?
                .into_iter()
                .map(Record::Page)
                .collect(),
            Kind::Database => database::all_databases(conn)?
                .into_iter()
                .map(Record::Database)
                .collect(),
            Kind::View => view::all_views(conn)?
                .into_iter()
                .map(Record::View)
                .collect(),
            Kind::Tag => tag::all_tags(conn)?.into_iter().map(Record::Tag).collect(),
            Kind::PageTag => page_tag::all_page_tags(conn)?
                .into_iter()
                .map(Record::PageTag)
                .collect(),
        })
    }

    /// The kind's local records that the staged snapshot does not have.
    fn records_not_staged(self, conn: &Connection) -> Result<Vec<RecordId>, StoreError> {
        let number = self.staging_number();
        Ok(match self {
            Kind::Page => ids_not_staged(conn, "page", number)?
                .into_iter()
                .map(|id| RecordId::Page(PageId(id)))
                .collect(),
            Kind::Database => ids_not_staged(conn, "database", number)?
                .into_iter()
                .map(|id| RecordId::Database(DatabaseId(id)))
                .collect(),
            Kind::View => ids_not_staged(conn, "view", number)?
                .into_iter()
                .map(|id| RecordId::View(ViewId(id)))
                .collect(),
            Kind::Tag => ids_not_staged(conn, "tag", number)?
                .into_iter()
                .map(|id| RecordId::Tag(TagId(id)))
                .collect(),
            Kind::PageTag => page_tag::memberships_not_staged(conn, number)?,
        })
    }
}

/// Every record, kind by kind in `KINDS` order, so each comes after what it refers to.
pub(in crate::store) fn all_records(conn: &Connection) -> Result<Vec<Record>, StoreError> {
    let mut records = Vec::new();
    for kind in KINDS {
        records.extend(kind.all_records(conn)?);
    }
    Ok(records)
}

/// Where a record is kept while a snapshot is staged: its kind's number and its one or two keys.
pub(in crate::store) fn staging_key(id: RecordId) -> (u8, Uuid, Uuid) {
    let number = Kind::of(id).staging_number();
    match id {
        RecordId::Page(id) => (number, id.0, Uuid::nil()),
        RecordId::Database(id) => (number, id.0, Uuid::nil()),
        RecordId::View(id) => (number, id.0, Uuid::nil()),
        RecordId::Tag(id) => (number, id.0, Uuid::nil()),
        RecordId::PageTag { page, tag } => (number, page.0, tag.0),
    }
}

/// Local records, membership rows included, that the staged snapshot does not have.
pub(in crate::store) fn records_missing_from_staging(
    conn: &Connection,
) -> Result<Vec<RecordId>, StoreError> {
    let mut missing = Vec::new();
    for kind in KINDS {
        missing.extend(kind.records_not_staged(conn)?);
    }
    Ok(missing)
}

/// The ids in `table` (keyed by `id`) with no staged record numbered `kind`. `table` is always
/// a literal from `Kind::records_not_staged`.
fn ids_not_staged(conn: &Connection, table: &str, kind: u8) -> Result<Vec<Uuid>, StoreError> {
    let sql = format!(
        "SELECT id FROM {table} WHERE NOT EXISTS (SELECT 1 FROM staged_record
         WHERE kind = ?1 AND key = {table}.id AND second_key = ?2)"
    );
    let mut statement = conn.prepare(&sql)?;
    let ids = statement.query_map(params![kind, Uuid::nil()], |row| row.get(0))?;
    Ok(ids.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_listed_once_and_numbered_in_apply_order() {
        let numbers: Vec<u8> = KINDS.iter().map(|kind| kind.staging_number()).collect();
        assert_eq!(numbers, [0, 1, 2, 3, 4]);
        let page = PageId::generate();
        let tag = TagId::generate();
        let one_of_each = [
            RecordId::Page(page),
            RecordId::Database(DatabaseId::generate()),
            RecordId::View(ViewId::generate()),
            RecordId::Tag(tag),
            RecordId::PageTag { page, tag },
        ];
        for id in one_of_each {
            assert!(KINDS.contains(&Kind::of(id)), "{id:?}");
        }
    }
}
