//! The page tree the sidebar shows, one expanded parent at a time, and the trash.

use rusqlite::{OptionalExtension, params};

use crate::id::{DatabaseId, PageId};
use crate::record::Page;
use crate::store::kinds::page::{PAGE_COLUMNS, in_tree, page_from_row, read_page};

const IN_TREE: &str = in_tree!();
use crate::store::{StoreError, WorkspaceStore, sql_limit};

/// A page in the trash, with what the trash says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedPage {
    pub page: Page,
    /// When it went to the trash, in seconds since 1970: when that was done here, or when the
    /// change arrived from elsewhere. `None` if this file never saw it go (see the migration
    /// that adds the column).
    pub archived_at: Option<i64>,
    /// The title of the page it is under, if it is under one.
    pub parent_title: Option<String>,
}

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

    /// The trash: archived pages that are not database rows, the one archived last first.
    /// Those with no time come after the rest, by title.
    pub fn trashed_pages(&self) -> Result<Vec<TrashedPage>, StoreError> {
        let sql = format!(
            "SELECT {PAGE_COLUMNS}, archived_at,
                    (SELECT parent.title FROM page AS parent WHERE parent.id = page.parent_id)
                        AS parent_title
             FROM page
             WHERE is_archived AND row_of_database_id IS NULL
             ORDER BY archived_at DESC NULLS LAST, title COLLATE NOCASE, id"
        );
        let pages = self
            .conn
            .prepare_cached(&sql)?
            .query_map([], |row| {
                Ok(TrashedPage {
                    page: page_from_row(row)?,
                    archived_at: row.get("archived_at")?,
                    parent_title: row.get("parent_title")?,
                })
            })?
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
    use crate::store::open_in_memory;

    fn page(title: &str, parent: Option<PageId>, position: &Position) -> Page {
        Page {
            id: PageId::generate(),
            parent,
            title: title.into(),
            icon: None,
            position: position.clone(),
            row_of: None,
            is_archived: false,
            is_folder: false,
        }
    }

    #[test]
    fn a_folder_is_kept_as_one_and_lists_the_pages_under_it() {
        let mut store = open_in_memory();
        let position = Position::between(None, None).unwrap();
        let folder = Page {
            is_folder: true,
            ..page("Projects", None, &position)
        };
        let inside = page("Plan", Some(folder.id), &position);
        store
            .apply_local_transaction(&Transaction::new(vec![
                Operation::Create(Record::Page(folder.clone())),
                Operation::Create(Record::Page(inside.clone())),
            ]))
            .unwrap();

        let top = store.page_children(None, None, 10).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(
            (top[0].page.clone(), top[0].child_count),
            (folder.clone(), 1)
        );
        assert!(!store.page(inside.id).unwrap().unwrap().is_folder);

        // A rename writes one field and leaves the rest of the row, the folder flag included.
        store
            .apply_local_transaction(&Transaction::set_title(folder.id, "Work"))
            .unwrap();
        let renamed = store.page(folder.id).unwrap().unwrap();
        assert_eq!((renamed.title.as_str(), renamed.is_folder), ("Work", true));
    }

    fn set_archived(id: PageId, is_archived: bool) -> Transaction {
        Transaction::new(vec![Operation::Set(FieldChange::Page {
            id,
            field: PageField::IsArchived(is_archived),
        })])
    }

    #[test]
    fn the_trash_lists_the_page_archived_last_first_and_restoring_takes_one_out() {
        let mut store = open_in_memory();
        let position = Position::between(None, None).unwrap();
        let parent = page("Projects", None, &position);
        let plan = page("Plan", Some(parent.id), &position);
        let notes = page("Notes", None, &position);
        // In the trash from the start, as a page is that syncs to a new device already there.
        let arrived = Page {
            is_archived: true,
            ..page("Arrived", None, &position)
        };
        let created = [&parent, &plan, &notes, &arrived]
            .map(|page| Operation::Create(Record::Page(page.clone())));
        store
            .apply_local_transaction(&Transaction::new(created.into()))
            .unwrap();
        for page in [&plan, &notes] {
            store
                .apply_local_transaction(&set_archived(page.id, true))
                .unwrap();
        }
        // Both went in within the same second, so move one back an hour to tell them apart.
        store
            .conn
            .execute(
                "UPDATE page SET archived_at = archived_at - 3600 WHERE id = ?1",
                [plan.id],
            )
            .unwrap();

        let trash = store.trashed_pages().unwrap();
        let titles: Vec<&str> = trash
            .iter()
            .map(|trashed| trashed.page.title.as_str())
            .collect();
        assert_eq!(titles, ["Notes", "Plan", "Arrived"]);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let notes_archived_at = trash[0]
            .archived_at
            .expect("archived here, so it has a time");
        assert!(
            (now - notes_archived_at).abs() < 60,
            "{notes_archived_at} is not now"
        );
        assert_eq!(trash[1].archived_at, Some(notes_archived_at - 3600));
        assert_eq!(trash[2].archived_at, None, "this file never saw it go");
        assert_eq!(trash[1].parent_title.as_deref(), Some("Projects"));
        assert_eq!(trash[0].parent_title, None);

        store
            .apply_local_transaction(&set_archived(notes.id, false))
            .unwrap();
        let trash = store.trashed_pages().unwrap();
        let titles: Vec<&str> = trash
            .iter()
            .map(|trashed| trashed.page.title.as_str())
            .collect();
        assert_eq!(titles, ["Plan", "Arrived"]);
    }

    #[test]
    fn a_page_archived_elsewhere_has_the_time_the_change_arrived() {
        let mut store = crate::store::open_linked_in_memory();
        let position = Position::between(None, None).unwrap();
        let shared = page("Shared", None, &position);
        let create = Transaction::new(vec![Operation::Create(Record::Page(shared.clone()))]);
        store.apply_remote_transaction(1, &create).unwrap();
        assert!(store.trashed_pages().unwrap().is_empty());

        store
            .apply_remote_transaction(2, &set_archived(shared.id, true))
            .unwrap();
        let trash = store.trashed_pages().unwrap();
        assert_eq!(trash.len(), 1);
        assert!(trash[0].archived_at.is_some());
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
