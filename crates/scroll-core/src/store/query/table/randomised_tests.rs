//! `view_row_ids` against the reference `rows_in_view`, on random databases and views: every
//! filter condition and sort direction, empty values, values whose type the property no longer
//! has, conditions and keys on removed or retyped properties, ties and equal positions.

use crate::id::{DatabaseId, PageId, PropertyId, SelectOptionId, ViewId};
use crate::operation::{DatabaseField, FieldChange, Operation, Record, RecordId, Transaction};
use crate::position::Position;
use crate::record::{
    Condition, Database, Date, FilterCondition, Page, Property, PropertyType, PropertyValue,
    SelectOption, SortDirection, SortKey, View, ViewType,
};
use crate::store::random::Random;
use crate::store::test_support;
use crate::store::{WorkspaceStore, open_in_memory};
use crate::view_query::{TableRow, rows_in_view};

const TYPES: [PropertyType; 7] = [
    PropertyType::Text,
    PropertyType::Number,
    PropertyType::Select,
    PropertyType::MultiSelect,
    PropertyType::Date,
    PropertyType::Checkbox,
    PropertyType::Url,
];

// Small domains, so ties and equal values are common.
const TEXTS: [&str; 8] = ["", "a", "A", "b", "ab", "Ab", "é", "É"];
const NUMBERS: [f64; 6] = [-1.0, -0.0, 0.0, 0.5, 1.0, 2.0];

fn date(random: &mut Random) -> Date {
    Date {
        year: 2025 + random.below(2) as i32,
        month: 1 + random.below(2) as u8,
        day: 1 + random.below(3) as u8,
    }
}

fn value(random: &mut Random, property: &Property) -> PropertyValue {
    let options: Vec<SelectOptionId> = property.options.iter().map(|option| option.id).collect();
    match property.property_type {
        PropertyType::Text => PropertyValue::Text((*random.pick(&TEXTS)).into()),
        PropertyType::Url => PropertyValue::Url((*random.pick(&TEXTS)).into()),
        PropertyType::Number => PropertyValue::Number(*random.pick(&NUMBERS)),
        PropertyType::Select => PropertyValue::Select(*random.pick(&options)),
        PropertyType::MultiSelect => {
            let chosen = (0..random.below(4))
                .map(|_| *random.pick(&options))
                .collect();
            PropertyValue::MultiSelect(chosen)
        }
        PropertyType::Date => PropertyValue::Date(date(random)),
        PropertyType::Checkbox => PropertyValue::Checkbox(random.one_in(2)),
    }
}

fn condition(random: &mut Random, property: &Property) -> Condition {
    if random.one_in(5) {
        return if random.one_in(2) {
            Condition::IsEmpty
        } else {
            Condition::IsNotEmpty
        };
    }
    let option = property.options[random.below(property.options.len())].id;
    match property.property_type {
        PropertyType::Text | PropertyType::Url => {
            let text = (*random.pick(&TEXTS)).to_owned();
            if random.one_in(2) {
                Condition::TextContains(text)
            } else {
                Condition::TextEquals(text)
            }
        }
        PropertyType::Number => {
            let number = *random.pick(&NUMBERS);
            match random.below(3) {
                0 => Condition::NumberEquals(number),
                1 => Condition::NumberGreaterThan(number),
                _ => Condition::NumberLessThan(number),
            }
        }
        PropertyType::Select => Condition::SelectIs(option),
        PropertyType::MultiSelect => Condition::MultiSelectContains(option),
        PropertyType::Date => match random.below(3) {
            0 => Condition::DateBefore(date(random)),
            1 => Condition::DateOn(date(random)),
            _ => Condition::DateAfter(date(random)),
        },
        PropertyType::Checkbox => Condition::CheckboxIs(random.one_in(2)),
    }
}

fn apply(store: &mut WorkspaceStore, operations: Vec<Operation>) {
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
}

fn page(row_of: Option<DatabaseId>, position: Position) -> Page {
    Page {
        id: PageId::generate(),
        parent: None,
        title: "Row".into(),
        icon: None,
        position,
        row_of,
        is_archived: false,
    }
}

