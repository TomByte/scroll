//! What a write changed, so the app can refresh only that: which records, and which parents'
//! lists of child pages (the sidebar's tree).

use rusqlite::Connection;

use crate::id::PageId;
use crate::operation::RecordId;
use crate::record::Page;
use crate::store::StoreError;
use crate::store::kinds::page::read_page;

/// Returned by every write that changes what readers see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreChanges {
    /// Records that may have changed (created, edited or deleted), each once. A property value or
    /// tag membership change counts as a change to its page.
    pub records: Vec<RecordId>,
    /// Parents whose list of tree pages (`page_children`) may have changed, each once; `None` is
    /// the top level. The parent's own `child_count` changed with it.
    pub child_lists: Vec<Option<PageId>>,
    /// Too much changed to list (a whole snapshot was applied): reload everything shown.
    pub is_everything: bool,
}

impl StoreChanges {
    pub fn everything() -> StoreChanges {
        StoreChanges {
            is_everything: true,
            ..StoreChanges::default()
        }
    }

    pub fn merge(&mut self, other: StoreChanges) {
        self.is_everything |= other.is_everything;
        for record in other.records {
            if !self.records.contains(&record) {
                self.records.push(record);
            }
        }
        for parent in other.child_lists {
            if !self.child_lists.contains(&parent) {
                self.child_lists.push(parent);
            }
        }
    }
}

/// Notes the tree placement of the pages among `records` before a write, then compares after it.
pub(in crate::store) struct ChangeWatch {
    records: Vec<RecordId>,
    pages_before: Vec<(PageId, Option<Page>)>,
}

impl ChangeWatch {
    /// `records` holds each record once, as `Transaction::records_touched` gives them.
    pub(in crate::store) fn before(
        conn: &Connection,
        records: Vec<RecordId>,
    ) -> Result<Self, StoreError> {
        let pages_before = page_ids(&records)
            .map(|page| Ok((page, read_page(conn, page)?)))
            .collect::<Result<_, StoreError>>()?;
        Ok(ChangeWatch {
            records,
            pages_before,
        })
    }

    pub(in crate::store) fn after(self, conn: &Connection) -> Result<StoreChanges, StoreError> {
        let mut changes = StoreChanges {
            records: self.records,
            ..StoreChanges::default()
        };
        for (page, before) in self.pages_before {
            let after = read_page(conn, page)?;
            if tree_entry(before.as_ref()) == tree_entry(after.as_ref()) {
                continue;
            }
            for parent in [before, after]
                .iter()
                .flatten()
                .filter(|page| page.is_in_tree())
            {
                if !changes.child_lists.contains(&parent.parent) {
                    changes.child_lists.push(parent.parent);
                }
            }
        }
        Ok(changes)
    }
}

fn page_ids(records: &[RecordId]) -> impl Iterator<Item = PageId> + '_ {
    records.iter().filter_map(|id| match id {
        RecordId::Page(page) => Some(*page),
        _ => None,
    })
}

