//! Databases: their schema, querying their rows through a view, and adding and changing rows.

use std::collections::BTreeMap;

use scroll_core::id::{DatabaseId, PageId, ViewId};
use scroll_core::operation::{Operation, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{
    Condition, Database, FilterCondition, Page, PropertyType, SortDirection, SortKey, View,
    ViewType,
};
use scroll_core::store::WorkspaceStore;
use serde::Deserialize;
use serde_json::{Value, json};

use super::values::{
    boolean, date, number, option, option_names, property_named, text, value_from_json,
    values_to_json,
};
use super::{ToolError, Writer, require_page};

const DEFAULT_ROWS: usize = 50;
const MAXIMUM_ROWS: usize = 200;

/// The schema, as `get_page` shows it on a page hosting a database.
pub(super) fn database_to_json(
    store: &WorkspaceStore,
    database: &Database,
) -> Result<Value, ToolError> {
    let properties: Vec<Value> = database
        .properties
        .iter()
        .map(|property| {
            let mut described = json!({
                "name": property.name,
                "type": type_name(property.property_type),
            });
            if matches!(
                property.property_type,
                PropertyType::Select | PropertyType::MultiSelect
            ) {
                described["options"] = json!(option_names(property));
            }
            described
        })
        .collect();
    let views: Vec<Value> = store
        .views(database.id)?
        .into_iter()
        .map(|view| json!({"id": view.id.0, "name": view.name}))
        .collect();
    Ok(json!({"id": database.id.0, "properties": properties, "views": views}))
}

fn type_name(property_type: PropertyType) -> &'static str {
    match property_type {
        PropertyType::Text => "text",
        PropertyType::Number => "number",
        PropertyType::Select => "select",
        PropertyType::MultiSelect => "multi_select",
        PropertyType::Date => "date",
        PropertyType::Checkbox => "checkbox",
        PropertyType::Url => "url",
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryDatabase {
    database_id: DatabaseId,
    view_id: Option<ViewId>,
    filter: Option<Vec<FilterInput>>,
    sort: Option<Vec<SortInput>>,
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterInput {
    property: String,
    operator: Operator,
    value: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operator {
    IsEmpty,
    IsNotEmpty,
    Equals,
    Contains,
    GreaterThan,
    LessThan,
    Before,
    After,
}

impl Operator {
    pub(super) const ALL: [Operator; 8] = [
        Operator::IsEmpty,
        Operator::IsNotEmpty,
        Operator::Equals,
        Operator::Contains,
        Operator::GreaterThan,
        Operator::LessThan,
        Operator::Before,
        Operator::After,
    ];

    /// As the model writes it.
    pub(super) fn name(self) -> &'static str {
        match self {
            Operator::IsEmpty => "is_empty",
            Operator::IsNotEmpty => "is_not_empty",
            Operator::Equals => "equals",
            Operator::Contains => "contains",
            Operator::GreaterThan => "greater_than",
            Operator::LessThan => "less_than",
            Operator::Before => "before",
            Operator::After => "after",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SortInput {
    property: String,
    #[serde(default)]
    direction: Direction,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    #[default]
    Ascending,
    Descending,
}

/// Runs a view (the one named, else the database's first) with the filter and sort given in
/// place of its own, through the same query the table uses, and returns one page of its rows.
pub(super) fn query_database(
    store: &WorkspaceStore,
    arguments: QueryDatabase,
) -> Result<Value, ToolError> {
    let database = require_database(store, arguments.database_id)?;
    let mut view = base_view(store, &database, arguments.view_id)?;
    if let Some(filter) = arguments.filter {
        view.filter = filter
            .iter()
            .map(|condition| filter_condition(&database, condition))
            .collect::<Result<_, _>>()?;
    }
    if let Some(sort) = arguments.sort {
        view.sort = sort
            .iter()
            .map(|key| sort_key(&database, key))
            .collect::<Result<_, _>>()?;
    }
    let shown = store.rows_for_view(view)?.rows;
    let limit = arguments
        .limit
        .unwrap_or(DEFAULT_ROWS)
        .clamp(1, MAXIMUM_ROWS);
    let wanted: Vec<PageId> = shown
        .iter()
        .skip(arguments.offset)
        .take(limit)
        .copied()
        .collect();
    let rows: Vec<Value> = store
        .table_rows(&wanted)?
        .into_iter()
        .map(|row| {
            json!({
                "id": row.page.id.0,
                "title": row.page.title,
                "values": values_to_json(&database, &row.values),
            })
        })
        .collect();
    Ok(json!({
        "database_id": database.id.0,
        "total": shown.len(),
        "offset": arguments.offset,
        "rows": rows,
    }))
}

fn base_view(
    store: &WorkspaceStore,
    database: &Database,
    view: Option<ViewId>,
) -> Result<View, ToolError> {
    if let Some(id) = view {
        return store
            .view(id)?
            .filter(|view| view.database == database.id)
            .ok_or(ToolError::ViewNotFound {
                database: database.id,
                view: id,
            });
    }
    // A database always has a view in the app; one made elsewhere might not.
    Ok(store
        .views(database.id)?
        .into_iter()
        .next()
        .unwrap_or_else(|| View {
            id: ViewId::generate(),
            database: database.id,
            name: String::new(),
            view_type: ViewType::Table,
            filter: Vec::new(),
            sort: Vec::new(),
            visible_columns: Vec::new(),
            column_widths: Vec::new(),
        }))
}

fn filter_condition(
    database: &Database,
    filter: &FilterInput,
) -> Result<FilterCondition, ToolError> {
    let property = property_named(database, &filter.property)?;
    let operand = filter
        .value
        .as_ref()
        .ok_or_else(|| ToolError::MissingFilterValue {
            property: property.name.clone(),
            operator: filter.operator.name(),
        });
    use Operator as Op;
    use PropertyType as Type;
    let condition = match (filter.operator, property.property_type) {
        (Op::IsEmpty, _) => Condition::IsEmpty,
        (Op::IsNotEmpty, _) => Condition::IsNotEmpty,
        (Op::Equals, Type::Text | Type::Url) => Condition::TextEquals(text(property, operand?)?),
        (Op::Contains, Type::Text | Type::Url) => {
            Condition::TextContains(text(property, operand?)?)
        }
        (Op::Equals, Type::Number) => Condition::NumberEquals(number(property, operand?)?),
        (Op::GreaterThan, Type::Number) => {
            Condition::NumberGreaterThan(number(property, operand?)?)
        }
        (Op::LessThan, Type::Number) => Condition::NumberLessThan(number(property, operand?)?),
        (Op::Equals, Type::Select) => Condition::SelectIs(option(property, operand?)?),
        (Op::Contains, Type::MultiSelect) => {
            Condition::MultiSelectContains(option(property, operand?)?)
        }
        (Op::Equals, Type::Checkbox) => Condition::CheckboxIs(boolean(property, operand?)?),
        (Op::Equals, Type::Date) => Condition::DateOn(date(property, operand?)?),
        (Op::Before, Type::Date) => Condition::DateBefore(date(property, operand?)?),
        (Op::After, Type::Date) => Condition::DateAfter(date(property, operand?)?),
        _ => {
            return Err(ToolError::UnsupportedFilter {
                property: property.name.clone(),
                operator: filter.operator.name(),
            });
        }
    };
    Ok(FilterCondition {
        property: property.id,
        condition,
    })
}

fn sort_key(database: &Database, sort: &SortInput) -> Result<SortKey, ToolError> {
    Ok(SortKey {
        property: property_named(database, &sort.property)?.id,
        direction: match sort.direction {
            Direction::Ascending => SortDirection::Ascending,
            Direction::Descending => SortDirection::Descending,
        },
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateRow {
    database_id: DatabaseId,
    #[serde(default)]
    title: String,
    #[serde(default)]
    values: BTreeMap<String, Value>,
}

/// A row after the database's last, as the table adds one, with its title and values set in the
/// same transaction.
pub(super) fn create_row(writer: &mut Writer, arguments: CreateRow) -> Result<Value, ToolError> {
    let database = require_database(writer.store, arguments.database_id)?;
    let last = writer.store.last_row_position(database.id)?;
    let page = Page {
        id: PageId::generate(),
        parent: Some(database.host_page),
        title: arguments.title,
        icon: None,
        position: Position::between(last.as_ref(), None)?,
        row_of: Some(database.id),
        is_archived: false,
    };
    let id = page.id;
    let mut operations = vec![Operation::Create(Record::Page(page))];
    operations.extend(value_operations(&database, id, &arguments.values)?);
    writer.apply(&Transaction::new(operations))?;
    Ok(json!({"page_id": id.0}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateRow {
    page_id: PageId,
    title: Option<String>,
    #[serde(default)]
    values: BTreeMap<String, Value>,
}

pub(super) fn update_row(writer: &mut Writer, arguments: UpdateRow) -> Result<Value, ToolError> {
    let page = require_page(writer.store, arguments.page_id)?;
    let database_id = page.row_of.ok_or(ToolError::NotARow(page.id))?;
    let database = require_database(writer.store, database_id)?;
    let mut operations: Vec<Operation> = arguments
        .title
        .iter()
        .map(|title| Operation::set_title(page.id, title))
        .collect();
    operations.extend(value_operations(&database, page.id, &arguments.values)?);
    if operations.is_empty() {
        return Err(ToolError::NothingToChange);
    }
    writer.apply(&Transaction::new(operations))?;
    Ok(json!({"page_id": page.id.0}))
}

fn value_operations(
    database: &Database,
    page: PageId,
    values: &BTreeMap<String, Value>,
) -> Result<Vec<Operation>, ToolError> {
    values
        .iter()
        .map(|(name, value)| {
            let property = property_named(database, name)?;
            let value = value_from_json(property, value)?;
            Ok(Operation::set_value(page, property.id, value))
        })
        .collect()
}

fn require_database(store: &WorkspaceStore, id: DatabaseId) -> Result<Database, ToolError> {
    store.database(id)?.ok_or(ToolError::DatabaseNotFound(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operator_is_read_by_the_name_it_is_described_with() {
        for operator in Operator::ALL {
            let read: Operator = serde_json::from_value(json!(operator.name())).unwrap();
            assert_eq!(read, operator);
        }
    }
}
