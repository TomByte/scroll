//! The part of the page tree loaded into memory, plus the transactions that change it. A large
//! workspace is never read whole: the top level loads first, each parent's children when it is
//! expanded, and an opened page with its ancestors, so every loaded page has its ancestors
//! loaded too. Changes are `Transaction`s applied here at once (so the UI never waits) and then
//! by the store; each touches only the lists it changes.

use std::collections::{HashMap, HashSet};

use scroll_core::id::PageId;
use scroll_core::operation::{FieldChange, Operation, PageField, Record, RecordId, Transaction};
use scroll_core::position::{Position, PositionError};
use scroll_core::record::Page;

pub const UNTITLED: &str = "Untitled";

/// One visible row of the sidebar tree. Titles are read from the tree when the row is drawn,
/// so building the rows of a 50,000-child parent copies no text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub id: PageId,
    pub depth: usize,
    pub has_children: bool,
}

/// Where a dragged page lands relative to the page under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageDrop {
    Before(PageId),
    After(PageId),
    Inside(PageId),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoveError {
    #[error("a page cannot be moved inside itself")]
    IntoItself,
    #[error("page {0:?} is not loaded")]
    UnknownPage(PageId),
    #[error("the pages under the destination are not loaded yet")]
    ChildrenNotLoaded,
    #[error(transparent)]
    Position(#[from] PositionError),
}

#[derive(Clone, Debug)]
pub struct PageTree {
    /// Every loaded page: tree pages, opened pages and their ancestors.
    pages: HashMap<PageId, Page>,
    /// The live, non-row children of each parent whose children are loaded, in sibling order.
    children: HashMap<Option<PageId>, Vec<PageId>>,
    /// How many live, non-row children each loaded page has, loaded or not.
    child_counts: HashMap<PageId, usize>,
}

impl Default for PageTree {
    /// An empty workspace: the top level is loaded and has nothing in it.
    fn default() -> Self {
        PageTree {
            pages: HashMap::new(),
            children: HashMap::from([(None, Vec::new())]),
            child_counts: HashMap::new(),
        }
    }
}

/// Live pages that are not database rows are the ones the tree lists.
fn is_listed(page: &Page) -> bool {
    !page.is_archived && page.row_of.is_none()
}

impl PageTree {
    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.pages.get(&id)
    }

    /// Whether the top level has no pages (as far as is loaded).
    pub fn is_empty(&self) -> bool {
        self.children.get(&None).is_none_or(Vec::is_empty)
    }

    /// Parents whose children are loaded, the top level first.
    pub fn loaded_parents(&self) -> Vec<Option<PageId>> {
        let mut parents: Vec<Option<PageId>> = self.children.keys().copied().collect();
        parents.sort_by_key(|parent| parent.is_some());
        parents
    }

    pub fn are_children_loaded(&self, parent: Option<PageId>) -> bool {
        self.children.contains_key(&parent)
    }

    /// Replaces `parent`'s child list with `children` (each with its own child count), the
    /// first batch as the store listed them.
    pub fn load_children(&mut self, parent: Option<PageId>, children: Vec<(Page, usize)>) {
        self.children
            .insert(parent, Vec::with_capacity(children.len()));
        self.append_children(parent, children);
    }

    /// Adds a further batch of `parent`'s children, which follow those already loaded.
    pub fn append_children(&mut self, parent: Option<PageId>, children: Vec<(Page, usize)>) {
        let mut ids = Vec::with_capacity(children.len());
        for (page, child_count) in children {
            let id = page.id;
            ids.push(id);
            self.child_counts.insert(id, child_count);
            if child_count == 0 {
                self.children.entry(Some(id)).or_default();
            }
            self.pages.insert(id, page);
        }
        self.children.entry(parent).or_default().extend(ids);
    }

    /// Records how many children `id` has, unless its children are loaded and counted already.
    pub fn set_child_count(&mut self, id: PageId, count: usize) {
        if !self.children.contains_key(&Some(id)) {
            self.child_counts.insert(id, count);
        }
    }

    /// Adds pages read from the store, such as an opened page and its ancestors.
    pub fn insert_pages(&mut self, pages: impl IntoIterator<Item = Page>) {
        for page in pages {
            self.upsert(page);
        }
    }

    /// Whether the page is loaded and neither it nor an ancestor is archived.
    pub fn is_live(&self, id: PageId) -> bool {
        self.ancestors_and_self(id).iter().all(|ancestor| {
            self.pages
                .get(ancestor)
                .is_some_and(|page| !page.is_archived)
        }) && self.pages.contains_key(&id)
    }

    /// Loaded children of `parent`, in sibling order; empty if they are not loaded.
    pub fn children(&self, parent: Option<PageId>) -> Vec<&Page> {
        self.children
            .get(&parent)
            .map(|ids| ids.iter().filter_map(|id| self.pages.get(id)).collect())
            .unwrap_or_default()
    }

    pub fn child_count(&self, id: PageId) -> usize {
        match self.children.get(&Some(id)) {
            Some(children) => children.len(),
            None => self.child_counts.get(&id).copied().unwrap_or(0),
        }
    }

    /// The sidebar's rows: loaded pages depth-first, descending only into `expanded` parents
    /// whose children are loaded. Costs one step per visible row.
    pub fn visible_rows(&self, expanded: &HashSet<PageId>) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        let mut stack: Vec<(PageId, usize)> = self
            .children
            .get(&None)
            .map(|ids| ids.iter().rev().map(|id| (*id, 0)).collect())
            .unwrap_or_default();
        while let Some((id, depth)) = stack.pop() {
            rows.push(TreeRow {
                id,
                depth,
                has_children: self.child_count(id) > 0,
            });
            if expanded.contains(&id)
                && let Some(children) = self.children.get(&Some(id))
            {
                stack.extend(children.iter().rev().map(|child| (*child, depth + 1)));
            }
        }
        rows
    }

    /// The page and its loaded ancestors, nearest first.
    pub fn ancestors_and_self(&self, id: PageId) -> Vec<PageId> {
        let mut chain = vec![id];
        let mut current = self.pages.get(&id).and_then(|page| page.parent);
        while let Some(parent) = current {
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            current = self.pages.get(&parent).and_then(|page| page.parent);
        }
        chain
    }

    pub fn first_live_page(&self) -> Option<PageId> {
        self.children.get(&None)?.first().copied()
    }

    /// Applies a transaction to the loaded pages, as the store will.
    pub fn apply(&mut self, transaction: &Transaction) {
        for operation in &transaction.operations {
            match operation {
                Operation::Create(Record::Page(page)) => {
                    self.children.entry(Some(page.id)).or_default();
                    self.upsert(page.clone());
                }
                Operation::Set(FieldChange::Page { id, field }) => {
                    if let Some(mut page) = self.pages.get(id).cloned() {
                        field.apply_to(&mut page);
                        self.upsert(page);
                    }
                }
                Operation::Delete(RecordId::Page(id)) => self.remove(*id),
                _ => {}
            }
        }
    }

    /// Stores `page`, moving it between child lists if its place or state changed.
    pub fn upsert(&mut self, page: Page) {
        let previous = self.pages.get(&page.id).cloned();
        let was_listed_under = previous
            .as_ref()
            .filter(|previous| is_listed(previous))
            .map(|previous| previous.parent);
        let is_listed_under = is_listed(&page).then_some(page.parent);
        let has_moved = was_listed_under != is_listed_under
            || previous
                .as_ref()
                .is_some_and(|previous| previous.position != page.position);
        let id = page.id;
        self.pages.insert(id, page);
        if !has_moved && previous.is_some() {
            return;
        }
        if let Some(old_parent) = was_listed_under {
            self.unlist(id, old_parent);
        }
        if let Some(new_parent) = is_listed_under {
            self.list(id, new_parent);
        }
    }

    /// Removes a page that was deleted or is gone from the store.
    pub fn remove(&mut self, id: PageId) {
        if let Some(page) = self.pages.remove(&id)
            && is_listed(&page)
        {
            self.unlist(id, page.parent);
        }
        self.children.remove(&Some(id));
        self.child_counts.remove(&id);
    }

    fn unlist(&mut self, id: PageId, parent: Option<PageId>) {
        if let Some(siblings) = self.children.get_mut(&parent) {
            siblings.retain(|sibling| *sibling != id);
        }
        if let Some(parent) = parent
            && let Some(count) = self.child_counts.get_mut(&parent)
        {
            *count = count.saturating_sub(1);
        }
    }

    /// Puts `id` in its parent's list at its position: a binary search, so inserting into a
    /// parent of 50,000 children costs one search and one shift.
    fn list(&mut self, id: PageId, parent: Option<PageId>) {
        if let Some(parent) = parent {
            *self.child_counts.entry(parent).or_default() += 1;
        }
        let Some(page) = self.pages.get(&id) else {
            return;
        };
        let key = (page.position.clone(), id);
        let pages = &self.pages;
        if let Some(siblings) = self.children.get_mut(&parent) {
            let index = siblings
                .binary_search_by(|sibling| {
                    let sibling_page = &pages[sibling];
                    (&sibling_page.position, *sibling).cmp(&(&key.0, key.1))
                })
                .unwrap_or_else(|index| index);
            siblings.insert(index, id);
        }
    }

    /// A new page as the last child of `parent`, whose children must be loaded.
    pub fn create_page(
        &self,
        parent: Option<PageId>,
        title: &str,
    ) -> Result<(PageId, Transaction), MoveError> {
        let siblings = self
            .children
            .get(&parent)
            .ok_or(MoveError::ChildrenNotLoaded)?;
        let last = siblings
            .last()
            .and_then(|id| self.pages.get(id))
            .map(|page| page.position.clone());
        let page = Page {
            id: PageId::generate(),
            parent,
            title: title.to_string(),
            icon: None,
            position: Position::between(last.as_ref(), None)?,
            row_of: None,
            is_archived: false,
        };
        let id = page.id;
        Ok((
            id,
            Transaction::new(vec![Operation::Create(Record::Page(page))]),
        ))
    }

    /// `None` removes the icon.
    pub fn set_icon(&self, id: PageId, icon: Option<String>) -> Transaction {
        set_field(id, PageField::Icon(icon))
    }

    pub fn set_archived(&self, id: PageId, is_archived: bool) -> Transaction {
        set_field(id, PageField::IsArchived(is_archived))
    }

    /// Moves `id` to the drop location: a new parent (if it changed) and a position between
    /// its new neighbours, so no sibling is renumbered. The destination's children must be
    /// loaded.
    pub fn move_page(&self, id: PageId, drop: PageDrop) -> Result<Transaction, MoveError> {
        let target = match drop {
            PageDrop::Before(target) | PageDrop::After(target) | PageDrop::Inside(target) => target,
        };
        let target_page = self
            .pages
            .get(&target)
            .ok_or(MoveError::UnknownPage(target))?;
        if self.ancestors_and_self(target).contains(&id) {
            return Err(MoveError::IntoItself);
        }
        let new_parent = match drop {
            PageDrop::Inside(target) => Some(target),
            PageDrop::Before(_) | PageDrop::After(_) => target_page.parent,
        };
        let siblings: Vec<&Page> = self
            .children
            .get(&new_parent)
            .ok_or(MoveError::ChildrenNotLoaded)?
            .iter()
            .filter(|sibling| **sibling != id)
            .filter_map(|sibling| self.pages.get(sibling))
            .collect();
        let insert_at = match drop {
            PageDrop::Inside(_) => siblings.len(),
            PageDrop::Before(target) => siblings
                .iter()
                .position(|page| page.id == target)
                .unwrap_or(0),
            PageDrop::After(target) => siblings
                .iter()
                .position(|page| page.id == target)
                .map_or(siblings.len(), |index| index + 1),
        };
        let before = insert_at
            .checked_sub(1)
            .map(|index| &siblings[index].position);
        let after = siblings.get(insert_at).map(|page| &page.position);
        let position = Position::between(before, after)?;
        let current_parent = self
            .pages
            .get(&id)
            .ok_or(MoveError::UnknownPage(id))?
            .parent;
        let mut operations = vec![Operation::Set(FieldChange::Page {
            id,
            field: PageField::Position(position),
        })];
        if current_parent != new_parent {
            operations.push(Operation::Set(FieldChange::Page {
                id,
                field: PageField::Parent(new_parent),
            }));
        }
        Ok(Transaction::new(operations))
    }
}