/// What a parent's child list shows of the page; `None` when it is not in the tree.
fn tree_entry(page: Option<&Page>) -> Option<&Page> {
    page.filter(|page| page.is_in_tree())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{DatabaseId, PropertyId};
    use crate::operation::{DatabaseField, FieldChange, Operation, PageField, Record, Transaction};
    use crate::position::Position;
    use crate::record::{Database, Property, PropertyType, PropertyValue};
    use crate::store::test_support;
    use crate::store::{WorkspaceStore, open_in_memory, open_linked_in_memory};
    use crate::sync::WorkspaceSnapshot;

    fn page(parent: Option<PageId>) -> Page {
        Page {
            id: PageId::generate(),
            parent,
            title: "Page".into(),
            icon: None,
            position: Position::between(None, None).unwrap(),
            row_of: None,
            is_archived: false,
        }
    }

    fn apply(store: &mut WorkspaceStore, operations: Vec<Operation>) -> StoreChanges {
        store
            .apply_local_transaction(&Transaction::new(operations))
            .unwrap()
    }

    fn set(id: PageId, field: PageField) -> Operation {
        Operation::Set(FieldChange::Page { id, field })
    }

    #[test]
    fn writes_report_the_records_and_child_lists_they_changed() {
        let mut store = open_in_memory();
        let (first, second) = (page(None), page(None));
        let child = page(Some(first.id));
        let (first_id, second_id, child_id) = (first.id, second.id, child.id);
        let created = apply(
            &mut store,
            vec![
                Operation::Create(Record::Page(first)),
                Operation::Create(Record::Page(second)),
                Operation::Create(Record::Page(child)),
            ],
        );
        assert_eq!(
            created.records,
            [
                RecordId::Page(first_id),
                RecordId::Page(second_id),
                RecordId::Page(child_id)
            ]
        );
        assert_eq!(created.child_lists, [None, Some(first_id)]);

        let moved = apply(
            &mut store,
            vec![set(child_id, PageField::Parent(Some(second_id)))],
        );
        assert_eq!(moved.records, [RecordId::Page(child_id)]);
        assert_eq!(moved.child_lists, [Some(first_id), Some(second_id)]);

        let renamed = apply(
            &mut store,
            vec![set(child_id, PageField::Title("New".into()))],
        );
        assert_eq!(renamed.child_lists, [Some(second_id)]);

        let archived = apply(&mut store, vec![set(child_id, PageField::IsArchived(true))]);
        assert_eq!(archived.child_lists, [Some(second_id)]);
        // Out of the tree, its edits change no child list.
        let hidden_edit = apply(
            &mut store,
            vec![set(child_id, PageField::Title("x".into()))],
        );
        assert_eq!(hidden_edit.records, [RecordId::Page(child_id)]);
        assert!(hidden_edit.child_lists.is_empty());
    }

    #[test]
    fn editing_a_row_value_changes_no_child_list() {
        let mut store = open_in_memory();
        let host = page(None);
        let property = Property {
            id: PropertyId::generate(),
            name: "Points".into(),
            property_type: PropertyType::Number,
            options: vec![],
        };
        let database = Database {
            id: DatabaseId::generate(),
            host_page: host.id,
            properties: vec![],
        };
        let row = Page {
            row_of: Some(database.id),
            ..page(Some(host.id))
        };
        let row_id = row.id;
        let database_id = database.id;
        apply(
            &mut store,
            vec![
                Operation::Create(Record::Page(host)),
                Operation::Create(Record::Database(database)),
                Operation::Create(Record::Page(row)),
                Operation::Set(FieldChange::Database {
                    id: database_id,
                    field: DatabaseField::Property(property.clone()),
                }),
            ],
        );
        let edit = apply(
            &mut store,
            vec![Operation::Set(FieldChange::PropertyValue {
                page: row_id,
                property: property.id,
                value: Some(PropertyValue::Number(1.0)),
            })],
        );
        assert_eq!(edit.records, [RecordId::Page(row_id)]);
        assert!(edit.child_lists.is_empty());
    }

    #[test]
    fn remote_writes_and_rejections_report_changes_and_a_snapshot_reports_everything() {
        let mut store = open_linked_in_memory();
        let shared = page(None);
        let shared_id = shared.id;
        let remote = store
            .apply_remote_transaction(
                1,
                &Transaction::new(vec![Operation::Create(Record::Page(shared))]),
            )
            .unwrap();
        assert_eq!(remote.child_lists, [None]);

        let rename = Transaction::new(vec![set(shared_id, PageField::Title("Mine".into()))]);
        store.apply_local_transaction(&rename).unwrap();
        let rejected = store.reject_transaction(rename.id).unwrap();
        assert_eq!(rejected.records, [RecordId::Page(shared_id)]);
        assert_eq!(rejected.child_lists, [None]);

        let caught_up = test_support::apply_snapshot(
            &mut store,
            WorkspaceSnapshot {
                seq: 2,
                records: vec![],
                property_values: vec![],
            },
        )
        .unwrap();
        assert!(caught_up.is_everything);
    }
}
