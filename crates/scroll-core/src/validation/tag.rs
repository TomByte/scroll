//! Rules for tags: names are non-empty and unique ignoring case.

use super::ValidationError;
use crate::id::TagId;

/// What tag names are compared by: two names with the same key are the same name.
pub fn tag_name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

pub fn check_tag_name(name: &str) -> Result<(), ValidationError> {
    if name.trim().is_empty() {
        return Err(ValidationError::TagNameEmpty);
    }
    Ok(())
}

/// `holder` is the tag whose name has the same `tag_name_key` as `name`, if any. A tag may keep
/// its own name, for example when only the letter case changes.
pub fn check_tag_name_is_free(
    tag: TagId,
    name: &str,
    holder: Option<TagId>,
) -> Result<(), ValidationError> {
    match holder {
        Some(existing) if existing != tag => Err(ValidationError::TagNameTaken {
            name: name.to_owned(),
            existing,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_names_must_be_non_empty_and_unique_ignoring_case() {
        assert_eq!(check_tag_name("  "), Err(ValidationError::TagNameEmpty));
        assert_eq!(check_tag_name("Urgent"), Ok(()));
        assert_eq!(tag_name_key(" Urgent "), tag_name_key("URGENT"));
        assert_ne!(tag_name_key("Urgent"), tag_name_key("Urgently"));

        let tag = TagId::generate();
        let other = TagId::generate();
        assert_eq!(check_tag_name_is_free(tag, "urgent", None), Ok(()));
        assert_eq!(check_tag_name_is_free(tag, "urgent", Some(tag)), Ok(()));
        assert_eq!(
            check_tag_name_is_free(tag, "urgent", Some(other)),
            Err(ValidationError::TagNameTaken {
                name: "urgent".into(),
                existing: other
            })
        );
    }
}
