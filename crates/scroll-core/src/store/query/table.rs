//! The rows of a table view. `view_row_ids` finds which rows a view shows and in what order
//! without loading them; `table_rows` loads the few on screen.
//!
//! The order is worked out here rather than in SQL: SQLite reads the rows in position order and
//! each filtered or sorted property's values in one sequential range (values are keyed by
//! property first), and the conditions and sort keys are evaluated with the same rules as
//! `view_query::rows_in_view`, which stays the reference.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::PathBuf;

use rusqlite::Connection;

use crate::id::{PageId, ViewId};
use crate::record::{Database, PropertyValue, View};
use crate::store::kinds::database::read_database;
use crate::store::kinds::page::read_page;
use crate::store::kinds::value::read_property_values;
use crate::store::kinds::view::read_view;
use crate::store::{StoreError, WorkspaceStore, decode};
use crate::validation::ValidationError;
use crate::view_query::{Column, Scan, TableRow, scans, shown_in_order};

/// Which rows a view shows, in display order, without their contents.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewRowIds {
    pub view: View,
    pub database: Database,
    pub rows: Vec<PageId>,
}

impl WorkspaceStore {
    /// The view, its database and the ids of the rows it shows, filtered and in display order.
    pub fn view_row_ids(&self, view_id: ViewId) -> Result<ViewRowIds, StoreError> {
        let view = read_view(&self.conn, view_id)?.ok_or(ValidationError::ViewNotFound(view_id))?;
        self.rows_for_view(view)
    }

    /// As `view_row_ids`, for a view as given rather than as stored: a viewer's sort or filter
    /// that is kept for the session and never written.
    pub fn rows_for_view(&self, view: View) -> Result<ViewRowIds, StoreError> {
        let database = read_database(&self.conn, view.database)?
            .ok_or(ValidationError::DatabaseNotFound(view.database))?;
        let rows = ordered_row_ids(&self.conn, &view, &database)?;
        Ok(ViewRowIds {
            view,
            database,
            rows,
        })
    }

    /// The given rows with all their values, in the order given. Ids of pages that no longer
    /// exist are skipped.
    pub fn table_rows(&self, rows: &[PageId]) -> Result<Vec<TableRow>, StoreError> {
        let mut table_rows = Vec::with_capacity(rows.len());
        for id in rows {
            let Some(page) = read_page(&self.conn, *id)? else {
                continue;
            };
            table_rows.push(TableRow {
                values: read_property_values(&self.conn, *id)?,
                page,
            });
        }
        Ok(table_rows)
    }
}

fn ordered_row_ids(
    conn: &Connection,
    view: &View,
    database: &Database,
) -> Result<Vec<PageId>, StoreError> {
    let rows = rows_in_position_order(conn, database)?;
    let numbers: RowNumbers = rows
        .iter()
        .enumerate()
        .map(|(number, id)| (*id, number))
        .collect();
    let columns = scan_columns(conn, &numbers, &scans(view, database))?;
    let shown = shown_in_order(rows.len(), columns);
    Ok(shown.into_iter().map(|number| rows[number]).collect())
}

