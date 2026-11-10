//! Property values: what a database row holds for each property.

use serde::{Deserialize, Serialize};

use super::database::{Property, PropertyType};
use crate::id::{PropertyId, SelectOptionId};

/// The value of one property on one page, keyed by (page id, property id). An absent value means empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    Select(SelectOptionId),
    MultiSelect(Vec<SelectOptionId>),
    Date(Date),
    Checkbox(bool),
    Url(String),
}

impl PropertyValue {
    pub fn property_type(&self) -> PropertyType {
        match self {
            PropertyValue::Text(_) => PropertyType::Text,
            PropertyValue::Number(_) => PropertyType::Number,
            PropertyValue::Select(_) => PropertyType::Select,
            PropertyValue::MultiSelect(_) => PropertyType::MultiSelect,
            PropertyValue::Date(_) => PropertyType::Date,
            PropertyValue::Checkbox(_) => PropertyType::Checkbox,
            PropertyValue::Url(_) => PropertyType::Url,
        }
    }
}

/// A calendar date with no time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month) && (1..=self.days_in_month()).contains(&self.day)
    }

    pub fn days_in_month(&self) -> u8 {
        match self.month {
            2 if self.is_leap_year() => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    fn is_leap_year(&self) -> bool {
        (self.year % 4 == 0 && self.year % 100 != 0) || self.year % 400 == 0
    }
}

/// The row's value for the property, if it has one of the property's current type. A value of
/// another type (left from before the property's type changed) counts as no value.
pub fn value_for<'values>(
    values: &'values [(PropertyId, PropertyValue)],
    property: &Property,
) -> Option<&'values PropertyValue> {
    values
        .iter()
        .find(|(id, _)| *id == property.id)
        .map(|(_, value)| value)
        .filter(|value| fits(value, property))
}

/// Sets `property`'s value among a row's values, kept in property id order; `None` removes it.
pub fn replace_value(
    values: &mut Vec<(PropertyId, PropertyValue)>,
    property: PropertyId,
    value: Option<&PropertyValue>,
) {
    values.retain(|(existing, _)| *existing != property);
    if let Some(value) = value {
        values.push((property, value.clone()));
        values.sort_by_key(|(property, _)| *property);
    }
}

pub fn fits(value: &PropertyValue, property: &Property) -> bool {
    value.property_type() == property.property_type
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_value_keeps_property_id_order_and_none_removes_it() {
        let ids: Vec<PropertyId> = (0..3).map(|_| PropertyId::generate()).collect();
        let mut values = vec![
            (ids[0], PropertyValue::Number(1.0)),
            (ids[2], PropertyValue::Number(3.0)),
        ];
        replace_value(&mut values, ids[1], Some(&PropertyValue::Number(2.0)));
        replace_value(&mut values, ids[0], Some(&PropertyValue::Number(10.0)));
        assert_eq!(
            values,
            vec![
                (ids[0], PropertyValue::Number(10.0)),
                (ids[1], PropertyValue::Number(2.0)),
                (ids[2], PropertyValue::Number(3.0)),
            ]
        );
        replace_value(&mut values, ids[1], None);
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|(id, _)| *id != ids[1]));
    }

    #[test]
    fn dates_are_checked_against_the_calendar() {
        let date = |year, month, day| Date { year, month, day };
        assert!(date(2024, 2, 29).is_valid());
        assert!(!date(2023, 2, 29).is_valid());
        assert!(date(2000, 2, 29).is_valid());
        assert!(!date(1900, 2, 29).is_valid());
        assert!(!date(2024, 4, 31).is_valid());
        assert!(!date(2024, 13, 1).is_valid());
        assert!(!date(2024, 1, 0).is_valid());
    }
}