/// A database with one property of each type (some twice), rows with random values, and a view
/// with random conditions and keys. Then some properties change type or go away, leaving stale
/// values, conditions and keys behind.
fn random_table(random: &mut Random, store: &mut WorkspaceStore) -> ViewId {
    let properties: Vec<Property> = (0..9)
        .map(|index| Property {
            id: PropertyId::generate(),
            name: format!("P{index}"),
            property_type: TYPES[index % TYPES.len()],
            options: (0..3)
                .map(|option| SelectOption {
                    id: SelectOptionId::generate(),
                    name: format!("O{option}"),
                })
                .collect(),
        })
        .collect();
    let host = page(None, Position::between(None, None).unwrap());
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: properties.clone(),
    };
    let other_database = Database {
        id: DatabaseId::generate(),
        host_page: PageId::generate(),
        properties: properties.clone(),
    };
    let other_host = Page {
        id: other_database.host_page,
        ..page(None, Position::between(None, None).unwrap())
    };
    apply(
        store,
        vec![
            Operation::Create(Record::Page(host)),
            Operation::Create(Record::Page(other_host)),
            Operation::Create(Record::Database(database.clone())),
            Operation::Create(Record::Database(other_database.clone())),
        ],
    );

    // Few distinct positions, so rows tie on position and fall back to id order.
    let positions = Position::many_between(None, None, 5).unwrap();
    let mut operations = Vec::new();
    for _ in 0..40 + random.below(120) {
        let in_other = random.one_in(8);
        let row_of = if in_other {
            other_database.id
        } else {
            database.id
        };
        let row = page(Some(row_of), random.pick(&positions).clone());
        let id = row.id;
        operations.push(Operation::Create(Record::Page(row)));
        for property in &properties {
            if random.one_in(4) {
                continue;
            }
            operations.push(Operation::Set(FieldChange::PropertyValue {
                page: id,
                property: property.id,
                value: Some(value(random, property)),
            }));
        }
    }
    apply(store, operations);

    let filter = (0..random.below(4))
        .map(|_| {
            let property = random.pick(&properties);
            FilterCondition {
                property: property.id,
                condition: condition(random, property),
            }
        })
        .collect();
    let sort = (0..random.below(4))
        .map(|_| SortKey {
            property: random.pick(&properties).id,
            direction: if random.one_in(2) {
                SortDirection::Ascending
            } else {
                SortDirection::Descending
            },
        })
        .collect();
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "Random".into(),
        view_type: ViewType::Table,
        filter,
        sort,
        visible_columns: Vec::new(),
        column_widths: Vec::new(),
    };
    let view_id = view.id;
    apply(store, vec![Operation::Create(Record::View(view))]);

    let mut schema_changes = Vec::new();
    for property in &properties {
        match random.below(6) {
            0 => schema_changes.push(DatabaseField::RemovedProperty(property.id)),
            1 => schema_changes.push(DatabaseField::Property(Property {
                property_type: *random.pick(&TYPES),
                ..property.clone()
            })),
            // Reordering options changes how select values sort.
            2 => schema_changes.push(DatabaseField::Property(Property {
                options: property.options.iter().rev().cloned().collect(),
                ..property.clone()
            })),
            _ => {}
        }
    }
    let operations = schema_changes
        .into_iter()
        .map(|field| {
            Operation::Set(FieldChange::Database {
                id: database.id,
                field,
            })
        })
        .collect();
    apply(store, operations);
    view_id
}

fn reference_order(store: &WorkspaceStore, view: ViewId) -> Vec<PageId> {
    let view = store.view(view).unwrap().unwrap();
    let database = store.database(view.database).unwrap().unwrap();
    let rows: Vec<TableRow> = test_support::rows_of_database(store, database.id)
        .unwrap()
        .into_iter()
        .map(|page| TableRow {
            values: store.property_values(page.id).unwrap(),
            page,
        })
        .collect();
    let shown = rows_in_view(&view, &database, rows);
    shown.into_iter().map(|row| row.page.id).collect()
}

#[test]
fn view_row_ids_match_the_reference_on_random_tables() {
    for seed in 0..300 {
        let mut random = Random(seed);
        let mut store = open_in_memory();
        let view = random_table(&mut random, &mut store);
        let expected = reference_order(&store, view);
        let actual = store.view_row_ids(view).unwrap().rows;
        assert_eq!(
            actual,
            expected,
            "seed {seed}: view {:?}",
            store.view(view).unwrap()
        );
    }
}

#[test]
fn rows_that_leave_or_are_deleted_drop_out_and_table_rows_skips_missing_ones() {
    let mut random = Random(7);
    let mut store = open_in_memory();
    let view = random_table(&mut random, &mut store);
    let database = store.view(view).unwrap().unwrap().database;
    let rows = test_support::rows_of_database(&store, database).unwrap();
    let (gone, kept) = (rows[0].id, rows[1].id);
    apply(&mut store, vec![Operation::Delete(RecordId::Page(gone))]);

    let after = store.view_row_ids(view).unwrap().rows;
    assert!(!after.contains(&gone));
    assert_eq!(after, reference_order(&store, view));
    let remaining = test_support::rows_of_database(&store, database).unwrap();
    let last = remaining.iter().map(|row| &row.position).max().cloned();
    assert_eq!(store.last_row_position(database).unwrap(), last);
    let loaded = store.table_rows(&[gone, kept]).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].page.id, kept);
    assert_eq!(loaded[0].values, store.property_values(kept).unwrap());
}

#[test]
fn a_view_changed_only_in_memory_is_evaluated_as_given() {
    let mut random = Random(11);
    let mut store = open_in_memory();
    let view_id = random_table(&mut random, &mut store);
    let stored = store.view(view_id).unwrap().unwrap();
    let database = store.database(stored.database).unwrap().unwrap();
    let changed = View {
        filter: Vec::new(),
        sort: vec![SortKey {
            property: database.properties[0].id,
            direction: SortDirection::Descending,
        }],
        ..stored.clone()
    };
    let rows: Vec<TableRow> = test_support::rows_of_database(&store, database.id)
        .unwrap()
        .into_iter()
        .map(|page| TableRow {
            values: store.property_values(page.id).unwrap(),
            page,
        })
        .collect();
    let expected: Vec<PageId> = rows_in_view(&changed, &database, rows)
        .into_iter()
        .map(|row| row.page.id)
        .collect();
    assert_eq!(store.rows_for_view(changed).unwrap().rows, expected);
    assert_eq!(store.view(view_id).unwrap().unwrap(), stored);
}

#[test]
fn a_view_of_a_file_scans_properties_in_parallel_with_the_same_result() {
    let directory = std::env::temp_dir().join(format!("scroll-scans-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    for seed in 0..40 {
        let path = directory.join(format!("{seed}.scroll"));
        let mut store = WorkspaceStore::open(&path).unwrap();
        let view = random_table(&mut Random(seed), &mut store);
        let expected = reference_order(&store, view);
        assert_eq!(
            store.view_row_ids(view).unwrap().rows,
            expected,
            "seed {seed}"
        );
    }
    std::fs::remove_dir_all(&directory).unwrap();
}
