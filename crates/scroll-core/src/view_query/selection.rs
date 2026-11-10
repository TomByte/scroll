//! The one algorithm that picks a view's rows and orders them, over rows numbered in position
//! order (position, then id: the final tie-break of every sort). Values arrive one property at a
//! time, so the store can stream each property's values without loading rows.

use std::cmp::Ordering;

use super::values::{SortValue, compare_key, holds, sort_value};
use crate::record::{Condition, Database, Property, PropertyValue, SortDirection, View, fits};

pub(crate) enum Scan<'view> {
    Filter(&'view Property, &'view Condition),
    Sort(&'view Property, SortDirection),
}

/// What the view reads, in order: its conditions, then its sort keys. Those naming a property
/// the database no longer has are left out.
pub(crate) fn scans<'view>(view: &'view View, database: &'view Database) -> Vec<Scan<'view>> {
    let property = |id| {
        database
            .properties
            .iter()
            .find(|property| property.id == id)
    };
    let filters = view.filter.iter().filter_map(move |filter| {
        Some(Scan::Filter(property(filter.property)?, &filter.condition))
    });
    let sorts = view
        .sort
        .iter()
        .filter_map(move |key| Some(Scan::Sort(property(key.property)?, key.direction)));
    filters.chain(sorts).collect()
}

impl Scan<'_> {
    pub(crate) fn property(&self) -> &Property {
        match self {
            Scan::Filter(property, _) | Scan::Sort(property, _) => property,
        }
    }

    pub(crate) fn empty_column(&self, row_count: usize) -> Column {
        match self {
            Scan::Filter(_, condition) => Column::Passing(vec![holds(condition, None); row_count]),
            Scan::Sort(_, direction) => Column::Keys(vec![None; row_count], *direction),
        }
    }

    /// Records row `number`'s value. A value of a type the property no longer has counts as
    /// no value.
    pub(crate) fn add(&self, column: &mut Column, number: usize, value: &PropertyValue) {
        let value = Some(value).filter(|value| fits(value, self.property()));
        match (self, column) {
            (Scan::Filter(_, condition), Column::Passing(passing)) => {
                passing[number] = holds(condition, value);
            }
            (Scan::Sort(property, _), Column::Keys(keys, _)) => {
                keys[number] = value.map(|value| sort_value(property, value));
            }
            _ => {}
        }
    }
}

pub(crate) enum Column {
    Passing(Vec<bool>),
    Keys(Vec<Option<SortValue>>, SortDirection),
}

/// The numbers of the rows that pass every condition column, ordered by the key columns.
pub(crate) fn shown_in_order(row_count: usize, columns: Vec<Column>) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..row_count).collect();
    let mut keys = Vec::new();
    for column in columns {
        match column {
            Column::Passing(passing) => shown.retain(|number| passing[*number]),
            Column::Keys(column, direction) => keys.push((column, direction)),
        }
    }
    if !keys.is_empty() {
        shown.sort_by(|left, right| {
            keys.iter()
                .map(|(column, direction)| {
                    compare_key(column[*left].as_ref(), column[*right].as_ref(), *direction)
                })
                .find(|ordering| ordering.is_ne())
                .unwrap_or(Ordering::Equal)
        });
    }
    shown
}
