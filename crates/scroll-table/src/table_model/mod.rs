//! What the table shows: the view, its database's schema, the ids of every row the view shows in
//! order, and the contents of the rows near the screen.
//!
//! The table never waits for the store. Its own changes are applied here at once (`apply`), then
//! written in the background. Applying a change does not re-sort or re-filter, so an edited row
//! stays put until the table re-runs the query (`placement` says when that is needed). Which rows
//! are loaded, and reading them, is in `loading`.

mod loading;
mod placement;

#[cfg(test)]
pub use loading::MAX_LOADED_ROWS;
pub use loading::{ReadRequest, ReadResult, read_rows, window_around};
pub use placement::changes_row_placement;

use std::collections::{HashMap, HashSet};

use scroll_core::id::{DatabaseId, PageId, PropertyId, ViewId};
use scroll_core::operation::{FieldChange, Operation, Record, RecordId, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{
    Database, Property, PropertyValue, View, ViewType, replace_value, value_for,
};
use scroll_core::view_query::TableRow;
use uuid::Uuid;

pub struct TableModel {
    view: View,
    database: Database,
    /// Every row the view shows, in display order.
    order: Vec<PageId>,
    loaded: HashMap<PageId, LoadedRow>,
    /// The position of the database's last row, so a new row goes after all of them.
    last_position: Option<Position>,
}

struct LoadedRow {
    row: TableRow,
    /// Where the row was when it was loaded, to tell which rows are farthest from the screen.
    /// Only approximate once rows are added or deleted.
    index: usize,
}

impl TableModel {
    /// An empty table for `view`, shown until the first read arrives.
    pub fn placeholder(view: ViewId) -> TableModel {
        TableModel {
            view: View {
                id: view,
                database: DatabaseId(Uuid::nil()),
                name: String::new(),
                view_type: ViewType::Table,
                filter: Vec::new(),
                sort: Vec::new(),
                visible_columns: Vec::new(),
                column_widths: Vec::new(),
            },
            database: Database {
                id: DatabaseId(Uuid::nil()),
                host_page: PageId(Uuid::nil()),
                properties: Vec::new(),
            },
            order: Vec::new(),
            loaded: HashMap::new(),
            last_position: None,
        }
    }

    /// Mirrors one of the table's own transactions, as the store will once it is written. Rows
    /// keep their places; created rows go to the bottom.
    pub fn apply(&mut self, transaction: &Transaction) {
        let mut deleted = HashSet::new();
        for operation in &transaction.operations {
            match operation {
                Operation::Create(Record::Page(page)) if page.row_of == Some(self.database.id) => {
                    self.last_position =
                        self.last_position.clone().max(Some(page.position.clone()));
                    let index = self.order.len();
                    self.order.push(page.id);
                    let row = TableRow {
                        page: page.clone(),
                        values: Vec::new(),
                    };
                    self.loaded.insert(page.id, LoadedRow { row, index });
                }
                Operation::Delete(RecordId::Page(page)) => {
                    deleted.insert(*page);
                    self.loaded.remove(page);
                }
                Operation::Set(change) => self.apply_change(change),
                _ => {}
            }
        }
        if !deleted.is_empty() {
            self.order.retain(|page| !deleted.contains(page));
        }
    }

    fn apply_change(&mut self, change: &FieldChange) {
        match change {
            FieldChange::PropertyValue {
                page,
                property,
                value,
            } => {
                if let Some(loaded) = self.loaded.get_mut(page) {
                    replace_value(&mut loaded.row.values, *property, value.as_ref());
                }
            }
            FieldChange::Page { id, field } => {
                if let Some(loaded) = self.loaded.get_mut(id) {
                    field.apply_to(&mut loaded.row.page);
                }
            }
            FieldChange::Database { id, field } if *id == self.database.id => {
                field.apply_to(&mut self.database)
            }
            FieldChange::View { id, field } if *id == self.view.id => {
                field.apply_to(&mut self.view)
            }
            _ => {}
        }
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn row_count(&self) -> usize {
        self.order.len()
    }

    /// The id of the row at `index`, loaded or not.
    pub fn row_id(&self, index: usize) -> Option<PageId> {
        self.order.get(index).copied()
    }

    #[cfg(test)]
    pub fn row_ids(&self, range: std::ops::Range<usize>) -> &[PageId] {
        let end = range.end.min(self.order.len());
        &self.order[range.start.min(end)..end]
    }

    /// The row's contents, if loaded.
    pub fn row(&self, index: usize) -> Option<&TableRow> {
        let page = self.order.get(index)?;
        self.loaded.get(page).map(|loaded| &loaded.row)
    }

    pub fn property(&self, id: PropertyId) -> Option<&Property> {
        self.database
            .properties
            .iter()
            .find(|property| property.id == id)
    }

    /// The view's columns that still exist in the schema, in display order.
    pub fn visible_properties(&self) -> Vec<&Property> {
        self.view
            .visible_columns
            .iter()
            .filter_map(|id| self.property(*id))
            .collect()
    }

    /// A scan of the ids: well under a millisecond at a million rows.
    pub fn row_index(&self, page: PageId) -> Option<usize> {
        self.order.iter().position(|row| *row == page)
    }

    /// The pages among `pages` that are still rows of the view.
    pub fn rows_among(&self, pages: &HashSet<PageId>) -> HashSet<PageId> {
        if pages.is_empty() {
            return HashSet::new();
        }
        self.order
            .iter()
            .filter(|page| pages.contains(page))
            .copied()
            .collect()
    }

    /// The row's value for the property, if loaded and of the property's current type.
    pub fn value(&self, row: usize, property: &Property) -> Option<&PropertyValue> {
        value_for(&self.row(row)?.values, property)
    }

    /// The position after every row of the database, for appending a new one.
    pub fn last_position(&self) -> Option<&Position> {
        self.last_position.as_ref()
    }
}

#[cfg(test)]
mod tests;
