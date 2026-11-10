//! Rules a record transaction must satisfy. The checks here are pure: the caller looks up the records
//! they need (from SQLite in the app, from Postgres on the server) and passes them in.
//!
//! Operations in a transaction are checked one at a time, each against the state left by the
//! operations before it in the same transaction. Any failure rejects the whole transaction.
//! "X exists" means: look X up and fail with the matching `*NotFound` error if absent.
//! These are the checks, per operation; both the app and the server must run all of them:
//!
//! | Operation | Checks |
//! |---|---|
//! | Create page | id is new (`RecordAlreadyExists`); `check_position`; parent exists, if set; `row_of` database exists, if set |
//! | Create database | id is new; host page exists; `check_host_page_is_free(host, database hosted there)`; `check_database` |
//! | Create view | id is new; its database exists; `check_view(view, database)` |
//! | Set page `Parent(Some(p))` | page exists; `p` exists; `check_new_parent(page, lineage of p)` (p, then its ancestors) |
//! | Set page `Position` | page exists; `check_position` |
//! | Set page `RowOf(Some(d))` | page exists; `d` exists; `check_row_of(page, d)` |
//! | Set page, any other field | page exists |
//! | Set database `Property` | database exists; `check_property` |
//! | Set database `RemovedProperty` | database exists; `find_property` |
//! | Set property value | page exists; its `row_of` is set (`PageIsNotADatabaseRow`); that database exists; `find_property`; `check_property_value` when the value is `Some` |
//! | Set view field | view exists; its database exists; `check_view_field(field, database)` |
//! | Delete page | page exists; `check_page_has_no_children(page, number of pages with it as parent)`; `check_page_hosts_no_database(page, database hosted there)` |
//! | Delete database | database exists; `check_database_has_no_rows(database, number of pages that are its rows)`; `check_database_has_no_views(database, number of its views)` |
//! | Delete view | view exists |
//! | Create tag | id is new; `check_tag_name`; no tag has the same `tag_name_key` (`check_tag_name_is_free`) |
//! | Set tag `Name` | tag exists; `check_tag_name`; `check_tag_name_is_free(tag, tag holding that key)` |
//! | Set tag `Color` | tag exists |
//! | Delete tag | tag exists. Also deletes every membership of the tag |
//! | Create page tag | page exists; tag exists. Not "id is new": if the page already has the tag, nothing changes |
//! | Delete page tag | none: if the page does not have the tag, nothing changes |
//!
//! Deleting a page also deletes its tag memberships, as it does its property values.
//! Tag names are unique, so when two clients create the same name at once, the transaction the
//! server orders second is rejected with `TagNameTaken`, along with any memberships it created.

mod database;
mod page;
mod tag;
mod value;
mod view;

use serde::{Deserialize, Serialize};

use crate::id::{DatabaseId, PageId, PropertyId, SelectOptionId, TagId, ViewId};
use crate::operation::RecordId;
use crate::position::Position;
use crate::record::{Condition, Date, PropertyType};

pub use database::{
    check_database, check_database_has_no_rows, check_database_has_no_views, check_property,
    find_property,
};
pub use page::{
    check_host_page_is_free, check_new_parent, check_page_has_no_children,
    check_page_hosts_no_database, check_position, check_row_of,
};
pub use tag::{check_tag_name, check_tag_name_is_free, tag_name_key};
pub use value::check_property_value;
pub use view::{MAXIMUM_COLUMN_WIDTH, check_condition, check_view, check_view_field};

