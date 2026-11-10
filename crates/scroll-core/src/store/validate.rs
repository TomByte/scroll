//! Runs the checks listed in `validation` against the local store: looks up what each check needs.
//! The server does the same lookups in Postgres; keep the two in step with that table.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::id::{DatabaseId, PageId, TagId};
use crate::operation::{
    DatabaseField, FieldChange, Operation, PageField, Record, RecordId, TagField,
};
use crate::record::{Database, Page};
use crate::store::StoreError;
use crate::store::kinds::database::{database_hosted_by, read_database, row_count};
use crate::store::kinds::page::{child_page_count, page_lineage, read_page};
use crate::store::kinds::record_exists;
use crate::store::kinds::tag::tag_holding_name;
use crate::store::kinds::view::{read_view, view_count};
use crate::validation::{
    ValidationError, check_database, check_database_has_no_rows, check_database_has_no_views,
    check_host_page_is_free, check_new_parent, check_page_has_no_children,
    check_page_hosts_no_database, check_position, check_property, check_property_value,
    check_row_of, check_tag_name, check_tag_name_is_free, check_view, check_view_field,
    find_property,
};

/// Validates the operations of one transaction in order. Database schemas are read once per
/// transaction and cached, so a bulk insert of rows does not decode the schema for every value.
pub(in crate::store) struct Validator<'conn> {
    conn: &'conn Connection,
    schemas: HashMap<DatabaseId, Database>,
}

impl<'conn> Validator<'conn> {
    pub(in crate::store) fn new(conn: &'conn Connection) -> Self {
        Validator {
            conn,
            schemas: HashMap::new(),
        }
    }

    pub(in crate::store) fn validate(&mut self, operation: &Operation) -> Result<(), StoreError> {
        match operation {
            Operation::Create(record) => self.validate_create(record),
            Operation::Set(change) => self.validate_field_change(change),
            Operation::Delete(id) => self.validate_delete(*id),
        }
    }

    /// Call after applying `operation`, so the next operation sees any schema it changed.
    pub(in crate::store) fn forget_schema_changed_by(&mut self, operation: &Operation) {
        if let RecordId::Database(id) = operation.record_id() {
            self.schemas.remove(&id);
        }
    }

    fn validate_create(&mut self, record: &Record) -> Result<(), StoreError> {
        if let Record::PageTag(membership) = record {
            self.require_page(membership.page)?;
            return self.require_record(RecordId::Tag(membership.tag));
        }
        if record_exists(self.conn, record.id())? {
            return Err(ValidationError::RecordAlreadyExists(record.id()).into());
        }
        match record {
            Record::Page(page) => self.validate_new_page(page),
            Record::Database(database) => {
                self.require_page(database.host_page)?;
                check_host_page_is_free(
                    database.host_page,
                    database_hosted_by(self.conn, database.host_page)?,
                )?;
                Ok(check_database(database)?)
            }
            Record::View(view) => {
                let database = self.require_database(view.database)?;
                Ok(check_view(view, database)?)
            }
            Record::Tag(tag) => self.validate_tag_name(tag.id, &tag.name),
            Record::PageTag(_) => Ok(()),
        }
    }

    fn validate_tag_name(&self, tag: TagId, name: &str) -> Result<(), StoreError> {
        check_tag_name(name)?;
        Ok(check_tag_name_is_free(
            tag,
            name,
            tag_holding_name(self.conn, name)?,
        )?)
    }

    fn validate_new_page(&mut self, page: &Page) -> Result<(), StoreError> {
        check_position(&page.position)?;
        if let Some(parent) = page.parent {
            self.require_page(parent)?;
        }
        if let Some(database) = page.row_of {
            self.require_database(database)?;
        }
        Ok(())
    }

    fn validate_field_change(&mut self, change: &FieldChange) -> Result<(), StoreError> {
        match change {
            FieldChange::Page { id, field } => {
                self.require_page(*id)?;
                self.validate_page_field(*id, field)
            }
            FieldChange::Database { id, field } => {
                let database = self.require_database(*id)?;
                match field {
                    DatabaseField::Property(property) => check_property(property)?,
                    DatabaseField::RemovedProperty(property) => {
                        find_property(database, *property)?;
                    }
                }
                Ok(())
            }
            FieldChange::PropertyValue {
                page,
                property,
                value,
            } => {
                let page = self.require_page(*page)?;
                let database_id = page
                    .row_of
                    .ok_or(ValidationError::PageIsNotADatabaseRow(page.id))?;
                let database = self.require_database(database_id)?;
                let property = find_property(database, *property)?;
                if let Some(value) = value {
                    check_property_value(property, value)?;
                }
                Ok(())
            }
            FieldChange::View { id, field } => {
                let view = read_view(self.conn, *id)?.ok_or(ValidationError::ViewNotFound(*id))?;
                let database = self.require_database(view.database)?;
                Ok(check_view_field(field, database)?)
            }
            FieldChange::Tag { id, field } => {
                self.require_record(RecordId::Tag(*id))?;
                match field {
                    TagField::Name(name) => self.validate_tag_name(*id, name),
                    TagField::Color(_) => Ok(()),
                }
            }
        }
    }

    fn validate_page_field(&mut self, id: PageId, field: &PageField) -> Result<(), StoreError> {
        match field {
            PageField::Parent(Some(parent)) => {
                self.require_page(*parent)?;
                Ok(check_new_parent(id, &page_lineage(self.conn, *parent)?)?)
            }
            PageField::Position(position) => Ok(check_position(position)?),
            PageField::RowOf(Some(database)) => {
                let database = self.require_database(*database)?;
                Ok(check_row_of(id, database)?)
            }
            _ => Ok(()),
        }
    }

    fn validate_delete(&mut self, id: RecordId) -> Result<(), StoreError> {
        if let RecordId::PageTag { .. } = id {
            return Ok(());
        }
        self.require_record(id)?;
        let conn = self.conn;
        match id {
            RecordId::Page(page) => {
                check_page_has_no_children(page, child_page_count(conn, page)?)?;
                Ok(check_page_hosts_no_database(
                    page,
                    database_hosted_by(conn, page)?,
                )?)
            }
            RecordId::Database(database) => {
                check_database_has_no_rows(database, row_count(conn, database)?)?;
                Ok(check_database_has_no_views(
                    database,
                    view_count(conn, database)?,
                )?)
            }
            RecordId::View(_) | RecordId::Tag(_) | RecordId::PageTag { .. } => Ok(()),
        }
    }

    fn require_page(&self, id: PageId) -> Result<Page, StoreError> {
        Ok(read_page(self.conn, id)?.ok_or(ValidationError::PageNotFound(id))?)
    }

    fn require_database(&mut self, id: DatabaseId) -> Result<&Database, StoreError> {
        if !self.schemas.contains_key(&id) {
            let database =
                read_database(self.conn, id)?.ok_or(ValidationError::DatabaseNotFound(id))?;
            self.schemas.insert(id, database);
        }
        Ok(&self.schemas[&id])
    }

    fn require_record(&self, id: RecordId) -> Result<(), StoreError> {
        if record_exists(self.conn, id)? {
            return Ok(());
        }
        let error = match id {
            RecordId::Page(id) => ValidationError::PageNotFound(id),
            RecordId::Database(id) => ValidationError::DatabaseNotFound(id),
            RecordId::View(id) => ValidationError::ViewNotFound(id),
            RecordId::Tag(id) => ValidationError::TagNotFound(id),
            RecordId::PageTag { page, .. } => ValidationError::PageNotFound(page),
        };
        Err(error.into())
    }
}
