//! The page tree the sidebar shows, one expanded parent at a time, and the trash.

use rusqlite::{OptionalExtension, params};

use crate::id::{DatabaseId, PageId};
use crate::record::Page;
use crate::store::kinds::page::{PAGE_COLUMNS, in_tree, page_from_row, read_page};

const IN_TREE: &str = in_tree!();
use crate::store::{StoreError, WorkspaceStore, sql_limit};

/// A page in the tree and how many children it shows, so the sidebar can draw a disclosure
/// arrow without loading them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildPage {
    pub page: Page,
    /// Children that are neither archived nor database rows.
    pub child_count: usize,
    pub hosted_database: Option<DatabaseId>,
}

/// A page's tree children, counted from the tree index, and the database it hosts.
const CHILD_COUNT: &str = concat!(
    "(SELECT count(*) FROM page AS child INDEXED BY page_in_tree
        WHERE child.parent_id = page.id AND ",
    in_tree!(child),
    ") AS child_count,
    (SELECT database.id FROM database WHERE database.host_page_id = page.id) AS hosted_database"
);

fn child_page_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChildPage> {
    Ok(ChildPage {
        page: page_from_row(row)?,
        child_count: row.get("child_count")?,
        hosted_database: row.get("hosted_database")?,
    })
}

impl WorkspaceStore {
    /// Up to `limit` of the pages directly under `parent` (`None` for the top level) that are
    /// neither archived nor database rows, in sibling order, starting after the page `after`
    /// (`None` for the first). Each comes with its own count of such children. Both come from an
    /// index of tree pages alone, so a database's rows cost nothing here. To page through a long
    /// list, pass the last page of each batch as `after`.
    pub fn page_children(
        &self,
        parent: Option<PageId>,
        after: Option<PageId>,
        limit: usize,
    ) -> Result<Vec<ChildPage>, StoreError> {
        let start = match after {
            Some(after) => read_page(&self.conn, after)?.map(|page| (page.position, page.id)),
            None => None,
        };
        let (position, id) = start.unzip();
        let sql = format!(
            "SELECT {PAGE_COLUMNS}, {CHILD_COUNT}
             FROM page INDEXED BY page_in_tree
             WHERE parent_id IS ?1 AND {IN_TREE}
               AND (?2 IS NULL OR (position, id) > (?2, ?3))
             ORDER BY position, id
             LIMIT ?4"
        );
        let children = self
            .conn
            .prepare_cached(&sql)?
            .query_map(
                params![parent, position, id, sql_limit(limit)],
                child_page_from_row,
            )?
            .collect::<Result<_, _>>()?;
        Ok(children)
    }

    /// The given pages (any page: tree page, row or archived), each with its tree child count,
    /// in the order given. Ids of pages that do not exist are skipped.
    pub fn pages_by_id(&self, ids: &[PageId]) -> Result<Vec<ChildPage>, StoreError> {
        let sql = format!("SELECT {PAGE_COLUMNS}, {CHILD_COUNT} FROM page WHERE id = ?1");
        let mut statement = self.conn.prepare_cached(&sql)?;
        let mut pages = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(page) = statement.query_row([id], child_page_from_row).optional()? {
                pages.push(page);
            }
        }
        Ok(pages)
    }

    /// Archived pages that are not database rows (the trash), by title.
    pub fn archived_pages(&self) -> Result<Vec<Page>, StoreError> {
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM page
             WHERE is_archived AND row_of_database_id IS NULL
             ORDER BY title COLLATE NOCASE, id"
        );
        let pages = self
            .conn
            .prepare_cached(&sql)?
            .query_map([], page_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(pages)
    }
}

#[cfg(test)]
mod tests {
    use crate::id::{DatabaseId, PageId};
    use crate::operation::{FieldChange, Operation, PageField, Record, Transaction};
    use crate::position::Position;
    use crate::record::{Database, Page};
    use crate::store::{WorkspaceStore, open_in_memory};

    fn page(title: &str, parent: Option<PageId>, position: &Position) -> Page {
        Page {
            id: PageId::generate(),
            parent,
            title: title.into(),
            icon: None,
            position: position.clone(),
            row_of: None,
            is_archived: false,
        }
    }