/// What to show for a page with an empty title.
pub fn display_title(title: &str) -> &str {
    if title.trim().is_empty() {
        UNTITLED
    } else {
        title
    }
}

/// The title with the page's emoji in front, for breadcrumbs, the palette and the window title.
pub fn labelled_title(page: &Page) -> String {
    match page.icon.as_deref() {
        Some(icon) => format!("{icon} {}", display_title(&page.title)),
        None => display_title(&page.title).to_string(),
    }
}

fn set_field(id: PageId, field: PageField) -> Transaction {
    Transaction::new(vec![Operation::Set(FieldChange::Page { id, field })])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_with(titles: &[(&str, Option<usize>)]) -> (PageTree, Vec<PageId>) {
        let mut tree = PageTree::default();
        let mut ids = Vec::new();
        for (title, parent_index) in titles {
            let parent = parent_index.map(|index| ids[index]);
            let (id, transaction) = tree.create_page(parent, title).unwrap();
            tree.apply(&transaction);
            ids.push(id);
        }
        (tree, ids)
    }

    fn titles(tree: &PageTree, rows: &[TreeRow]) -> Vec<(String, usize)> {
        rows.iter()
            .map(|row| {
                (
                    display_title(&tree.page(row.id).unwrap().title).to_string(),
                    row.depth,
                )
            })
            .collect()
    }

    #[test]
    fn rows_follow_expansion_in_sibling_order() {
        let (tree, ids) = tree_with(&[("A", None), ("B", None), ("A1", Some(0)), ("A2", Some(0))]);
        let collapsed = tree.visible_rows(&HashSet::new());
        assert_eq!(
            titles(&tree, &collapsed),
            [("A".into(), 0), ("B".into(), 0)]
        );
        assert!(collapsed[0].has_children);
        let expanded = tree.visible_rows(&HashSet::from([ids[0]]));
        assert_eq!(
            titles(&tree, &expanded),
            [
                ("A".into(), 0),
                ("A1".into(), 1),
                ("A2".into(), 1),
                ("B".into(), 0)
            ]
        );
    }

    #[test]
    fn empty_titles_read_as_untitled() {
        let (tree, _) = tree_with(&[("  ", None)]);
        let first = tree.visible_rows(&HashSet::new())[0].id;
        assert_eq!(display_title(&tree.page(first).unwrap().title), UNTITLED);
    }

    #[test]
    fn moving_reorders_and_reparents_with_one_position_write() {
        let (mut tree, ids) = tree_with(&[("A", None), ("B", None), ("C", None)]);
        let transaction = tree.move_page(ids[2], PageDrop::Before(ids[0])).unwrap();
        assert_eq!(transaction.operations.len(), 1);
        tree.apply(&transaction);
        assert_eq!(
            titles(&tree, &tree.visible_rows(&HashSet::new())),
            [("C".into(), 0), ("A".into(), 0), ("B".into(), 0)]
        );

        let transaction = tree.move_page(ids[1], PageDrop::Inside(ids[2])).unwrap();
        assert_eq!(transaction.operations.len(), 2);
        tree.apply(&transaction);
        let rows = tree.visible_rows(&HashSet::from([ids[2]]));
        assert_eq!(
            titles(&tree, &rows),
            [("C".into(), 0), ("B".into(), 1), ("A".into(), 0)]
        );

        let transaction = tree.move_page(ids[1], PageDrop::After(ids[0])).unwrap();
        tree.apply(&transaction);
        assert_eq!(
            titles(&tree, &tree.visible_rows(&HashSet::new())),
            [("C".into(), 0), ("A".into(), 0), ("B".into(), 0)]
        );
    }

    #[test]
    fn a_page_cannot_move_into_its_own_subtree() {
        let (tree, ids) = tree_with(&[("A", None), ("A1", Some(0))]);
        assert_eq!(
            tree.move_page(ids[0], PageDrop::Inside(ids[1])),
            Err(MoveError::IntoItself)
        );
        assert_eq!(
            tree.move_page(ids[0], PageDrop::Inside(ids[0])),
            Err(MoveError::IntoItself)
        );
    }

    #[test]
    fn archiving_hides_the_subtree() {
        let (mut tree, ids) = tree_with(&[("A", None), ("A1", Some(0)), ("B", None)]);
        tree.apply(&tree.set_archived(ids[0], true));
        assert_eq!(
            titles(&tree, &tree.visible_rows(&HashSet::from([ids[0]]))),
            [("B".into(), 0)]
        );
        assert!(!tree.is_live(ids[1]));
        tree.apply(&tree.set_archived(ids[0], false));
        assert!(tree.is_live(ids[1]));
    }

    #[test]
    fn ancestors_run_from_the_page_up() {
        let (tree, ids) = tree_with(&[("A", None), ("A1", Some(0)), ("A1a", Some(1))]);
        assert_eq!(
            tree.ancestors_and_self(ids[2]),
            vec![ids[2], ids[1], ids[0]]
        );
    }
}

