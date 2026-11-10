//! The record transactions behind each table edit.

use scroll_core::id::{PageId, PropertyId};
use scroll_core::operation::{
    DatabaseField, FieldChange, Operation, Record, RecordId, Transaction, ViewField,
};
use scroll_core::position::{Position, PositionError};
use scroll_core::record::{
    ColumnWidth, Database, FilterCondition, Page, Property, PropertyType, SortKey, View,
};
use scroll_core::validation::check_condition;

/// A new, empty row placed after `last_position`. Rows are children of the database's host page.
/// Fails only if `last_position` is malformed.
pub fn new_row(
    database: &Database,
    last_position: Option<&Position>,
) -> Result<(Transaction, Page), PositionError> {
    let position = Position::between(last_position, None)?;
    let page = Page {
        id: PageId::generate(),
        parent: Some(database.host_page),
        title: String::new(),
        icon: None,
        position,
        row_of: Some(database.id),
        is_archived: false,
    };
    let transaction = Transaction::new(vec![Operation::Create(Record::Page(page.clone()))]);
    Ok((transaction, page))
}

pub fn delete_rows(pages: &[PageId]) -> Transaction {
    Transaction::new(
        pages
            .iter()
            .map(|page| Operation::Delete(RecordId::Page(*page)))
            .collect(),
    )
}

pub fn new_property(name: &str, property_type: PropertyType) -> Property {
    Property {
        id: PropertyId::generate(),
        name: name.into(),
        property_type,
        options: Vec::new(),
    }
}

/// Adds the property to the schema and shows it as the view's last column.
pub fn add_property(view: &View, property: Property) -> Transaction {
    let mut columns = view.visible_columns.clone();
    columns.push(property.id);
    Transaction::new(vec![
        Operation::Set(FieldChange::Database {
            id: view.database,
            field: DatabaseField::Property(property),
        }),
        set_view_field(view, ViewField::VisibleColumns(columns)),
    ])
}

/// Changes the property's type, dropping this view's filter conditions that no longer fit it.
/// Existing values of the old type stay stored and read as empty.
pub fn change_property_type(
    view: &View,
    property: &Property,
    property_type: PropertyType,
) -> Transaction {
    let retyped = Property {
        property_type,
        ..property.clone()
    };
    let filter: Vec<FilterCondition> = view
        .filter
        .iter()
        .filter(|condition| {
            condition.property != property.id
                || check_condition(&retyped, &condition.condition).is_ok()
        })
        .cloned()
        .collect();
    Transaction::new(vec![
        Operation::Set(FieldChange::Database {
            id: view.database,
            field: DatabaseField::Property(retyped),
        }),
        set_view_field(view, ViewField::Filter(filter)),
    ])
}

/// Removes the property from the schema and from this view's columns, widths, sort and filter, so the
/// view never names a property the database no longer has.
pub fn delete_property(view: &View, property: PropertyId) -> Transaction {
    let columns: Vec<PropertyId> = view
        .visible_columns
        .iter()
        .copied()
        .filter(|column| *column != property)
        .collect();
    let sort: Vec<SortKey> = view
        .sort
        .iter()
        .copied()
        .filter(|key| key.property != property)
        .collect();
    let filter: Vec<FilterCondition> = view
        .filter
        .iter()
        .filter(|condition| condition.property != property)
        .cloned()
        .collect();
    let widths: Vec<ColumnWidth> = view
        .column_widths
        .iter()
        .copied()
        .filter(|width| width.property != property)
        .collect();
    Transaction::new(vec![
        set_view_field(view, ViewField::VisibleColumns(columns)),
        set_view_field(view, ViewField::Sort(sort)),
        set_view_field(view, ViewField::Filter(filter)),
        set_view_field(view, ViewField::ColumnWidths(widths)),
        Operation::Set(FieldChange::Database {
            id: view.database,
            field: DatabaseField::RemovedProperty(property),
        }),
    ])
}

pub fn set_visible_columns(view: &View, columns: Vec<PropertyId>) -> Transaction {
    Transaction::new(vec![set_view_field(
        view,
        ViewField::VisibleColumns(columns),
    )])
}

pub fn set_sort(view: &View, sort: Vec<SortKey>) -> Transaction {
    Transaction::new(vec![set_view_field(view, ViewField::Sort(sort))])
}

pub fn set_filter(view: &View, filter: Vec<FilterCondition>) -> Transaction {
    Transaction::new(vec![set_view_field(view, ViewField::Filter(filter))])
}

pub fn set_column_widths(view: &View, widths: Vec<ColumnWidth>) -> Transaction {
    Transaction::new(vec![set_view_field(view, ViewField::ColumnWidths(widths))])
}