/// Reads each scan's property values, in order of `scans`. With a file and more than one scan,
/// each runs on a read connection of its own at the same time: each is a sequential read of one
/// property's values, so they overlap well (a filter and two sort keys at 500,000 rows: 95 ms
/// instead of 140 ms of scanning).
fn scan_columns(
    conn: &Connection,
    numbers: &RowNumbers,
    scans: &[Scan],
) -> Result<Vec<Column>, StoreError> {
    let path = conn
        .path()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let Some(path) = path.filter(|_| scans.len() > 1) else {
        return scans
            .iter()
            .map(|scan| read_column(conn, numbers, scan))
            .collect();
    };
    std::thread::scope(|scope| {
        let running: Vec<_> = scans
            .iter()
            .map(|scan| {
                let path = &path;
                scope.spawn(move || {
                    let reader = WorkspaceStore::open_reader(path)?;
                    read_column(&reader.conn, numbers, scan)
                })
            })
            .collect();
        running
            .into_iter()
            .map(|scan| {
                scan.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

/// Row number by page id. Hashing is most of the cost of matching values to rows, and page ids
/// end in random bits, so those are the hash.
type RowNumbers = HashMap<PageId, usize, BuildHasherDefault<PageIdHasher>>;

#[derive(Default)]
struct PageIdHasher(u64);

impl Hasher for PageIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = self.0.rotate_left(5) ^ u64::from_le_bytes(word);
        }
    }
}

fn rows_in_position_order(
    conn: &Connection,
    database: &Database,
) -> Result<Vec<PageId>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT id FROM page WHERE row_of_database_id = ?1 ORDER BY position, id",
    )?;
    let rows = statement.query_map([database.id], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// One scan's column, from one sequential read of the property's values. Values of pages that
/// are not rows of this view (they left the database) are skipped.
fn read_column(conn: &Connection, numbers: &RowNumbers, scan: &Scan) -> Result<Column, StoreError> {
    let mut column = scan.empty_column(numbers.len());
    let mut statement =
        conn.prepare_cached("SELECT page_id, value FROM property_value WHERE property_id = ?1")?;
    let mut rows = statement.query([scan.property().id])?;
    while let Some(row) = rows.next()? {
        let page: PageId = row.get(0)?;
        let Some(number) = numbers.get(&page) else {
            continue;
        };
        let bytes = row.get_ref(1)?.as_blob().map_err(rusqlite::Error::from)?;
        scan.add(&mut column, *number, &decode::<PropertyValue>(bytes)?);
    }
    Ok(column)
}

#[cfg(test)]
mod randomised_tests;

#[cfg(test)]
mod tests {
    use crate::id::{DatabaseId, PageId, PropertyId, ViewId};
    use crate::operation::{FieldChange, Operation, Record, Transaction};
    use crate::position::Position;
    use crate::record::{
        Condition, Database, FilterCondition, Page, Property, PropertyType, PropertyValue,
        SortDirection, SortKey, View, ViewType,
    };
    use crate::store::{WorkspaceStore, open_in_memory};

    struct Table {
        view: ViewId,
        estimate: PropertyId,
    }

    /// A database with a number property, `row_count` rows whose estimate is `estimate(index)`, and a
    /// view showing rows with an estimate above 10, largest first.
    fn table(
        store: &mut WorkspaceStore,
        row_count: usize,
        estimate: impl Fn(usize) -> f64,
    ) -> Table {
        let page = |row_of| Page {
            id: PageId::generate(),
            parent: None,
            title: "Row".into(),
            icon: None,
            position: Position::between(None, None).unwrap(),
            row_of,
            is_archived: false,
        };
        let host = page(None);
        let property = Property {
            id: PropertyId::generate(),
            name: "Estimate".into(),
            property_type: PropertyType::Number,
            options: vec![],
        };
        let database = Database {
            id: DatabaseId::generate(),
            host_page: host.id,
            properties: vec![property.clone()],
        };
        let view = View {
            id: ViewId::generate(),
            database: database.id,
            name: "Big ones".into(),
            view_type: ViewType::Table,
            filter: vec![FilterCondition {
                property: property.id,
                condition: Condition::NumberGreaterThan(10.0),
            }],
            sort: vec![SortKey {
                property: property.id,
                direction: SortDirection::Descending,
            }],
            visible_columns: vec![property.id],
            column_widths: Vec::new(),
        };
        let mut operations = vec![
            Operation::Create(Record::Page(host)),
            Operation::Create(Record::Database(database.clone())),
            Operation::Create(Record::View(view.clone())),
        ];
        for index in 0..row_count {
            let row = page(Some(database.id));
            let row_id = row.id;
            operations.push(Operation::Create(Record::Page(row)));
            operations.push(Operation::Set(FieldChange::PropertyValue {
                page: row_id,
                property: property.id,
                value: Some(PropertyValue::Number(estimate(index))),
            }));
        }
        store
            .apply_local_transaction(&Transaction::new(operations))
            .unwrap();
        Table {
            view: view.id,
            estimate: property.id,
        }
    }

    fn estimates(store: &WorkspaceStore, table: &Table) -> Vec<f64> {
        store
            .table_rows(&store.view_row_ids(table.view).unwrap().rows)
            .unwrap()
            .iter()
            .map(
                |row| match row.values.iter().find(|(id, _)| *id == table.estimate) {
                    Some((_, PropertyValue::Number(number))) => *number,
                    other => panic!("expected a number, got {other:?}"),
                },
            )
            .collect()
    }

    #[test]
    fn a_view_returns_its_filtered_rows_in_sort_order() {
        let mut store = open_in_memory();
        let table = table(&mut store, 6, |index| {
            [5.0, 40.0, 12.0, 10.0, 99.0, 11.0][index]
        });
        assert_eq!(estimates(&store, &table), [99.0, 40.0, 12.0, 11.0]);
    }

    #[test]
    #[ignore = "timing; run with --release --ignored to measure"]
    fn a_hundred_thousand_row_view_loads_quickly() {
        let mut store = open_in_memory();
        let started = std::time::Instant::now();
        let table = table(&mut store, 100_000, |index| ((index * 7919) % 1000) as f64);
        println!("inserting 100,000 rows: {:?}", started.elapsed());

        let started = std::time::Instant::now();
        let shown = estimates(&store, &table);
        println!(
            "view_row_ids and table_rows over 100,000 rows: {:?} ({} shown)",
            started.elapsed(),
            shown.len()
        );
        assert!(shown.windows(2).all(|pair| pair[0] >= pair[1]));
    }
}