#[cfg(test)]
mod lazy_tests {
    use super::*;
    use std::time::Instant;

    fn child(title: &str, parent: Option<PageId>, position: &Position) -> (Page, usize) {
        (
            Page {
                id: PageId::generate(),
                parent,
                title: title.into(),
                icon: None,
                position: position.clone(),
                row_of: None,
                is_archived: false,
            },
            0,
        )
    }

    #[test]
    fn a_parent_with_fifty_thousand_children_stays_cheap() {
        let mut tree = PageTree::default();
        let (big, create) = tree.create_page(None, "Big").unwrap();
        tree.apply(&create);
        let mut position = Position::between(None, None).unwrap();
        let mut children = Vec::new();
        for index in 0..50_000 {
            children.push(child(&format!("Child {index}"), Some(big), &position));
            position = Position::between(Some(&position), None).unwrap();
        }
        let started = Instant::now();
        tree.load_children(Some(big), children);
        let rows = tree.visible_rows(&HashSet::from([big]));
        assert_eq!(rows.len(), 50_001);
        let (new, create) = tree.create_page(Some(big), "Last").unwrap();
        tree.apply(&create);
        let moved = tree.move_page(new, PageDrop::Before(rows[1].id)).unwrap();
        tree.apply(&moved);
        assert_eq!(tree.children(Some(big))[0].id, new);
        tree.apply(&Transaction::set_title(new, "Renamed"));
        // Generous for debug builds; the point is no quadratic step.
        assert!(
            started.elapsed().as_millis() < 2_000,
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn unloaded_children_still_show_an_arrow_and_count_new_ones() {
        let mut tree = PageTree::default();
        let (parent, _) = child("Parent", None, &Position::between(None, None).unwrap());
        let parent_id = parent.id;
        tree.load_children(None, vec![(parent, 3)]);
        let rows = tree.visible_rows(&HashSet::new());
        assert!(rows[0].has_children);
        assert!(!tree.are_children_loaded(Some(parent_id)));
        assert_eq!(
            tree.create_page(Some(parent_id), "x"),
            Err(MoveError::ChildrenNotLoaded)
        );
        let arrived = child(
            "Arrived",
            Some(parent_id),
            &Position::between(None, None).unwrap(),
        );
        tree.upsert(arrived.0);
        assert_eq!(tree.child_count(parent_id), 4);
    }
}
