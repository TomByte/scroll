//! Rules for property values: they must suit their property.

use super::ValidationError;
use crate::id::SelectOptionId;
use crate::record::{Property, PropertyValue, fits};

pub fn check_property_value(
    property: &Property,
    value: &PropertyValue,
) -> Result<(), ValidationError> {
    if !fits(value, property) {
        return Err(ValidationError::PropertyTypeMismatch {
            property: property.id,
            expected: property.property_type,
            actual: value.property_type(),
        });
    }
    match value {
        PropertyValue::Number(number) if !number.is_finite() => {
            Err(ValidationError::NumberNotFinite(property.id))
        }
        PropertyValue::Date(date) if !date.is_valid() => Err(ValidationError::InvalidDate(*date)),
        PropertyValue::Select(option) => check_option_exists(property, *option),
        PropertyValue::MultiSelect(options) => options
            .iter()
            .try_for_each(|option| check_option_exists(property, *option)),
        _ => Ok(()),
    }
}

pub(super) fn check_option_exists(
    property: &Property,
    option: SelectOptionId,
) -> Result<(), ValidationError> {
    if !property
        .options
        .iter()
        .any(|candidate| candidate.id == option)
    {
        return Err(ValidationError::SelectOptionNotFound {
            property: property.id,
            option,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SelectOptionId;
    use crate::record::{Date, PropertyType};
    use crate::validation::test_builders::select_property;

    #[test]
    fn a_value_must_match_its_property_type() {
        let property = select_property();
        let error = check_property_value(&property, &PropertyValue::Text("x".into())).unwrap_err();
        assert_eq!(
            error,
            ValidationError::PropertyTypeMismatch {
                property: property.id,
                expected: PropertyType::Select,
                actual: PropertyType::Text,
            }
        );
    }

    #[test]
    fn select_values_must_name_an_existing_option() {
        let property = select_property();
        let known = property.options[0].id;
        let unknown = SelectOptionId::generate();

        assert_eq!(
            check_property_value(&property, &PropertyValue::Select(known)),
            Ok(())
        );
        assert_eq!(
            check_property_value(&property, &PropertyValue::Select(unknown)),
            Err(ValidationError::SelectOptionNotFound {
                property: property.id,
                option: unknown
            })
        );

        let multi = Property {
            property_type: PropertyType::MultiSelect,
            ..property
        };
        assert_eq!(
            check_property_value(&multi, &PropertyValue::MultiSelect(vec![known, unknown])),
            Err(ValidationError::SelectOptionNotFound {
                property: multi.id,
                option: unknown
            })
        );
    }

    #[test]
    fn numbers_must_be_finite_and_dates_real() {
        let number = Property {
            property_type: PropertyType::Number,
            options: vec![],
            ..select_property()
        };
        assert_eq!(
            check_property_value(&number, &PropertyValue::Number(f64::NAN)),
            Err(ValidationError::NumberNotFinite(number.id))
        );
        assert_eq!(
            check_property_value(&number, &PropertyValue::Number(2.5)),
            Ok(())
        );

        let date_property = Property {
            property_type: PropertyType::Date,
            ..number
        };
        let bad_date = Date {
            year: 2025,
            month: 2,
            day: 30,
        };
        assert_eq!(
            check_property_value(&date_property, &PropertyValue::Date(bad_date)),
            Err(ValidationError::InvalidDate(bad_date))
        );
    }
}