    #[test]
    fn the_trash_lists_archived_pages_by_title_and_restoring_takes_one_out() {
        let mut store = open_in_memory();
        let position = Position::between(None, None).unwrap();
        let archived = |title: &str| Page {
            is_archived: true,
            ..page(title, None, &position)
        };
        let (later, earlier) = (archived("b old"), archived("A old"));
        let live = page("Live", None, &position);
        let operations = [
            Record::Page(later.clone()),
            Record::Page(earlier.clone()),
            Record::Page(live),
        ]
        .into_iter()
        .map(Operation::Create)
        .collect();
        store
            .apply_local_transaction(&Transaction::new(operations))
            .unwrap();
        let trash = |store: &WorkspaceStore| -> Vec<String> {
            let pages = store.archived_pages().unwrap();
            pages.into_iter().map(|page| page.title).collect()
        };
        assert_eq!(trash(&store), ["A old", "b old"]);

        let restore = Operation::Set(FieldChange::Page {
            id: earlier.id,
            field: PageField::IsArchived(false),
        });
        store
            .apply_local_transaction(&Transaction::new(vec![restore]))
            .unwrap();
        assert_eq!(trash(&store), ["b old"]);
    }

    #[test]
    fn children_load_one_parent_at_a_time_with_their_own_counts() {
        let mut store = open_in_memory();
        let positions = Position::many_between(None, None, 3).unwrap();
        let parent = page("Parent", None, &positions[0]);
        let second = page("Second", Some(parent.id), &positions[1]);
        let first = page("First", Some(parent.id), &positions[0]);
        let grandchild = page("Grandchild", Some(first.id), &positions[0]);
        let archived = Page {
            is_archived: true,
            ..page("Archived", Some(first.id), &positions[1])
        };
        let database = Database {
            id: DatabaseId::generate(),
            host_page: second.id,
            properties: vec![],
        };
        let database_id = database.id;
        let row = Page {
            row_of: Some(database.id),
            ..page("Row", Some(second.id), &positions[0])
        };
        let operations = [
            Record::Page(parent.clone()),
            Record::Page(second.clone()),
            Record::Page(first.clone()),
            Record::Page(grandchild),
            Record::Page(archived),
            Record::Database(database),
            Record::Page(row),
        ]
        .into_iter()
        .map(Operation::Create)
        .collect();
        store
            .apply_local_transaction(&Transaction::new(operations))
            .unwrap();

        let summary = |parent| -> Vec<(String, usize)> {
            store
                .page_children(parent, None, 100)
                .unwrap()
                .into_iter()
                .map(|child| (child.page.title, child.child_count))
                .collect()
        };
        assert_eq!(summary(None), [("Parent".to_string(), 2)]);
        assert_eq!(
            summary(Some(parent.id)),
            [("First".to_string(), 1), ("Second".to_string(), 0)]
        );
        let hosts: Vec<Option<DatabaseId>> = store
            .page_children(Some(parent.id), None, 100)
            .unwrap()
            .into_iter()
            .map(|child| child.hosted_database)
            .collect();
        assert_eq!(hosts, [None, Some(database_id)]);
        assert_eq!(summary(Some(first.id)), [("Grandchild".to_string(), 0)]);

        let by_id: Vec<(String, usize)> = store
            .pages_by_id(&[second.id, PageId::generate(), first.id])
            .unwrap()
            .into_iter()
            .map(|child| (child.page.title, child.child_count))
            .collect();
        assert_eq!(by_id, [("Second".to_string(), 0), ("First".to_string(), 1)]);
    }

    #[test]
    fn a_long_child_list_pages_through_in_order_including_equal_positions() {
        let mut store = open_in_memory();
        let parent = page("Parent", None, &Position::between(None, None).unwrap());
        let positions = Position::many_between(None, None, 40).unwrap();
        let mut operations = vec![Operation::Create(Record::Page(parent.clone()))];
        // Pairs of children share a position, so paging must also go by id.
        for index in 0..80 {
            let child = page(&index.to_string(), Some(parent.id), &positions[index / 2]);
            operations.push(Operation::Create(Record::Page(child)));
        }
        store
            .apply_local_transaction(&Transaction::new(operations))
            .unwrap();

        let all: Vec<PageId> = store
            .page_children(Some(parent.id), None, 1000)
            .unwrap()
            .into_iter()
            .map(|child| child.page.id)
            .collect();
        let mut paged = Vec::new();
        let mut after = None;
        loop {
            let batch = store.page_children(Some(parent.id), after, 7).unwrap();
            let Some(last) = batch.last() else {
                break;
            };
            after = Some(last.page.id);
            paged.extend(batch.into_iter().map(|child| child.page.id));
        }
        assert_eq!(all.len(), 80);
        assert_eq!(paged, all);
    }
}
