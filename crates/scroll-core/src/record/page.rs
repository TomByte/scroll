//! Pages: the tree the sidebar shows, and database rows.

use serde::{Deserialize, Serialize};

use crate::id::{DatabaseId, PageId};
use crate::position::Position;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    pub parent: Option<PageId>,
    pub title: String,
    pub icon: Option<String>,
    /// Order among pages with the same parent.
    pub position: Position,
    pub row_of: Option<DatabaseId>,
    pub is_archived: bool,
}

impl Page {
    /// Whether the page shows in the sidebar's tree: database rows and archived pages do not.
    /// The store's SQL form of this rule is the `in_tree!` macro in `store::kinds::page`.
    pub(crate) fn is_in_tree(&self) -> bool {
        self.row_of.is_none() && !self.is_archived
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageField {
    Parent(Option<PageId>),
    Title(String),
    Icon(Option<String>),
    Position(Position),
    RowOf(Option<DatabaseId>),
    IsArchived(bool),
}

impl PageField {
    pub fn apply_to(&self, page: &mut Page) {
        match self {
            PageField::Parent(parent) => page.parent = *parent,
            PageField::Title(title) => page.title.clone_from(title),
            PageField::Icon(icon) => page.icon.clone_from(icon),
            PageField::Position(position) => page.position = position.clone(),
            PageField::RowOf(database) => page.row_of = *database,
            PageField::IsArchived(is_archived) => page.is_archived = *is_archived,
        }
    }
}
