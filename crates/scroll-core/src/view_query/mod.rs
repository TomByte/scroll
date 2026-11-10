//! Evaluating a table view: which of a database's rows it shows, and in what order. The store
//! runs the one algorithm (`selection`) over each property's values as SQLite streams them;
//! `rows_in_view` runs it over rows held in memory, as the reference the store's tests check
//! against.
//!
//! Values whose type no longer matches their property (after a type change) count as empty.
//! Text comparisons ignore case. Empty values sort last in either direction. Rows the sort keys
//! cannot tell apart keep their position order, then id order.

mod selection;
mod values;

use crate::id::PropertyId;
use crate::record::{Database, Page, PropertyValue, View};

pub(crate) use selection::{Column, Scan, scans, shown_in_order};

#[derive(Debug, Clone, PartialEq)]
pub struct TableRow {
    pub page: Page,
    pub values: Vec<(PropertyId, PropertyValue)>,
}

/// The rows the view shows, in the order it shows them. Filter conditions and sort keys naming a
/// property the database no longer has are ignored.
#[cfg(test)]
pub(crate) fn rows_in_view(
    view: &View,
    database: &Database,
    mut rows: Vec<TableRow>,
) -> Vec<TableRow> {
    rows.sort_by(|left, right| {
        (&left.page.position, left.page.id).cmp(&(&right.page.position, right.page.id))
    });
    let columns = scans(view, database)
        .iter()
        .map(|scan| {
            let mut column = scan.empty_column(rows.len());
            for (number, row) in rows.iter().enumerate() {
                let value = row.values.iter().find(|(id, _)| *id == scan.property().id);
                if let Some((_, value)) = value {
                    scan.add(&mut column, number, value);
                }
            }
            column
        })
        .collect();
    let mut rows: Vec<Option<TableRow>> = rows.into_iter().map(Some).collect();
    shown_in_order(rows.len(), columns)
        .into_iter()
        .filter_map(|number| rows[number].take())
        .collect()
}

