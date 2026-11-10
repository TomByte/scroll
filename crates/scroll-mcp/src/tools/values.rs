//! Property values as the model reads and writes them: plain JSON by property type, with select
//! options named rather than numbered, and dates as "YYYY-MM-DD".

use scroll_core::id::{PropertyId, SelectOptionId};
use scroll_core::record::{Database, Date, Property, PropertyType, PropertyValue, value_for};
use serde_json::{Map, Value, json};

use super::ToolError;

/// The property called `name`, ignoring case.
pub(super) fn property_named<'a>(
    database: &'a Database,
    name: &str,
) -> Result<&'a Property, ToolError> {
    database
        .properties
        .iter()
        .find(|property| property.name.to_lowercase() == name.to_lowercase())
        .ok_or_else(|| ToolError::UnknownProperty(name.to_string()))
}

/// What the model writes for a property of this type, for the schema descriptions and errors.
pub(super) fn expected_form(property_type: PropertyType) -> &'static str {
    match property_type {
        PropertyType::Text | PropertyType::Url => "a string",
        PropertyType::Number => "a number",
        PropertyType::Select => "an option name",
        PropertyType::MultiSelect => "a list of option names",
        PropertyType::Date => "a date as \"YYYY-MM-DD\"",
        PropertyType::Checkbox => "true or false",
    }
}

/// The row's values by property name, for every property of the database (`null` when empty).
pub(super) fn values_to_json(
    database: &Database,
    values: &[(PropertyId, PropertyValue)],
) -> Map<String, Value> {
    database
        .properties
        .iter()
        .map(|property| {
            let value = value_for(values, property)
                .map_or(Value::Null, |value| value_to_json(property, value));
            (property.name.clone(), value)
        })
        .collect()
}

fn value_to_json(property: &Property, value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Text(text) | PropertyValue::Url(text) => json!(text),
        PropertyValue::Number(number) => json!(number),
        PropertyValue::Checkbox(is_checked) => json!(is_checked),
        PropertyValue::Date(date) => json!(format_date(*date)),
        PropertyValue::Select(option) => {
            option_name(property, *option).map_or(Value::Null, |name| json!(name))
        }
        PropertyValue::MultiSelect(options) => options
            .iter()
            .filter_map(|option| option_name(property, *option))
            .collect(),
    }
}

/// The value the model gave for `property`; `null` clears it.
pub(super) fn value_from_json(
    property: &Property,
    value: &Value,
) -> Result<Option<PropertyValue>, ToolError> {
    if value.is_null() {
        return Ok(None);
    }
    let value = match property.property_type {
        PropertyType::Text => PropertyValue::Text(text(property, value)?),
        PropertyType::Url => PropertyValue::Url(text(property, value)?),
        PropertyType::Number => PropertyValue::Number(number(property, value)?),
        PropertyType::Checkbox => PropertyValue::Checkbox(boolean(property, value)?),
        PropertyType::Date => PropertyValue::Date(date(property, value)?),
        PropertyType::Select => PropertyValue::Select(option(property, value)?),
        PropertyType::MultiSelect => {
            let names = value
                .as_array()
                .ok_or_else(|| wrong_value(property, value))?;
            let options = names
                .iter()
                .map(|name| option(property, name))
                .collect::<Result<_, _>>()?;
            PropertyValue::MultiSelect(options)
        }
    };
    Ok(Some(value))
}

pub(super) fn text(property: &Property, value: &Value) -> Result<String, ToolError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| wrong_value(property, value))
}

pub(super) fn number(property: &Property, value: &Value) -> Result<f64, ToolError> {
    value.as_f64().ok_or_else(|| wrong_value(property, value))
}

pub(super) fn boolean(property: &Property, value: &Value) -> Result<bool, ToolError> {
    value.as_bool().ok_or_else(|| wrong_value(property, value))
}

pub(super) fn date(property: &Property, value: &Value) -> Result<Date, ToolError> {
    value
        .as_str()
        .and_then(parse_date)
        .ok_or_else(|| wrong_value(property, value))
}

