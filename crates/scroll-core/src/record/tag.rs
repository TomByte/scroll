//! Tags and which pages carry them.

use serde::{Deserialize, Serialize};

use crate::id::{PageId, TagId};

/// A label any page (database rows included) can carry. Names are unique within a workspace,
/// compared case-insensitively (`validation::tag_name_key`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    pub color: TagColor,
}

/// The fixed palette; the app maps each to theme colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TagColor {
    Gray,
    Brown,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Pink,
    Red,
}

/// A page carrying a tag. Each membership is its own record keyed by (page, tag), so two people
/// tagging the same page at once both keep their tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PageTag {
    pub page: PageId,
    pub tag: TagId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagField {
    Name(String),
    Color(TagColor),
}

impl TagField {
    pub fn apply_to(&self, tag: &mut Tag) {
        match self {
            TagField::Name(name) => tag.name.clone_from(name),
            TagField::Color(color) => tag.color = *color,
        }
    }
}
