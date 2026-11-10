//! Rules for pages: where they sit in the tree and what they may host.

use super::ValidationError;
use crate::id::{DatabaseId, PageId};
use crate::position::Position;
use crate::record::Database;

pub fn check_position(position: &Position) -> Result<(), ValidationError> {
    if !position.is_well_formed() {
        return Err(ValidationError::MalformedPosition(position.clone()));
    }
    Ok(())
}

/// `parent_lineage` is the proposed parent followed by its ancestors up to a root.
pub fn check_new_parent(page: PageId, parent_lineage: &[PageId]) -> Result<(), ValidationError> {
    let Some(&parent) = parent_lineage.first() else {
        return Ok(());
    };
    if parent_lineage.contains(&page) {
        return Err(ValidationError::PageWouldBeItsOwnAncestor { page, parent });
    }
    Ok(())
}

/// `hosted_there` is the database the page already hosts, if any.
pub fn check_host_page_is_free(
    host_page: PageId,
    hosted_there: Option<DatabaseId>,
) -> Result<(), ValidationError> {
    match hosted_there {
        Some(database) => Err(ValidationError::PageAlreadyHostsDatabase {
            page: host_page,
            database,
        }),
        None => Ok(()),
    }
}

pub fn check_page_has_no_children(page: PageId, child_count: usize) -> Result<(), ValidationError> {
    if child_count > 0 {
        return Err(ValidationError::PageHasChildren(page));
    }
    Ok(())
}

/// `hosted_there` is the database the page hosts, if any.
pub fn check_page_hosts_no_database(
    page: PageId,
    hosted_there: Option<DatabaseId>,
) -> Result<(), ValidationError> {
    match hosted_there {
        Some(database) => Err(ValidationError::PageHostsDatabase { page, database }),
        None => Ok(()),
    }
}

/// `database` is the one the page is becoming a row of.
pub fn check_row_of(page: PageId, database: &Database) -> Result<(), ValidationError> {
    if database.host_page == page {
        return Err(ValidationError::PageCannotBeRowOfItsOwnDatabase {
            page,
            database: database.id,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::test_builders::database_with;
    use crate::validation::{check_database_has_no_rows, check_database_has_no_views};

    #[test]
    fn a_page_cannot_move_under_itself_or_a_descendant() {
        let page = PageId::generate();
        let child = PageId::generate();
        let grandchild = PageId::generate();

        assert_eq!(
            check_new_parent(page, &[page]),
            Err(ValidationError::PageWouldBeItsOwnAncestor { page, parent: page })
        );
        assert_eq!(
            check_new_parent(page, &[grandchild, child, page]),
            Err(ValidationError::PageWouldBeItsOwnAncestor {
                page,
                parent: grandchild
            })
        );
        assert_eq!(check_new_parent(page, &[PageId::generate()]), Ok(()));
        assert_eq!(check_new_parent(page, &[]), Ok(()));
    }

    #[test]
    fn a_page_hosts_at_most_one_database_and_is_not_its_own_row() {
        let database = database_with(vec![]);
        assert_eq!(check_host_page_is_free(database.host_page, None), Ok(()));
        assert_eq!(
            check_host_page_is_free(database.host_page, Some(database.id)),
            Err(ValidationError::PageAlreadyHostsDatabase {
                page: database.host_page,
                database: database.id
            })
        );
        assert_eq!(
            check_row_of(database.host_page, &database),
            Err(ValidationError::PageCannotBeRowOfItsOwnDatabase {
                page: database.host_page,
                database: database.id
            })
        );
        assert_eq!(check_row_of(PageId::generate(), &database), Ok(()));
    }

    #[test]
    fn deleting_requires_no_children_rows_views_or_hosted_database() {
        let page = PageId::generate();
        let database = DatabaseId::generate();
        assert_eq!(check_page_has_no_children(page, 0), Ok(()));
        assert_eq!(
            check_page_has_no_children(page, 2),
            Err(ValidationError::PageHasChildren(page))
        );
        assert_eq!(check_page_hosts_no_database(page, None), Ok(()));
        assert_eq!(
            check_page_hosts_no_database(page, Some(database)),
            Err(ValidationError::PageHostsDatabase { page, database })
        );
        assert_eq!(check_database_has_no_rows(database, 0), Ok(()));
        assert_eq!(
            check_database_has_no_rows(database, 1),
            Err(ValidationError::DatabaseHasRows(database))
        );
        assert_eq!(check_database_has_no_views(database, 0), Ok(()));
        assert_eq!(
            check_database_has_no_views(database, 3),
            Err(ValidationError::DatabaseHasViews(database))
        );
    }
}
