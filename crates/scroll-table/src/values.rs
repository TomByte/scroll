//! Property values as people read and type them: the text a value shows as, the value typed or
//! pasted text stands for, the icon and name of each property type, and the picker for select
//! values. The table and the app's row properties both use these, so the two show and accept the
//! same formats.

use gpui::Context;
use scroll_core::id::SelectOptionId;
use scroll_core::record::{Date, Property, PropertyType, PropertyValue, SelectOption};
use scroll_ui::IconName;
use scroll_ui::{Combobox, ComboboxOption};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("“{0}” is not a number")]
    NotANumber(String),
    #[error("“{0}” is not a date; use a form like 2026-11-09 or 9 Nov 2026")]
    NotADate(String),
    #[error("“{0}” is not yes or no")]
    NotYesOrNo(String),
    #[error("there is no option called “{0}”")]
    UnknownOption(String),
}

/// The icon shown next to a property's name.
pub fn type_icon(property_type: PropertyType) -> IconName {
    match property_type {
        PropertyType::Text => IconName::Text,
        PropertyType::Url => IconName::Link,
        PropertyType::Checkbox => IconName::CheckSquare,
        PropertyType::MultiSelect => IconName::List,
        PropertyType::Number => IconName::Hash,
        PropertyType::Date => IconName::Calendar,
        PropertyType::Select => IconName::Select,
    }
}

pub fn type_label(property_type: PropertyType) -> &'static str {
    match property_type {
        PropertyType::Text => "Text",
        PropertyType::Number => "Number",
        PropertyType::Select => "Select",
        PropertyType::MultiSelect => "Multi-select",
        PropertyType::Date => "Date",
        PropertyType::Checkbox => "Checkbox",
        PropertyType::Url => "URL",
    }
}

/// Every property type, in the order menus list them.
pub const ALL_TYPES: [PropertyType; 7] = PropertyType::ALL;

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// How a value reads in a cell and on the clipboard. Option ids missing from the schema read as empty.
pub fn display_text(value: &PropertyValue, property: &Property) -> String {
    match value {
        PropertyValue::Text(text) | PropertyValue::Url(text) => text.clone(),
        PropertyValue::Number(number) => format_number(*number),
        PropertyValue::Date(date) => format_date(*date),
        PropertyValue::Checkbox(true) => "Yes".into(),
        PropertyValue::Checkbox(false) => "No".into(),
        PropertyValue::Select(option) => option_name(property, *option).unwrap_or_default(),
        PropertyValue::MultiSelect(options) => options
            .iter()
            .filter_map(|option| option_name(property, *option))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The value typed or pasted text means for the property; empty text clears the cell.
pub fn value_from_text(
    text: &str,
    property: &Property,
) -> Result<Option<PropertyValue>, ParseError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value = match property.property_type {
        PropertyType::Text => PropertyValue::Text(text.into()),
        PropertyType::Url => PropertyValue::Url(text.into()),
        PropertyType::Number => PropertyValue::Number(parse_number(text)?),
        PropertyType::Date => PropertyValue::Date(parse_date(text)?),
        PropertyType::Checkbox => PropertyValue::Checkbox(parse_yes_or_no(text)?),
        PropertyType::Select => PropertyValue::Select(option_named(property, text)?),
        PropertyType::MultiSelect => PropertyValue::MultiSelect(
            text.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| option_named(property, name))
                .collect::<Result<_, _>>()?,
        ),
    };
    Ok(Some(value))
}

/// Accepts thousands separators ("1,234.5"). Rejects infinities and NaN, which the store refuses.
pub fn parse_number(text: &str) -> Result<f64, ParseError> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|character| *character != ',')
        .collect();
    match cleaned.parse::<f64>() {
        Ok(number) if number.is_finite() => Ok(number),
        _ => Err(ParseError::NotANumber(text.into())),
    }
}

/// Whole numbers without a decimal point; others with at most six decimals, trailing zeros trimmed.
pub fn format_number(number: f64) -> String {
    let formatted = format!("{number:.6}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    match trimmed {
        "-0" => "0".into(),
        _ => trimmed.into(),
    }
}