fn set_view_field(view: &View, field: ViewField) -> Operation {
    Operation::Set(FieldChange::View { id: view.id, field })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::with_new_option;
    use scroll_core::id::DatabaseId;
    use scroll_core::id::ViewId;
    use scroll_core::record::{Condition, SortDirection, ViewType};
    use scroll_core::store::WorkspaceStore;

    struct Fixture {
        store: WorkspaceStore,
        database: Database,
        view: View,
        status: Property,
    }

    fn fixture() -> Fixture {
        let mut store = WorkspaceStore::open(std::path::Path::new(":memory:")).unwrap();
        let host = Page {
            id: PageId::generate(),
            parent: None,
            title: "Tasks".into(),
            icon: None,
            position: Position::between(None, None).unwrap(),
            row_of: None,
            is_archived: false,
        };
        let status = new_property("Status", PropertyType::Text);
        let database = Database {
            id: DatabaseId::generate(),
            host_page: host.id,
            properties: vec![status.clone()],
        };
        let view = View {
            id: ViewId::generate(),
            database: database.id,
            name: "All".into(),
            view_type: ViewType::Table,
            filter: vec![FilterCondition {
                property: status.id,
                condition: Condition::IsNotEmpty,
            }],
            sort: vec![SortKey {
                property: status.id,
                direction: SortDirection::Ascending,
            }],
            visible_columns: vec![status.id],
            column_widths: Vec::new(),
        };
        store
            .apply_local_transaction(&Transaction::new(vec![
                Operation::Create(Record::Page(host)),
                Operation::Create(Record::Database(database.clone())),
                Operation::Create(Record::View(view.clone())),
            ]))
            .unwrap();
        Fixture {
            store,
            database,
            view,
            status,
        }
    }

    /// Every row of the fixture's database, through an unfiltered copy of its view.
    fn row_count(fixture: &Fixture) -> usize {
        let every_row = View {
            filter: Vec::new(),
            ..fixture.view.clone()
        };
        fixture.store.rows_for_view(every_row).unwrap().rows.len()
    }

    #[test]
    fn rows_are_created_after_the_last_and_deleted_together() {
        let mut fixture = fixture();
        let (first, first_page) = new_row(&fixture.database, None).unwrap();
        let (second, second_page) = new_row(&fixture.database, Some(&first_page.position)).unwrap();
        assert!(second_page.position > first_page.position);
        fixture.store.apply_local_transaction(&first).unwrap();
        fixture.store.apply_local_transaction(&second).unwrap();
        assert_eq!(row_count(&fixture), 2);

        let delete = delete_rows(&[first_page.id, second_page.id]);
        fixture.store.apply_local_transaction(&delete).unwrap();
        assert_eq!(row_count(&fixture), 0);
    }

    #[test]
    fn adding_a_property_shows_it_and_deleting_it_cleans_the_view() {
        let mut fixture = fixture();
        let points = new_property("Points", PropertyType::Number);
        fixture
            .store
            .apply_local_transaction(&add_property(&fixture.view, points.clone()))
            .unwrap();
        let view = &fixture.store.views(fixture.database.id).unwrap()[0];
        assert_eq!(view.visible_columns, vec![fixture.status.id, points.id]);

        fixture
            .store
            .apply_local_transaction(&delete_property(view, fixture.status.id))
            .unwrap();
        let view = &fixture.store.views(fixture.database.id).unwrap()[0];
        assert_eq!(view.visible_columns, vec![points.id]);
        assert!(view.sort.is_empty() && view.filter.is_empty());
        let schema = fixture
            .store
            .database(fixture.database.id)
            .unwrap()
            .unwrap();
        assert_eq!(schema.properties, vec![points]);
    }

    #[test]
    fn a_new_option_is_saved_with_the_property() {
        let mut fixture = fixture();
        let select = Property {
            property_type: PropertyType::Select,
            ..fixture.status.clone()
        };
        let (updated, option) = with_new_option(&select, "  Doing ");
        fixture
            .store
            .apply_local_transaction(&Transaction::set_property(fixture.database.id, updated))
            .unwrap();
        let schema = fixture
            .store
            .database(fixture.database.id)
            .unwrap()
            .unwrap();
        assert_eq!(schema.properties[0].property_type, PropertyType::Select);
        assert_eq!(schema.properties[0].options[0].id, option);
        assert_eq!(schema.properties[0].options[0].name, "Doing");
    }

    #[test]
    fn changing_a_type_drops_filter_conditions_that_no_longer_fit() {
        let mut fixture = fixture();
        let contains = FilterCondition {
            property: fixture.status.id,
            condition: Condition::TextContains("x".into()),
        };
        let view = View {
            filter: vec![fixture.view.filter[0].clone(), contains],
            ..fixture.view.clone()
        };
        fixture
            .store
            .apply_local_transaction(&set_filter(&view, view.filter.clone()))
            .unwrap();
        fixture
            .store
            .apply_local_transaction(&change_property_type(
                &view,
                &fixture.status,
                PropertyType::Number,
            ))
            .unwrap();
        let saved = &fixture.store.views(fixture.database.id).unwrap()[0];
        assert_eq!(saved.filter, vec![fixture.view.filter[0].clone()]);
    }
}
