//! The filter conditions offered for each property type, and building a `Condition` from what the
//! filter editor holds: a kind and the text typed for its argument.

use scroll_core::record::{Condition, Property, PropertyType};

use crate::values::{
    ParseError, display_text, format_date, format_number, parse_date, parse_number, value_from_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionKind {
    Contains,
    TextIs,
    NumberIs,
    GreaterThan,
    LessThan,
    SelectIs,
    MultiSelectContains,
    Before,
    On,
    After,
    IsChecked,
    IsUnchecked,
    IsEmpty,
    IsNotEmpty,
}

impl ConditionKind {
    pub fn for_type(property_type: PropertyType) -> &'static [ConditionKind] {
        use ConditionKind::*;
        match property_type {
            PropertyType::Text | PropertyType::Url => &[Contains, TextIs, IsEmpty, IsNotEmpty],
            PropertyType::Number => &[NumberIs, GreaterThan, LessThan, IsEmpty, IsNotEmpty],
            PropertyType::Select => &[SelectIs, IsEmpty, IsNotEmpty],
            PropertyType::MultiSelect => &[MultiSelectContains, IsEmpty, IsNotEmpty],
            PropertyType::Date => &[On, Before, After, IsEmpty, IsNotEmpty],
            PropertyType::Checkbox => &[IsChecked, IsUnchecked],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ConditionKind::Contains | ConditionKind::MultiSelectContains => "contains",
            ConditionKind::TextIs | ConditionKind::NumberIs | ConditionKind::SelectIs => "is",
            ConditionKind::GreaterThan => "is greater than",
            ConditionKind::LessThan => "is less than",
            ConditionKind::Before => "is before",
            ConditionKind::On => "is on",
            ConditionKind::After => "is after",
            ConditionKind::IsChecked => "is checked",
            ConditionKind::IsUnchecked => "is unchecked",
            ConditionKind::IsEmpty => "is empty",
            ConditionKind::IsNotEmpty => "is not empty",
        }
    }

    pub fn takes_argument(self) -> bool {
        !matches!(
            self,
            ConditionKind::IsChecked
                | ConditionKind::IsUnchecked
                | ConditionKind::IsEmpty
                | ConditionKind::IsNotEmpty
        )
    }

    pub fn of(condition: &Condition) -> ConditionKind {
        match condition {
            Condition::IsEmpty => ConditionKind::IsEmpty,
            Condition::IsNotEmpty => ConditionKind::IsNotEmpty,
            Condition::TextContains(_) => ConditionKind::Contains,
            Condition::TextEquals(_) => ConditionKind::TextIs,
            Condition::NumberEquals(_) => ConditionKind::NumberIs,
            Condition::NumberGreaterThan(_) => ConditionKind::GreaterThan,
            Condition::NumberLessThan(_) => ConditionKind::LessThan,
            Condition::SelectIs(_) => ConditionKind::SelectIs,
            Condition::MultiSelectContains(_) => ConditionKind::MultiSelectContains,
            Condition::DateBefore(_) => ConditionKind::Before,
            Condition::DateOn(_) => ConditionKind::On,
            Condition::DateAfter(_) => ConditionKind::After,
            Condition::CheckboxIs(true) => ConditionKind::IsChecked,
            Condition::CheckboxIs(false) => ConditionKind::IsUnchecked,
        }
    }

    /// The condition this kind means with `argument` typed for it.
    pub fn build(self, argument: &str, property: &Property) -> Result<Condition, ParseError> {
        let argument = argument.trim();
        Ok(match self {
            ConditionKind::Contains => Condition::TextContains(argument.into()),
            ConditionKind::TextIs => Condition::TextEquals(argument.into()),
            ConditionKind::NumberIs => Condition::NumberEquals(parse_number(argument)?),
            ConditionKind::GreaterThan => Condition::NumberGreaterThan(parse_number(argument)?),
            ConditionKind::LessThan => Condition::NumberLessThan(parse_number(argument)?),
            ConditionKind::SelectIs | ConditionKind::MultiSelectContains => {
                let option = option_for(argument, property)?;
                match self {
                    ConditionKind::SelectIs => Condition::SelectIs(option),
                    _ => Condition::MultiSelectContains(option),
                }
            }
            ConditionKind::Before => Condition::DateBefore(parse_date(argument)?),
            ConditionKind::On => Condition::DateOn(parse_date(argument)?),
            ConditionKind::After => Condition::DateAfter(parse_date(argument)?),
            ConditionKind::IsChecked => Condition::CheckboxIs(true),
            ConditionKind::IsUnchecked => Condition::CheckboxIs(false),
            ConditionKind::IsEmpty => Condition::IsEmpty,
            ConditionKind::IsNotEmpty => Condition::IsNotEmpty,
        })
    }
}