/// The option of a select or multi-select property named by `value`, ignoring case. Options are
/// not created here: adding one changes the database's schema.
pub(super) fn option(property: &Property, value: &Value) -> Result<SelectOptionId, ToolError> {
    let name = value.as_str().ok_or_else(|| wrong_value(property, value))?;
    property
        .options
        .iter()
        .find(|option| option.name.to_lowercase() == name.to_lowercase())
        .map(|option| option.id)
        .ok_or_else(|| ToolError::UnknownOption {
            property: property.name.clone(),
            option: name.to_string(),
            options: option_names(property).join(", "),
        })
}

pub(super) fn option_names(property: &Property) -> Vec<&str> {
    property
        .options
        .iter()
        .map(|option| option.name.as_str())
        .collect()
}

fn option_name(property: &Property, id: SelectOptionId) -> Option<&str> {
    property
        .options
        .iter()
        .find(|option| option.id == id)
        .map(|option| option.name.as_str())
}

fn wrong_value(property: &Property, value: &Value) -> ToolError {
    ToolError::WrongValue {
        property: property.name.clone(),
        expected: expected_form(property.property_type),
        value: value.clone(),
    }
}

fn format_date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

fn parse_date(text: &str) -> Option<Date> {
    let mut parts = text.splitn(3, '-');
    let date = Date {
        year: parts.next()?.parse().ok()?,
        month: parts.next()?.parse().ok()?,
        day: parts.next()?.parse().ok()?,
    };
    date.is_valid().then_some(date)
}

#[cfg(test)]
mod tests {
    use scroll_core::id::DatabaseId;
    use scroll_core::record::SelectOption;

    use super::*;

    fn property(property_type: PropertyType, options: &[&str]) -> Property {
        Property {
            id: PropertyId::generate(),
            name: "Status".into(),
            property_type,
            options: options
                .iter()
                .map(|name| SelectOption {
                    id: SelectOptionId::generate(),
                    name: (*name).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn values_round_trip_through_json_by_type() {
        let cases = [
            (property(PropertyType::Text, &[]), json!("ripe")),
            (
                property(PropertyType::Url, &[]),
                json!("https://scroll.example"),
            ),
            (property(PropertyType::Number, &[]), json!(2.5)),
            (property(PropertyType::Checkbox, &[]), json!(true)),
            (property(PropertyType::Date, &[]), json!("2026-02-28")),
            (
                property(PropertyType::Select, &["Todo", "Done"]),
                json!("Done"),
            ),
            (
                property(PropertyType::MultiSelect, &["Red", "Blue"]),
                json!(["Blue", "Red"]),
            ),
        ];
        for (property, given) in cases {
            let value = value_from_json(&property, &given).unwrap().unwrap();
            let database = Database {
                id: DatabaseId::generate(),
                host_page: scroll_core::id::PageId::generate(),
                properties: vec![property.clone()],
            };
            let values = values_to_json(&database, &[(property.id, value)]);
            assert_eq!(values["Status"], given, "{:?}", property.property_type);
        }
    }

    #[test]
    fn options_are_named_ignoring_case_and_never_made_up() {
        let status = property(PropertyType::Select, &["Todo", "Done"]);
        assert_eq!(
            value_from_json(&status, &json!("done")).unwrap(),
            Some(PropertyValue::Select(status.options[1].id))
        );
        let error = value_from_json(&status, &json!("Blocked")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"Status\" has no option called \"Blocked\"; its options are: Todo, Done"
        );
    }

    #[test]
    fn wrong_forms_and_impossible_dates_are_refused_and_null_clears() {
        let date = property(PropertyType::Date, &[]);
        for given in [json!("2026-02-30"), json!("tomorrow"), json!(20260101)] {
            assert!(value_from_json(&date, &given).is_err(), "{given}");
        }
        assert_eq!(value_from_json(&date, &Value::Null).unwrap(), None);
        let number = property(PropertyType::Number, &[]);
        assert!(value_from_json(&number, &json!("3")).is_err());
    }
}
