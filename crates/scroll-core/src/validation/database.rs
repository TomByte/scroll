//! Rules for databases and their schemas.

use std::collections::HashSet;

use super::ValidationError;
use crate::id::{DatabaseId, PropertyId};
use crate::record::{Database, Property};

pub fn check_database_has_no_rows(
    database: DatabaseId,
    row_count: usize,
) -> Result<(), ValidationError> {
    if row_count > 0 {
        return Err(ValidationError::DatabaseHasRows(database));
    }
    Ok(())
}

pub fn check_database_has_no_views(
    database: DatabaseId,
    view_count: usize,
) -> Result<(), ValidationError> {
    if view_count > 0 {
        return Err(ValidationError::DatabaseHasViews(database));
    }
    Ok(())
}

pub fn check_database(database: &Database) -> Result<(), ValidationError> {
    let mut seen = HashSet::new();
    for property in &database.properties {
        if !seen.insert(property.id) {
            return Err(ValidationError::DuplicateProperty {
                database: database.id,
                property: property.id,
            });
        }
        check_property(property)?;
    }
    Ok(())
}

pub fn check_property(property: &Property) -> Result<(), ValidationError> {
    let mut seen = HashSet::new();
    for option in &property.options {
        if !seen.insert(option.id) {
            return Err(ValidationError::DuplicateSelectOption {
                property: property.id,
                option: option.id,
            });
        }
    }
    Ok(())
}

pub fn find_property(
    database: &Database,
    property: PropertyId,
) -> Result<&Property, ValidationError> {
    database
        .properties
        .iter()
        .find(|candidate| candidate.id == property)
        .ok_or(ValidationError::PropertyNotFound {
            database: database.id,
            property,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::test_builders::{database_with, select_property};

    #[test]
    fn duplicate_ids_in_a_schema_are_rejected() {
        let property = select_property();
        let duplicate_option = Property {
            options: vec![property.options[0].clone(), property.options[0].clone()],
            ..property.clone()
        };
        assert_eq!(
            check_property(&duplicate_option),
            Err(ValidationError::DuplicateSelectOption {
                property: property.id,
                option: property.options[0].id
            })
        );

        let database = database_with(vec![property.clone(), property.clone()]);
        assert_eq!(
            check_database(&database),
            Err(ValidationError::DuplicateProperty {
                database: database.id,
                property: property.id
            })
        );
    }
}
