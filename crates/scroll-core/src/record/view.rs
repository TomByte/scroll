//! Table views of a database: filter, sort and columns.

use serde::{Deserialize, Serialize};

use super::database::PropertyType;
use super::value::Date;
use crate::id::{DatabaseId, PropertyId, SelectOptionId, ViewId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub id: ViewId,
    pub database: DatabaseId,
    pub name: String,
    pub view_type: ViewType,
    /// All conditions must hold.
    pub filter: Vec<FilterCondition>,
    /// Applied in order; later keys break ties in earlier ones.
    pub sort: Vec<SortKey>,
    /// In display order.
    pub visible_columns: Vec<PropertyId>,
    /// Columns the person has resized; others use the table's default width.
    pub column_widths: Vec<ColumnWidth>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnWidth {
    pub property: PropertyId,
    /// In points.
    pub width: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewType {
    Table,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterCondition {
    pub property: PropertyId,
    pub condition: Condition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    IsEmpty,
    IsNotEmpty,
    TextContains(String),
    TextEquals(String),
    NumberEquals(f64),
    NumberGreaterThan(f64),
    NumberLessThan(f64),
    SelectIs(SelectOptionId),
    MultiSelectContains(SelectOptionId),
    DateBefore(Date),
    DateOn(Date),
    DateAfter(Date),
    CheckboxIs(bool),
}

impl Condition {
    /// Whether the condition can filter a property of this type. Emptiness applies to every type.
    pub fn applies_to(&self, property_type: PropertyType) -> bool {
        use PropertyType as Type;
        match self {
            Condition::IsEmpty | Condition::IsNotEmpty => true,
            Condition::TextContains(_) | Condition::TextEquals(_) => {
                matches!(property_type, Type::Text | Type::Url)
            }
            Condition::NumberEquals(_)
            | Condition::NumberGreaterThan(_)
            | Condition::NumberLessThan(_) => property_type == Type::Number,
            Condition::SelectIs(_) => property_type == Type::Select,
            Condition::MultiSelectContains(_) => property_type == Type::MultiSelect,
            Condition::DateBefore(_) | Condition::DateOn(_) | Condition::DateAfter(_) => {
                property_type == Type::Date
            }
            Condition::CheckboxIs(_) => property_type == Type::Checkbox,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    pub property: PropertyId,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ViewField {
    Name(String),
    ViewType(ViewType),
    Filter(Vec<FilterCondition>),
    Sort(Vec<SortKey>),
    VisibleColumns(Vec<PropertyId>),
    ColumnWidths(Vec<ColumnWidth>),
}

impl ViewField {
    pub fn apply_to(&self, view: &mut View) {
        match self {
            ViewField::Name(name) => view.name.clone_from(name),
            ViewField::ViewType(view_type) => view.view_type = *view_type,
            ViewField::Filter(filter) => view.filter.clone_from(filter),
            ViewField::Sort(sort) => view.sort.clone_from(sort),
            ViewField::VisibleColumns(columns) => view.visible_columns.clone_from(columns),
            ViewField::ColumnWidths(widths) => view.column_widths.clone_from(widths),
        }
    }
}
