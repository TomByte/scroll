//! Record changes: operations grouped into transactions that commit all or nothing.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::id::{DatabaseId, PageId, PropertyId, TagId, TransactionId, ViewId};
use crate::record::{Database, Page, PageTag, Property, PropertyValue, Tag, View};
pub use crate::record::{DatabaseField, PageField, TagField, ViewField};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    /// Chosen by the client, so a resent transaction can be recognised and the acknowledgement matched.
    pub id: TransactionId,
    pub operations: Vec<Operation>,
}

impl Transaction {
    pub fn new(operations: Vec<Operation>) -> Self {
        Self {
            id: TransactionId::generate(),
            operations,
        }
    }

    pub fn set_title(page: PageId, title: &str) -> Self {
        Self::new(vec![Operation::set_title(page, title)])
    }

    /// `None` clears the value.
    pub fn set_value(page: PageId, property: PropertyId, value: Option<PropertyValue>) -> Self {
        Self::new(vec![Operation::set_value(page, property, value)])
    }

    /// Adds the property to the database's schema, or replaces the one with the same id (a
    /// rename, a retype, a new option).
    pub fn set_property(database: DatabaseId, property: Property) -> Self {
        Self::new(vec![Operation::Set(FieldChange::Database {
            id: database,
            field: DatabaseField::Property(property),
        })])
    }

    pub fn set_tag_field(tag: TagId, field: TagField) -> Self {
        Self::new(vec![Operation::Set(FieldChange::Tag { id: tag, field })])
    }

    pub fn rename_view(view: ViewId, name: &str) -> Self {
        Self::new(vec![Operation::Set(FieldChange::View {
            id: view,
            field: ViewField::Name(name.to_string()),
        })])
    }

    /// Deletes one record, e.g. a view, a tag, or a page's membership of a tag.
    pub fn delete(record: RecordId) -> Self {
        Self::new(vec![Operation::Delete(record)])
    }