/// Whether a change to `property`'s values can move rows in or out of the view or reorder
/// them: the view filters or sorts by it, and the database still has it.
pub fn places_rows_by(view: &View, database: &Database, property: PropertyId) -> bool {
    scans(view, database)
        .iter()
        .any(|scan| scan.property().id == property)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{DatabaseId, PageId, SelectOptionId, ViewId};
    use crate::position::Position;
    use crate::record::{Condition, FilterCondition, Property, SortDirection, SortKey};
    use crate::record::{PropertyType, SelectOption, ViewType};

    struct Fixture {
        database: Database,
        status: PropertyId,
        estimate: PropertyId,
        name: PropertyId,
        todo: SelectOptionId,
        done: SelectOptionId,
    }

    fn fixture() -> Fixture {
        let (todo, done) = (SelectOptionId::generate(), SelectOptionId::generate());
        let property = |name: &str, property_type, options| Property {
            id: PropertyId::generate(),
            name: name.into(),
            property_type,
            options,
        };
        let status = property(
            "Status",
            PropertyType::Select,
            vec![
                SelectOption {
                    id: todo,
                    name: "To do".into(),
                },
                SelectOption {
                    id: done,
                    name: "Done".into(),
                },
            ],
        );
        let estimate = property("Estimate", PropertyType::Number, vec![]);
        let name = property("Name", PropertyType::Text, vec![]);
        Fixture {
            status: status.id,
            estimate: estimate.id,
            name: name.id,
            todo,
            done,
            database: Database {
                id: DatabaseId::generate(),
                host_page: PageId::generate(),
                properties: vec![status, estimate, name],
            },
        }
    }

    fn row(title: &str, values: Vec<(PropertyId, PropertyValue)>) -> TableRow {
        TableRow {
            page: Page {
                id: PageId::generate(),
                parent: None,
                title: title.into(),
                icon: None,
                position: Position::between(None, None).unwrap(),
                row_of: None,
                is_archived: false,
            },
            values,
        }
    }

    fn view(filter: Vec<FilterCondition>, sort: Vec<SortKey>) -> View {
        View {
            id: ViewId::generate(),
            database: DatabaseId::generate(),
            name: "View".into(),
            view_type: ViewType::Table,
            filter,
            sort,
            visible_columns: vec![],
            column_widths: Vec::new(),
        }
    }

    fn titles(rows: &[TableRow]) -> Vec<&str> {
        rows.iter().map(|row| row.page.title.as_str()).collect()
    }

    fn sample_rows(f: &Fixture) -> Vec<TableRow> {
        vec![
            row(
                "a",
                vec![
                    (f.status, PropertyValue::Select(f.done)),
                    (f.estimate, PropertyValue::Number(3.0)),
                ],
            ),
            row(
                "b",
                vec![
                    (f.status, PropertyValue::Select(f.todo)),
                    (f.estimate, PropertyValue::Number(1.0)),
                ],
            ),
            row(
                "c",
                vec![
                    (f.estimate, PropertyValue::Number(2.0)),
                    (f.name, PropertyValue::Text("Report".into())),
                ],
            ),
            // A stale value of the wrong type counts as empty.
            row(
                "d",
                vec![
                    (f.status, PropertyValue::Text("old".into())),
                    (f.name, PropertyValue::Text("report draft".into())),
                ],
            ),
        ]
    }

    #[test]
    fn all_filter_conditions_must_hold() {
        let f = fixture();
        let condition = |property, condition| FilterCondition {
            property,
            condition,
        };

        let done = view(
            vec![condition(f.status, Condition::SelectIs(f.done))],
            vec![],
        );
        assert_eq!(
            titles(&rows_in_view(&done, &f.database, sample_rows(&f))),
            ["a"]
        );

        let unset = view(vec![condition(f.status, Condition::IsEmpty)], vec![]);
        let unset_rows = rows_in_view(&unset, &f.database, sample_rows(&f));
        let mut shown = titles(&unset_rows);
        shown.sort();
        assert_eq!(shown, ["c", "d"]);

        let both = view(
            vec![
                condition(f.name, Condition::TextContains("REPORT".into())),
                condition(f.estimate, Condition::NumberGreaterThan(1.5)),
            ],
            vec![],
        );
        assert_eq!(
            titles(&rows_in_view(&both, &f.database, sample_rows(&f))),
            ["c"]
        );
    }

    #[test]
    fn sort_keys_apply_in_order_with_empty_values_last() {
        let f = fixture();
        let key = |property, direction| SortKey {
            property,
            direction,
        };

        let by_estimate = view(vec![], vec![key(f.estimate, SortDirection::Descending)]);
        assert_eq!(
            titles(&rows_in_view(&by_estimate, &f.database, sample_rows(&f))),
            ["a", "c", "b", "d"]
        );

        // Select options sort in the order the property lists them: to do before done.
        let by_status = view(
            vec![],
            vec![
                key(f.status, SortDirection::Ascending),
                key(f.estimate, SortDirection::Ascending),
            ],
        );
        assert_eq!(
            titles(&rows_in_view(&by_status, &f.database, sample_rows(&f))),
            ["b", "a", "c", "d"]
        );
    }

    #[test]
    fn without_sort_keys_rows_keep_position_order() {
        let f = fixture();
        let mut rows = sample_rows(&f);
        let mut position = Position::between(None, None).unwrap();
        for row in rows.iter_mut().rev() {
            row.page.position = position.clone();
            position = Position::between(None, Some(&position)).unwrap();
        }
        assert_eq!(
            titles(&rows_in_view(&view(vec![], vec![]), &f.database, rows)),
            ["a", "b", "c", "d"]
        );
    }

    #[test]
    fn rows_are_placed_by_the_filtered_and_sorted_properties_the_database_has() {
        let f = fixture();
        let unsorted = view(Vec::new(), Vec::new());
        assert!(!places_rows_by(&unsorted, &f.database, f.estimate));

        let by_estimate = view(
            vec![FilterCondition {
                property: f.status,
                condition: Condition::SelectIs(f.todo),
            }],
            vec![SortKey {
                property: f.estimate,
                direction: SortDirection::Ascending,
            }],
        );
        assert!(places_rows_by(&by_estimate, &f.database, f.estimate));
        assert!(places_rows_by(&by_estimate, &f.database, f.status));
        assert!(!places_rows_by(&by_estimate, &f.database, f.name));

        let mut without_estimate = f.database.clone();
        without_estimate
            .properties
            .retain(|property| property.id != f.estimate);
        assert!(!places_rows_by(&by_estimate, &without_estimate, f.estimate));
    }

    #[test]
    #[ignore = "timing; run with --release --ignored to measure"]
    fn hundred_thousand_rows_filter_and_sort_quickly() {
        let f = fixture();
        let rows: Vec<TableRow> = (0..100_000)
            .map(|index| {
                let status = if index % 3 == 0 { f.done } else { f.todo };
                row(
                    &index.to_string(),
                    vec![
                        (f.status, PropertyValue::Select(status)),
                        (
                            f.estimate,
                            PropertyValue::Number(((index * 7919) % 1000) as f64),
                        ),
                        (
                            f.name,
                            PropertyValue::Text(format!("Task {}", (index * 31) % 100_000)),
                        ),
                    ],
                )
            })
            .collect();
        let query = view(
            vec![FilterCondition {
                property: f.status,
                condition: Condition::SelectIs(f.todo),
            }],
            vec![
                SortKey {
                    property: f.estimate,
                    direction: SortDirection::Descending,
                },
                SortKey {
                    property: f.name,
                    direction: SortDirection::Ascending,
                },
            ],
        );
        let started = std::time::Instant::now();
        let shown = rows_in_view(&query, &f.database, rows);
        let elapsed = started.elapsed();
        println!(
            "rows_in_view over 100,000 rows: {elapsed:?} ({} shown)",
            shown.len()
        );
        assert_eq!(shown.len(), 66_666);
    }
}