/// Accepts "2026-11-09", "9 Nov 2026" and "Nov 9, 2026" (month names in any case, full or short).
pub fn parse_date(text: &str) -> Result<Date, ParseError> {
    let not_a_date = || ParseError::NotADate(text.into());
    let parts: Vec<&str> = text
        .split(|character: char| character == '-' || character == ',' || character.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    let [first, second, third] = parts[..] else {
        return Err(not_a_date());
    };
    let date = match (
        first.parse::<i32>(),
        month_number(first),
        month_number(second),
    ) {
        (Ok(year), _, _) if first.len() == 4 => Date {
            year,
            month: second.parse().map_err(|_| not_a_date())?,
            day: third.parse().map_err(|_| not_a_date())?,
        },
        (Ok(day), _, Some(month)) => Date {
            year: third.parse().map_err(|_| not_a_date())?,
            month,
            day: u8::try_from(day).map_err(|_| not_a_date())?,
        },
        (_, Some(month), _) => Date {
            year: third.parse().map_err(|_| not_a_date())?,
            month,
            day: second.parse().map_err(|_| not_a_date())?,
        },
        _ => return Err(not_a_date()),
    };
    if !date.is_valid() {
        return Err(not_a_date());
    }
    Ok(date)
}

/// "9 Nov 2026".
pub fn format_date(date: Date) -> String {
    let month = MONTH_NAMES[usize::from(date.month.clamp(1, 12)) - 1];
    format!("{} {month} {}", date.day, date.year)
}

fn month_number(text: &str) -> Option<u8> {
    let lowercase = text.to_lowercase();
    let index = MONTH_NAMES
        .iter()
        .position(|name| lowercase.starts_with(&name.to_lowercase()))?;
    u8::try_from(index + 1).ok()
}

fn parse_yes_or_no(text: &str) -> Result<bool, ParseError> {
    match text.to_lowercase().as_str() {
        "yes" | "true" | "1" | "x" | "✓" | "checked" => Ok(true),
        "no" | "false" | "0" | "unchecked" => Ok(false),
        _ => Err(ParseError::NotYesOrNo(text.into())),
    }
}

fn option_name(property: &Property, option: SelectOptionId) -> Option<String> {
    property
        .options
        .iter()
        .find(|candidate| candidate.id == option)
        .map(|candidate| candidate.name.clone())
}

fn option_named(property: &Property, name: &str) -> Result<SelectOptionId, ParseError> {
    property
        .options
        .iter()
        .find(|option| option.name.eq_ignore_ascii_case(name))
        .map(|option| option.id)
        .ok_or_else(|| ParseError::UnknownOption(name.into()))
}

/// The picker for a select or multi-select property's value: search, choose, clear, or ask for
/// an option it does not have yet (`ComboboxEvent::CreateRequested`: add it with
/// `with_new_option`, save the property, then call `set_options` and `select_created` on the
/// picker). Its `ComboboxEvent::Changed` indexes become a value with `value_of_chosen`.
pub fn select_picker(
    property: &Property,
    value: Option<&PropertyValue>,
    cx: &mut Context<Combobox>,
) -> Combobox {
    let picker = Combobox::new(cx)
        .options(select_options(property))
        .creatable(cx)
        .clearable()
        .chosen(chosen_options(property, value));
    match property.property_type {
        PropertyType::MultiSelect => picker.multiple(),
        _ => picker,
    }
}

/// The property's options as a picker lists them: by name, each in the swatch of its place in
/// the schema, as cells show them.
pub fn select_options(property: &Property) -> Vec<ComboboxOption> {
    property
        .options
        .iter()
        .enumerate()
        .map(|(index, option)| ComboboxOption {
            label: option.name.clone().into(),
            swatch: Some(index),
        })
        .collect()
}

/// Where the value's options are among the property's options, in the value's order. Options
/// the schema no longer has are left out; a value of another type chooses nothing.
pub fn chosen_options(property: &Property, value: Option<&PropertyValue>) -> Vec<usize> {
    let chosen: &[SelectOptionId] = match value {
        Some(PropertyValue::Select(option)) => std::slice::from_ref(option),
        Some(PropertyValue::MultiSelect(options)) => options,
        _ => &[],
    };
    chosen
        .iter()
        .filter_map(|id| property.options.iter().position(|option| option.id == *id))
        .collect()
}

/// The value for the options at `chosen` (places in the schema); choosing nothing clears it.
pub fn value_of_chosen(property: &Property, chosen: &[usize]) -> Option<PropertyValue> {
    let options: Vec<SelectOptionId> = chosen
        .iter()
        .filter_map(|index| property.options.get(*index))
        .map(|option| option.id)
        .collect();
    match (property.property_type, options.as_slice()) {
        (_, []) => None,
        (PropertyType::Select, [first, ..]) => Some(PropertyValue::Select(*first)),
        _ => Some(PropertyValue::MultiSelect(options)),
    }
}

/// The property with an option named `name` (trimmed) appended, and the new option's id.
pub fn with_new_option(property: &Property, name: &str) -> (Property, SelectOptionId) {
    let option = SelectOption {
        id: SelectOptionId::generate(),
        name: name.trim().into(),
    };
    let id = option.id;
    let mut updated = property.clone();
    updated.options.push(option);
    (updated, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scroll_core::id::PropertyId;
    use scroll_core::record::SelectOption;

    fn property(property_type: PropertyType) -> Property {
        Property {
            id: PropertyId::generate(),
            name: "Property".into(),
            property_type,
            options: ["Red", "Green"]
                .into_iter()
                .map(|name| SelectOption {
                    id: SelectOptionId::generate(),
                    name: name.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn chosen_options_keep_the_value_s_order_and_become_the_value_again() {
        let multi = property(PropertyType::MultiSelect);
        let (red, green) = (multi.options[0].id, multi.options[1].id);
        let missing = SelectOptionId::generate();
        let value = PropertyValue::MultiSelect(vec![green, missing, red]);
        let chosen = chosen_options(&multi, Some(&value));
        assert_eq!(chosen, vec![1, 0]);
        assert_eq!(
            value_of_chosen(&multi, &chosen),
            Some(PropertyValue::MultiSelect(vec![green, red]))
        );
        assert_eq!(value_of_chosen(&multi, &[]), None);
        assert!(chosen_options(&multi, Some(&PropertyValue::Number(1.0))).is_empty());

        let select = property(PropertyType::Select);
        let chosen = chosen_options(&select, Some(&PropertyValue::Select(select.options[1].id)));
        assert_eq!(chosen, vec![1]);
        assert_eq!(
            value_of_chosen(&select, &[1, 0]),
            Some(PropertyValue::Select(select.options[1].id))
        );
        let swatches: Vec<Option<usize>> = select_options(&select)
            .into_iter()
            .map(|option| option.swatch)
            .collect();
        assert_eq!(swatches, vec![Some(0), Some(1)]);
    }

    #[test]
    fn a_new_option_is_appended_with_its_name_trimmed() {
        let select = property(PropertyType::Select);
        let (updated, option) = with_new_option(&select, "  Doing ");
        assert_eq!(updated.options.len(), 3);
        assert_eq!(updated.options[2].id, option);
        assert_eq!(updated.options[2].name, "Doing");
    }

    #[test]
    fn numbers_parse_with_separators_and_format_without_noise() {
        assert_eq!(parse_number(" 1,234.5 "), Ok(1234.5));
        assert_eq!(parse_number("-3"), Ok(-3.0));
        assert!(parse_number("12abc").is_err());
        assert!(parse_number("inf").is_err());
        assert!(parse_number("NaN").is_err());
        assert_eq!(format_number(1234.5), "1234.5");
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
    }

    #[test]
    fn dates_parse_in_common_forms_and_reject_impossible_days() {
        let expected = Date {
            year: 2026,
            month: 11,
            day: 9,
        };
        for text in [
            "2026-11-09",
            "9 Nov 2026",
            "Nov 9, 2026",
            "november 9 2026",
            "9 NOV 2026",
        ] {
            assert_eq!(parse_date(text), Ok(expected), "{text}");
        }
        for text in [
            "2026-02-30",
            "30 Feb 2026",
            "tomorrow",
            "9/11/2026",
            "2026-11",
        ] {
            assert!(parse_date(text).is_err(), "{text}");
        }
        assert_eq!(format_date(expected), "9 Nov 2026");
        assert_eq!(parse_date(&format_date(expected)), Ok(expected));
    }

    #[test]
    fn text_becomes_the_value_its_property_expects() {
        let checkbox = property(PropertyType::Checkbox);
        assert_eq!(
            value_from_text("yes", &checkbox),
            Ok(Some(PropertyValue::Checkbox(true)))
        );
        assert!(value_from_text("maybe", &checkbox).is_err());

        let select = property(PropertyType::Select);
        let green = select.options[1].id;
        assert_eq!(
            value_from_text("green", &select),
            Ok(Some(PropertyValue::Select(green)))
        );
        assert_eq!(
            value_from_text("Blue", &select),
            Err(ParseError::UnknownOption("Blue".into()))
        );

        let multi = property(PropertyType::MultiSelect);
        let red = multi.options[0].id;
        let green = multi.options[1].id;
        assert_eq!(
            value_from_text("Red, Green", &multi),
            Ok(Some(PropertyValue::MultiSelect(vec![red, green])))
        );
        assert_eq!(value_from_text("   ", &multi), Ok(None));
    }

    #[test]
    fn display_text_round_trips_through_value_from_text() {
        let multi = property(PropertyType::MultiSelect);
        let value = PropertyValue::MultiSelect(vec![multi.options[1].id, multi.options[0].id]);
        let text = display_text(&value, &multi);
        assert_eq!(text, "Green, Red");
        assert_eq!(value_from_text(&text, &multi), Ok(Some(value)));

        let number = property(PropertyType::Number);
        assert_eq!(display_text(&PropertyValue::Number(2.5), &number), "2.5");
    }
}