/// The text the editor shows for a saved condition's argument.
pub fn argument_text(condition: &Condition, property: &Property) -> String {
    use scroll_core::record::PropertyValue;
    match condition {
        Condition::TextContains(text) | Condition::TextEquals(text) => text.clone(),
        Condition::NumberEquals(number)
        | Condition::NumberGreaterThan(number)
        | Condition::NumberLessThan(number) => format_number(*number),
        Condition::SelectIs(option) | Condition::MultiSelectContains(option) => {
            display_text(&PropertyValue::Select(*option), property)
        }
        Condition::DateBefore(date) | Condition::DateOn(date) | Condition::DateAfter(date) => {
            format_date(*date)
        }
        Condition::CheckboxIs(_) | Condition::IsEmpty | Condition::IsNotEmpty => String::new(),
    }
}

/// Select options are named in the argument, the same way they are typed into a cell.
fn option_for(
    name: &str,
    property: &Property,
) -> Result<scroll_core::id::SelectOptionId, ParseError> {
    let as_select = Property {
        property_type: PropertyType::Select,
        ..property.clone()
    };
    match value_from_text(name, &as_select)? {
        Some(scroll_core::record::PropertyValue::Select(option)) => Ok(option),
        _ => Err(ParseError::UnknownOption(name.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::ALL_TYPES;
    use scroll_core::id::{PropertyId, SelectOptionId};
    use scroll_core::record::{Date, SelectOption};
    use scroll_core::validation::check_condition;

    fn property(property_type: PropertyType) -> Property {
        Property {
            id: PropertyId::generate(),
            name: "Property".into(),
            property_type,
            options: vec![SelectOption {
                id: SelectOptionId::generate(),
                name: "Done".into(),
            }],
        }
    }

    fn sample_argument(kind: ConditionKind) -> &'static str {
        match kind {
            ConditionKind::NumberIs | ConditionKind::GreaterThan | ConditionKind::LessThan => "4",
            ConditionKind::SelectIs | ConditionKind::MultiSelectContains => "done",
            ConditionKind::Before | ConditionKind::On | ConditionKind::After => "2026-11-09",
            _ => "text",
        }
    }

    #[test]
    fn every_offered_kind_builds_a_condition_the_store_accepts_and_reads_back() {
        for property_type in ALL_TYPES {
            let property = property(property_type);
            for kind in ConditionKind::for_type(property_type) {
                let condition = kind.build(sample_argument(*kind), &property).unwrap();
                assert_eq!(check_condition(&property, &condition), Ok(()), "{kind:?}");
                assert_eq!(ConditionKind::of(&condition), *kind);
                let text = argument_text(&condition, &property);
                assert_eq!(kind.build(&text, &property), Ok(condition), "{kind:?}");
            }
        }
    }

    #[test]
    fn bad_arguments_are_reported() {
        let number = property(PropertyType::Number);
        assert!(ConditionKind::GreaterThan.build("lots", &number).is_err());
        let select = property(PropertyType::Select);
        assert_eq!(
            ConditionKind::SelectIs.build("Blocked", &select),
            Err(ParseError::UnknownOption("Blocked".into()))
        );
        let date = property(PropertyType::Date);
        assert_eq!(
            ConditionKind::On.build("9 Nov 2026", &date),
            Ok(Condition::DateOn(Date {
                year: 2026,
                month: 11,
                day: 9
            }))
        );
    }
}
