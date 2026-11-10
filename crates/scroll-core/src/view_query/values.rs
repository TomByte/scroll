//! What filtering and sorting compare: whether a condition holds for a value, and the key a
//! value sorts by.

use std::cmp::Ordering;

use crate::record::{Condition, Date, Property, PropertyValue, SortDirection};

/// Whether `value` passes the condition. A missing value, or one of a type the condition does
/// not compare, passes only the emptiness conditions and an unchecked-checkbox condition.
pub(crate) fn holds(condition: &Condition, value: Option<&PropertyValue>) -> bool {
    match condition {
        Condition::IsEmpty => is_empty(value),
        Condition::IsNotEmpty => !is_empty(value),
        Condition::TextContains(needle) => value
            .and_then(text_of)
            .is_some_and(|text| text.to_lowercase().contains(&needle.to_lowercase())),
        Condition::TextEquals(expected) => value
            .and_then(text_of)
            .is_some_and(|text| text.to_lowercase() == expected.to_lowercase()),
        Condition::NumberEquals(expected) => number_of(value).is_some_and(|n| n == *expected),
        Condition::NumberGreaterThan(bound) => number_of(value).is_some_and(|n| n > *bound),
        Condition::NumberLessThan(bound) => number_of(value).is_some_and(|n| n < *bound),
        Condition::SelectIs(option) => {
            matches!(value, Some(PropertyValue::Select(selected)) if selected == option)
        }
        Condition::MultiSelectContains(option) => {
            matches!(value, Some(PropertyValue::MultiSelect(selected)) if selected.contains(option))
        }
        Condition::DateBefore(bound) => date_of(value).is_some_and(|date| date < *bound),
        Condition::DateOn(bound) => date_of(value).is_some_and(|date| date == *bound),
        Condition::DateAfter(bound) => date_of(value).is_some_and(|date| date > *bound),
        Condition::CheckboxIs(expected) => match value {
            None => !expected,
            Some(PropertyValue::Checkbox(checked)) => checked == expected,
            Some(_) => false,
        },
    }
}

fn number_of(value: Option<&PropertyValue>) -> Option<f64> {
    match value {
        Some(PropertyValue::Number(number)) => Some(*number),
        _ => None,
    }
}

fn date_of(value: Option<&PropertyValue>) -> Option<Date> {
    match value {
        Some(PropertyValue::Date(date)) => Some(*date),
        _ => None,
    }
}

fn is_empty(value: Option<&PropertyValue>) -> bool {
    match value {
        None => true,
        Some(PropertyValue::Text(text) | PropertyValue::Url(text)) => text.is_empty(),
        Some(PropertyValue::MultiSelect(options)) => options.is_empty(),
        Some(_) => false,
    }
}

fn text_of(value: &PropertyValue) -> Option<&str> {
    match value {
        PropertyValue::Text(text) | PropertyValue::Url(text) => Some(text),
        _ => None,
    }
}

/// A value reduced to what sorting compares, computed once per row rather than per comparison.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) enum SortValue {
    Text(String),
    Number(f64),
    /// Select options sort in the order the property lists them.
    OptionIndexes(Vec<usize>),
    Date(Date),
    Checkbox(bool),
}

pub(crate) fn sort_value(property: &Property, value: &PropertyValue) -> SortValue {
    let option_index = |option| {
        property
            .options
            .iter()
            .position(|candidate| candidate.id == option)
            .unwrap_or(usize::MAX)
    };
    match value {
        PropertyValue::Text(text) | PropertyValue::Url(text) => {
            SortValue::Text(text.to_lowercase())
        }
        PropertyValue::Number(number) => SortValue::Number(*number),
        PropertyValue::Select(option) => SortValue::OptionIndexes(vec![option_index(*option)]),
        PropertyValue::MultiSelect(options) => {
            let mut indexes: Vec<usize> =
                options.iter().map(|option| option_index(*option)).collect();
            indexes.sort_unstable();
            SortValue::OptionIndexes(indexes)
        }
        PropertyValue::Date(date) => SortValue::Date(*date),
        PropertyValue::Checkbox(checked) => SortValue::Checkbox(*checked),
    }
}

/// One sort key of two rows. Empty values go last whichever way the key sorts.
pub(crate) fn compare_key(
    left: Option<&SortValue>,
    right: Option<&SortValue>,
    direction: SortDirection,
) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => {
            let ordering = compare_sort_values(left, right);
            match direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        }
    }
}

fn compare_sort_values(left: &SortValue, right: &SortValue) -> Ordering {
    match (left, right) {
        // Validation keeps numbers finite; total order keeps sorting sound if one slips through.
        (SortValue::Number(left), SortValue::Number(right)) => left.total_cmp(right),
        _ => left.partial_cmp(right).unwrap_or(Ordering::Equal),
    }
}
