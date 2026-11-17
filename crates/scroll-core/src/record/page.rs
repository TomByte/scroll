//! Pages: the tree the sidebar shows, and database rows.

use serde::{Deserialize, Serialize};

use crate::id::{DatabaseId, PageId};
use crate::position::Position;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "EncodedPage", into = "EncodedPage")]
pub struct Page {
    pub id: PageId,
    pub parent: Option<PageId>,
    pub title: String,
    pub icon: Option<String>,
    /// Order among pages with the same parent.
    pub position: Position,
    pub row_of: Option<DatabaseId>,
    pub is_archived: bool,
    /// A folder groups the pages under it in the tree and has no content of its own. Decided
    /// when the page is made; there is no field change for it.
    pub is_folder: bool,
}

/// A page as it is encoded: its fields in order, with its yes-or-no ones packed into one byte.
/// That byte stands where `is_archived` was encoded as a `bool` (one byte, 0 or 1) before pages
/// could be folders, so everything written then (queued transactions, confirmed records, the
/// server's records and its transaction log) still reads as it was, with no rewrite.
#[derive(Serialize, Deserialize)]
struct EncodedPage {
    id: PageId,
    parent: Option<PageId>,
    title: String,
    icon: Option<String>,
    position: Position,
    row_of: Option<DatabaseId>,
    flags: u8,
}

const ARCHIVED_FLAG: u8 = 1;
const FOLDER_FLAG: u8 = 1 << 1;

impl From<Page> for EncodedPage {
    fn from(page: Page) -> EncodedPage {
        let flag = |is_set: bool, flag: u8| if is_set { flag } else { 0 };
        EncodedPage {
            id: page.id,
            parent: page.parent,
            title: page.title,
            icon: page.icon,
            position: page.position,
            row_of: page.row_of,
            flags: flag(page.is_archived, ARCHIVED_FLAG) | flag(page.is_folder, FOLDER_FLAG),
        }
    }
}

impl From<EncodedPage> for Page {
    fn from(page: EncodedPage) -> Page {
        Page {
            id: page.id,
            parent: page.parent,
            title: page.title,
            icon: page.icon,
            position: page.position,
            row_of: page.row_of,
            is_archived: page.flags & ARCHIVED_FLAG != 0,
            is_folder: page.flags & FOLDER_FLAG != 0,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A page as every version before folders encoded it.
    #[derive(Serialize)]
    struct PageBeforeFolders {
        id: PageId,
        parent: Option<PageId>,
        title: String,
        icon: Option<String>,
        position: Position,
        row_of: Option<DatabaseId>,
        is_archived: bool,
    }

    fn page(is_archived: bool, is_folder: bool) -> Page {
        Page {
            id: PageId::generate(),
            parent: Some(PageId::generate()),
            title: "Plans".into(),
            icon: Some("🗂".into()),
            position: Position::between(None, None).unwrap(),
            row_of: None,
            is_archived,
            is_folder,
        }
    }

    #[test]
    fn pages_encoded_before_folders_read_as_they_were() {
        for is_archived in [false, true] {
            let new = page(is_archived, false);
            let old = PageBeforeFolders {
                id: new.id,
                parent: new.parent,
                title: new.title.clone(),
                icon: new.icon.clone(),
                position: new.position.clone(),
                row_of: new.row_of,
                is_archived,
            };
            let old_bytes = postcard::to_stdvec(&old).unwrap();
            assert_eq!(postcard::from_bytes::<Page>(&old_bytes).unwrap(), new);
            assert_eq!(
                postcard::to_stdvec(&new).unwrap(),
                old_bytes,
                "and a page that is not a folder is still written the same way"
            );
        }
    }

    #[test]
    fn a_folder_round_trips_archived_or_not() {
        for is_archived in [false, true] {
            let folder = page(is_archived, true);
            let bytes = postcard::to_stdvec(&folder).unwrap();
            assert_eq!(postcard::from_bytes::<Page>(&bytes).unwrap(), folder);
        }
    }

    #[test]
    fn a_page_inside_a_larger_encoding_still_ends_where_it_did() {
        // A transaction holds pages between other values; one more or one fewer byte per page
        // would shift everything after it.
        let pages = vec![page(true, true), page(false, false), page(false, true)];
        let bytes = postcard::to_stdvec(&(pages.clone(), 7u32)).unwrap();
        let (decoded, after): (Vec<Page>, u32) = postcard::from_bytes(&bytes).unwrap();
        assert_eq!((decoded, after), (pages, 7));
    }
}