    /// Each record the transaction touches, once, in first-touched order.
    pub(crate) fn records_touched(&self) -> Vec<RecordId> {
        let mut seen = HashSet::new();
        self.operations
            .iter()
            .map(Operation::record_id)
            .filter(|id| seen.insert(*id))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Create(Record),
    Set(FieldChange),
    Delete(RecordId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Record {
    Page(Page),
    Database(Database),
    View(View),
    Tag(Tag),
    /// Creating a membership the page already has changes nothing.
    PageTag(PageTag),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordId {
    Page(PageId),
    Database(DatabaseId),
    View(ViewId),
    Tag(TagId),
    /// Deleting a membership the page does not have changes nothing.
    PageTag {
        page: PageId,
        tag: TagId,
    },
}

/// One field of one record. Each is the unit of last-write-wins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldChange {
    Page {
        id: PageId,
        field: PageField,
    },
    Database {
        id: DatabaseId,
        field: DatabaseField,
    },
    /// `None` clears the value. Property values have no create or delete of their own.
    PropertyValue {
        page: PageId,
        property: PropertyId,
        value: Option<PropertyValue>,
    },
    View {
        id: ViewId,
        field: ViewField,
    },
    Tag {
        id: TagId,
        field: TagField,
    },
}

impl Record {
    pub fn id(&self) -> RecordId {
        match self {
            Record::Page(page) => RecordId::Page(page.id),
            Record::Database(database) => RecordId::Database(database.id),
            Record::View(view) => RecordId::View(view.id),
            Record::Tag(tag) => RecordId::Tag(tag.id),
            Record::PageTag(membership) => RecordId::PageTag {
                page: membership.page,
                tag: membership.tag,
            },
        }
    }
}

impl Operation {
    /// For edits that change several pages in one transaction, such as a paste.
    pub fn set_title(page: PageId, title: &str) -> Self {
        Operation::Set(FieldChange::Page {
            id: page,
            field: PageField::Title(title.to_string()),
        })
    }

    /// For edits that change several cells in one transaction, such as a paste. `None` clears
    /// the value.
    pub fn set_value(page: PageId, property: PropertyId, value: Option<PropertyValue>) -> Self {
        Operation::Set(FieldChange::PropertyValue {
            page,
            property,
            value,
        })
    }

    /// The record the operation changes. A property value belongs to its page.
    pub(crate) fn record_id(&self) -> RecordId {
        match self {
            Operation::Create(record) => record.id(),
            Operation::Set(change) => change.record_id(),
            Operation::Delete(id) => *id,
        }
    }
}

impl FieldChange {
    /// The record that must exist for the change to mean anything. A property value belongs to its page.
    pub(crate) fn record_id(&self) -> RecordId {
        match self {
            FieldChange::Page { id, .. } => RecordId::Page(*id),
            FieldChange::Database { id, .. } => RecordId::Database(*id),
            FieldChange::PropertyValue { page, .. } => RecordId::Page(*page),
            FieldChange::View { id, .. } => RecordId::View(*id),
            FieldChange::Tag { id, .. } => RecordId::Tag(*id),
        }
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    #[test]
    fn builders_make_one_operation_on_the_named_record() {
        let page = PageId::generate();
        let property = PropertyId::generate();
        let tag = TagId::generate();
        let view = ViewId::generate();
        let only = |transaction: Transaction| {
            assert_eq!(transaction.operations.len(), 1);
            transaction.operations.into_iter().next().unwrap()
        };
        assert_eq!(
            only(Transaction::set_title(page, "Plan")),
            Operation::Set(FieldChange::Page {
                id: page,
                field: PageField::Title("Plan".into()),
            })
        );
        assert_eq!(
            only(Transaction::set_value(page, property, None)),
            Operation::Set(FieldChange::PropertyValue {
                page,
                property,
                value: None,
            })
        );
        assert_eq!(
            only(Transaction::set_tag_field(
                tag,
                TagField::Name("Urgent".into())
            )),
            Operation::Set(FieldChange::Tag {
                id: tag,
                field: TagField::Name("Urgent".into()),
            })
        );
        assert_eq!(
            only(Transaction::rename_view(view, "Board")),
            Operation::Set(FieldChange::View {
                id: view,
                field: ViewField::Name("Board".into()),
            })
        );
        let database = DatabaseId::generate();
        let property = Property {
            id: property,
            name: "Status".into(),
            property_type: crate::record::PropertyType::Select,
            options: Vec::new(),
        };
        assert_eq!(
            only(Transaction::set_property(database, property.clone())),
            Operation::Set(FieldChange::Database {
                id: database,
                field: DatabaseField::Property(property),
            })
        );
        let membership = RecordId::PageTag { page, tag };
        assert_eq!(
            only(Transaction::delete(membership)),
            Operation::Delete(membership)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SelectOptionId;
    use crate::position::Position;
    use crate::record::{Condition, Date, PropertyType, SelectOption, SortDirection};
    use crate::record::{FilterCondition, Property, SortKey, TagColor, ViewType};

    #[test]
    fn transactions_round_trip_through_postcard() {
        let page = PageId::generate();
        let property = PropertyId::generate();
        let option = SelectOptionId::generate();
        let tag = TagId::generate();
        let database = Database {
            id: DatabaseId::generate(),
            host_page: page,
            properties: vec![Property {
                id: property,
                name: "Status".into(),
                property_type: PropertyType::MultiSelect,
                options: vec![SelectOption {
                    id: option,
                    name: "Done".into(),
                }],
            }],
        };
        let view = View {
            id: ViewId::generate(),
            database: database.id,
            name: "All".into(),
            view_type: ViewType::Table,
            filter: vec![FilterCondition {
                property,
                condition: Condition::DateAfter(Date {
                    year: 2026,
                    month: 1,
                    day: 2,
                }),
            }],
            sort: vec![SortKey {
                property,
                direction: SortDirection::Descending,
            }],
            visible_columns: vec![property],
            column_widths: Vec::new(),
        };
        let transaction = Transaction::new(vec![
            Operation::Create(Record::Database(database.clone())),
            Operation::Create(Record::View(view.clone())),
            Operation::Set(FieldChange::Page {
                id: page,
                field: PageField::Icon(None),
            }),
            Operation::Set(FieldChange::Page {
                id: page,
                field: PageField::Position(Position::between(None, None).unwrap()),
            }),
            Operation::Set(FieldChange::PropertyValue {
                page,
                property,
                value: Some(PropertyValue::MultiSelect(vec![option])),
            }),
            Operation::Set(FieldChange::PropertyValue {
                page,
                property,
                value: Some(PropertyValue::Number(-1.5)),
            }),
            Operation::Set(FieldChange::Database {
                id: database.id,
                field: DatabaseField::RemovedProperty(property),
            }),
            Operation::Set(FieldChange::View {
                id: view.id,
                field: ViewField::Filter(vec![]),
            }),
            Operation::Delete(RecordId::Page(page)),
            Operation::Create(Record::Tag(Tag {
                id: tag,
                name: "Urgent".into(),
                color: TagColor::Red,
            })),
            Operation::Set(FieldChange::Tag {
                id: tag,
                field: TagField::Color(TagColor::Blue),
            }),
            Operation::Create(Record::PageTag(PageTag { page, tag })),
            Operation::Delete(RecordId::PageTag { page, tag }),
        ]);

        let bytes = postcard::to_stdvec(&transaction).unwrap();
        assert_eq!(
            postcard::from_bytes::<Transaction>(&bytes).unwrap(),
            transaction
        );
    }
}

#[cfg(test)]
mod encoding_tests {
    //! Transactions are stored and logged by the server and queued by clients, so their encoding
    //! must not change: new shapes are added as new enum variants at the end.

    use uuid::Uuid;

    use super::*;
    use crate::id::SelectOptionId;
    use crate::position::Position;
    use crate::record::{
        ColumnWidth, Condition, FilterCondition, Property, PropertyType, SelectOption,
        SortDirection, SortKey, ViewType,
    };

    /// Built only from shapes that existed before tags.
    fn transaction_with_every_older_shape() -> Transaction {
        let page = PageId(Uuid::from_u128(1));
        let database = DatabaseId(Uuid::from_u128(2));
        let property = PropertyId(Uuid::from_u128(3));
        let option = SelectOptionId(Uuid::from_u128(4));
        let view = ViewId(Uuid::from_u128(5));
        Transaction {
            id: TransactionId(Uuid::from_u128(6)),
            operations: vec![
                Operation::Create(Record::Page(Page {
                    id: page,
                    parent: None,
                    title: "T".into(),
                    icon: Some("i".into()),
                    position: Position::between(None, None).unwrap(),
                    row_of: Some(database),
                    is_archived: false,
                })),
                Operation::Create(Record::Database(Database {
                    id: database,
                    host_page: page,
                    properties: vec![Property {
                        id: property,
                        name: "P".into(),
                        property_type: PropertyType::Select,
                        options: vec![SelectOption {
                            id: option,
                            name: "O".into(),
                        }],
                    }],
                })),
                Operation::Create(Record::View(View {
                    id: view,
                    database,
                    name: "V".into(),
                    view_type: ViewType::Table,
                    filter: vec![FilterCondition {
                        property,
                        condition: Condition::SelectIs(option),
                    }],
                    sort: vec![SortKey {
                        property,
                        direction: SortDirection::Descending,
                    }],
                    visible_columns: vec![property],
                    column_widths: vec![ColumnWidth {
                        property,
                        width: 120,
                    }],
                })),
                Operation::Set(FieldChange::PropertyValue {
                    page,
                    property,
                    value: Some(PropertyValue::Number(1.5)),
                }),
                Operation::Set(FieldChange::View {
                    id: view,
                    field: ViewField::ColumnWidths(vec![]),
                }),
                Operation::Delete(RecordId::View(view)),
            ],
        }
    }

    #[test]
    fn the_encoding_of_existing_shapes_is_unchanged() {
        let bytes = postcard::to_stdvec(&transaction_with_every_older_shape()).unwrap();
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, ENCODED);
    }

    /// Recorded when tags were added; do not update it to make the test pass.
    const ENCODED: &str = concat!(
        "10000000000000000000000000000000060600001000000000000000000000000000000001000154",
        "01016902383001100000000000000000000000000000000200000110000000000000000000000000",
        "00000002100000000000000000000000000000000101100000000000000000000000000000000301",
        "5002011000000000000000000000000000000004014f000210000000000000000000000000000000",
        "05100000000000000000000000000000000201560001100000000000000000000000000000000307",
        "10000000000000000000000000000000040110000000000000000000000000000000030101100000",
        "00000000000000000000000000030110000000000000000000000000000000037801021000000000",
        "00000000000000000000000110000000000000000000000000000000030101000000000000f83f01",
        "031000000000000000000000000000000005050002021000000000000000000000000000000005",
    );
}
