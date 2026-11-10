//! Databases and their schemas.

use serde::{Deserialize, Serialize};

use crate::id::{DatabaseId, PageId, PropertyId, SelectOptionId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Database {
    pub id: DatabaseId,
    /// The page the database lives on. Fixed at creation; a page hosts at most one database.
    pub host_page: PageId,
    /// In creation order.
    pub properties: Vec<Property>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    pub id: PropertyId,
    pub name: String,
    pub property_type: PropertyType,
    /// Used by select and multi-select. Kept on other types so switching type and back keeps them.
    pub options: Vec<SelectOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PropertyType {
    Text,
    Number,
    Select,
    MultiSelect,
    Date,
    Checkbox,
    Url,
}

impl PropertyType {
    /// Every type, in the order menus list them.
    pub const ALL: [PropertyType; 7] = [
        PropertyType::Text,
        PropertyType::Number,
        PropertyType::Select,
        PropertyType::MultiSelect,
        PropertyType::Date,
        PropertyType::Checkbox,
        PropertyType::Url,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub id: SelectOptionId,
    pub name: String,
}

/// Each property of a schema is its own field, so concurrent edits to different properties both survive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseField {
    /// Adds the property, or replaces the one with the same id.
    Property(Property),
    RemovedProperty(PropertyId),
}

impl DatabaseField {
    /// Keeps `properties` ordered by id, which for UUIDv7 ids is creation order.
    pub fn apply_to(&self, database: &mut Database) {
        match self {
            DatabaseField::Property(property) => {
                let existing = database
                    .properties
                    .iter_mut()
                    .find(|existing| existing.id == property.id);
                match existing {
                    Some(existing) => *existing = property.clone(),
                    None => {
                        database.properties.push(property.clone());
                        database.properties.sort_by_key(|property| property.id);
                    }
                }
            }
            DatabaseField::RemovedProperty(id) => {
                database.properties.retain(|property| property.id != *id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_property_type_once() {
        // Exhaustive, so a new type stops this compiling until it is added to `ALL`.
        let listed = |property_type: PropertyType| match property_type {
            PropertyType::Text
            | PropertyType::Number
            | PropertyType::Select
            | PropertyType::MultiSelect
            | PropertyType::Date
            | PropertyType::Checkbox
            | PropertyType::Url => PropertyType::ALL
                .iter()
                .filter(|listed| **listed == property_type)
                .count(),
        };
        for property_type in PropertyType::ALL {
            assert_eq!(listed(property_type), 1, "{property_type:?}");
        }
        assert_eq!(PropertyType::ALL.len(), 7);
    }
}