/// Serializable, so the server can tell the client which rule a rejected transaction broke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum ValidationError {
    #[error("page {0:?} does not exist")]
    PageNotFound(PageId),
    #[error("database {0:?} does not exist")]
    DatabaseNotFound(DatabaseId),
    #[error("view {0:?} does not exist")]
    ViewNotFound(ViewId),
    #[error("record {0:?} already exists")]
    RecordAlreadyExists(RecordId),
    #[error(
        "page {page:?} cannot move under {parent:?}, which is itself or one of its descendants"
    )]
    PageWouldBeItsOwnAncestor { page: PageId, parent: PageId },
    #[error("page {0:?} still has child pages")]
    PageHasChildren(PageId),
    #[error("page {page:?} hosts database {database:?}; delete the database first")]
    PageHostsDatabase { page: PageId, database: DatabaseId },
    #[error("page {page:?} already hosts database {database:?}")]
    PageAlreadyHostsDatabase { page: PageId, database: DatabaseId },
    #[error("page {page:?} hosts database {database:?}, so it cannot be one of its rows")]
    PageCannotBeRowOfItsOwnDatabase { page: PageId, database: DatabaseId },
    #[error("database {0:?} still has rows")]
    DatabaseHasRows(DatabaseId),
    #[error("database {0:?} still has views")]
    DatabaseHasViews(DatabaseId),
    #[error("page {0:?} is not a row of a database, so it has no property values")]
    PageIsNotADatabaseRow(PageId),
    #[error("database {database:?} has no property {property:?}")]
    PropertyNotFound {
        database: DatabaseId,
        property: PropertyId,
    },
    #[error("database {database:?} lists property {property:?} more than once")]
    DuplicateProperty {
        database: DatabaseId,
        property: PropertyId,
    },
    #[error("property {property:?} holds {expected:?} values, not {actual:?}")]
    PropertyTypeMismatch {
        property: PropertyId,
        expected: PropertyType,
        actual: PropertyType,
    },
    #[error(
        "a filter condition {condition:?} cannot apply to property {property:?} of type {property_type:?}"
    )]
    ConditionDoesNotFitProperty {
        property: PropertyId,
        property_type: PropertyType,
        condition: Condition,
    },
    #[error("property {property:?} has no option {option:?}")]
    SelectOptionNotFound {
        property: PropertyId,
        option: SelectOptionId,
    },
    #[error("property {property:?} lists option {option:?} more than once")]
    DuplicateSelectOption {
        property: PropertyId,
        option: SelectOptionId,
    },
    #[error("{0:?} is not a calendar date")]
    InvalidDate(Date),
    #[error("property {0:?} was given a number that is not finite")]
    NumberNotFinite(PropertyId),
    #[error("position {0:?} is not a well-formed fractional index")]
    MalformedPosition(Position),
    #[error("column {property:?} cannot be {width} points wide")]
    InvalidColumnWidth { property: PropertyId, width: u32 },
    #[error("tag {0:?} does not exist")]
    TagNotFound(TagId),
    #[error("a tag name cannot be empty")]
    TagNameEmpty,
    #[error("tag {existing:?} is already called {name:?}")]
    TagNameTaken { name: String, existing: TagId },
}

#[cfg(test)]
pub(crate) mod test_builders {
    use crate::id::{DatabaseId, PageId, PropertyId, SelectOptionId};
    use crate::record::{Database, Property, PropertyType, SelectOption};

    pub(crate) fn select_property() -> Property {
        Property {
            id: PropertyId::generate(),
            name: "Status".into(),
            property_type: PropertyType::Select,
            options: vec![SelectOption {
                id: SelectOptionId::generate(),
                name: "Done".into(),
            }],
        }
    }

    pub(crate) fn database_with(properties: Vec<Property>) -> Database {
        Database {
            id: DatabaseId::generate(),
            host_page: PageId::generate(),
            properties,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_survive_the_wire() {
        let error = ValidationError::ConditionDoesNotFitProperty {
            property: PropertyId::generate(),
            property_type: PropertyType::Date,
            condition: Condition::DateOn(Date {
                year: 2026,
                month: 11,
                day: 9,
            }),
        };
        let bytes = postcard::to_stdvec(&error).unwrap();
        assert_eq!(
            postcard::from_bytes::<ValidationError>(&bytes).unwrap(),
            error
        );
    }
}
