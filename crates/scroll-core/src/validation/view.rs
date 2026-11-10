//! Rules for table views: filters, sort keys and columns must suit the database.

use super::ValidationError;
use super::database::find_property;
use super::value::check_option_exists;
use crate::id::PropertyId;
use crate::operation::ViewField;
use crate::record::{ColumnWidth, Condition, Database, FilterCondition, Property, View};

/// The widest a column may be set, in points.
pub const MAXIMUM_COLUMN_WIDTH: u32 = 4_000;

pub fn check_view(view: &View, database: &Database) -> Result<(), ValidationError> {
    check_filter(&view.filter, database)?;
    check_property_references(database, view.sort.iter().map(|key| key.property))?;
    check_property_references(database, view.visible_columns.iter().copied())?;
    check_column_widths(&view.column_widths, database)
}

/// Each width names a property of the database and is between 1 and `MAXIMUM_COLUMN_WIDTH` points.
pub(crate) fn check_column_widths(
    widths: &[ColumnWidth],
    database: &Database,
) -> Result<(), ValidationError> {
    for width in widths {
        find_property(database, width.property)?;
        if width.width == 0 || width.width > MAXIMUM_COLUMN_WIDTH {
            return Err(ValidationError::InvalidColumnWidth {
                property: width.property,
                width: width.width,
            });
        }
    }
    Ok(())
}

pub fn check_view_field(field: &ViewField, database: &Database) -> Result<(), ValidationError> {
    match field {
        ViewField::Name(_) | ViewField::ViewType(_) => Ok(()),
        ViewField::Filter(filter) => check_filter(filter, database),
        ViewField::Sort(sort) => {
            check_property_references(database, sort.iter().map(|key| key.property))
        }
        ViewField::VisibleColumns(columns) => {
            check_property_references(database, columns.iter().copied())
        }
        ViewField::ColumnWidths(widths) => check_column_widths(widths, database),
    }
}

pub(crate) fn check_filter(
    filter: &[FilterCondition],
    database: &Database,
) -> Result<(), ValidationError> {
    filter.iter().try_for_each(|condition| {
        let property = find_property(database, condition.property)?;
        check_condition(property, &condition.condition)
    })
}

/// A condition must suit its property's type, and any option, date or number it holds must be valid.
pub fn check_condition(property: &Property, condition: &Condition) -> Result<(), ValidationError> {
    if !condition.applies_to(property.property_type) {
        return Err(ValidationError::ConditionDoesNotFitProperty {
            property: property.id,
            property_type: property.property_type,
            condition: condition.clone(),
        });
    }
    match condition {
        Condition::NumberEquals(number)
        | Condition::NumberGreaterThan(number)
        | Condition::NumberLessThan(number)
            if !number.is_finite() =>
        {
            Err(ValidationError::NumberNotFinite(property.id))
        }
        Condition::DateBefore(date) | Condition::DateOn(date) | Condition::DateAfter(date)
            if !date.is_valid() =>
        {
            Err(ValidationError::InvalidDate(*date))
        }
        Condition::SelectIs(option) | Condition::MultiSelectContains(option) => {
            check_option_exists(property, *option)
        }
        _ => Ok(()),
    }
}

/// Every property a filter, sort or column list mentions must exist.
pub(crate) fn check_property_references(
    database: &Database,
    properties: impl IntoIterator<Item = PropertyId>,
) -> Result<(), ValidationError> {
    for property in properties {
        find_property(database, property)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SelectOptionId;
    use crate::record::PropertyType;
    use crate::validation::test_builders::{database_with, select_property};

    #[test]
    fn filter_conditions_must_suit_their_property() {
        let status = select_property();
        let option = status.options[0].id;
        let notes = Property {
            id: PropertyId::generate(),
            name: "Notes".into(),
            property_type: PropertyType::Text,
            options: vec![],
        };
        let database = database_with(vec![status.clone(), notes.clone()]);
        let condition = |property: &Property, condition| FilterCondition {
            property: property.id,
            condition,
        };

        let fitting = [
            condition(&status, Condition::SelectIs(option)),
            condition(&status, Condition::IsEmpty),
            condition(&notes, Condition::TextContains("x".into())),
        ];
        assert_eq!(check_filter(&fitting, &database), Ok(()));

        let wrong_type = [condition(&notes, Condition::NumberGreaterThan(1.0))];
        assert!(matches!(
            check_filter(&wrong_type, &database),
            Err(ValidationError::ConditionDoesNotFitProperty { property, .. }) if property == notes.id
        ));

        let unknown_option = SelectOptionId::generate();
        let missing_option = [condition(&status, Condition::SelectIs(unknown_option))];
        assert_eq!(
            check_filter(&missing_option, &database),
            Err(ValidationError::SelectOptionNotFound {
                property: status.id,
                option: unknown_option
            })
        );

        let unknown = PropertyId::generate();
        let missing_property = [FilterCondition {
            property: unknown,
            condition: Condition::IsEmpty,
        }];
        assert_eq!(
            check_filter(&missing_property, &database),
            Err(ValidationError::PropertyNotFound {
                database: database.id,
                property: unknown
            })
        );
    }
}
