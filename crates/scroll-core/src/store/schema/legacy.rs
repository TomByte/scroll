//! Encodings written before views had column widths. Postcard cannot read a struct with a field
//! missing, so a migration decodes queued transactions and confirmed records with these old
//! shapes once and writes them back in the current one. Only `View` changed; everything that
//! holds a view is mirrored here, the rest is the current type. Those current types (`Page`,
//! `Database`, `FieldChange` and the rest) must be copied here as frozen shapes before any of them
//! next changes, or this migration will stop reading what it was written for.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::id::{DatabaseId, PropertyId, TransactionId, ViewId};
use crate::operation::{FieldChange, Operation, Record, RecordId, Transaction};
use crate::record::{Database, FilterCondition, Page, PropertyValue, SortKey, View, ViewType};
use crate::store::sync::confirmed::{RecordState, ServerState};
use crate::store::sync::queue::{QueuedChange, QueuedPageUpdate};
use crate::store::{StoreError, decode, encode};

#[derive(Serialize, Deserialize)]
struct LegacyView {
    id: ViewId,
    database: DatabaseId,
    name: String,
    view_type: ViewType,
    filter: Vec<FilterCondition>,
    sort: Vec<SortKey>,
    visible_columns: Vec<PropertyId>,
}

#[derive(Serialize, Deserialize)]
enum LegacyRecord {
    Page(Page),
    Database(Database),
    View(LegacyView),
}

#[derive(Serialize, Deserialize)]
enum LegacyOperation {
    Create(LegacyRecord),
    Set(FieldChange),
    Delete(RecordId),
}

#[derive(Serialize, Deserialize)]
struct LegacyTransaction {
    id: TransactionId,
    operations: Vec<LegacyOperation>,
}

#[derive(Serialize, Deserialize)]
enum LegacyQueuedChange {
    Transaction(LegacyTransaction),
    PageUpdate(QueuedPageUpdate),
}

#[derive(Serialize, Deserialize)]
enum LegacyRecordState {
    Page {
        page: Page,
        values: Vec<(PropertyId, PropertyValue)>,
    },
    Database(Database),
    View(LegacyView),
}

#[derive(Serialize, Deserialize)]
struct LegacyServerState(Option<LegacyRecordState>);

impl From<LegacyView> for View {
    fn from(view: LegacyView) -> View {
        View {
            id: view.id,
            database: view.database,
            name: view.name,
            view_type: view.view_type,
            filter: view.filter,
            sort: view.sort,
            visible_columns: view.visible_columns,
            column_widths: Vec::new(),
        }
    }
}

impl From<LegacyRecord> for Record {
    fn from(record: LegacyRecord) -> Record {
        match record {
            LegacyRecord::Page(page) => Record::Page(page),
            LegacyRecord::Database(database) => Record::Database(database),
            LegacyRecord::View(view) => Record::View(view.into()),
        }
    }
}

impl From<LegacyQueuedChange> for QueuedChange {
    fn from(change: LegacyQueuedChange) -> QueuedChange {
        match change {
            LegacyQueuedChange::PageUpdate(update) => QueuedChange::PageUpdate(update),
            LegacyQueuedChange::Transaction(transaction) => {
                QueuedChange::Transaction(Transaction {
                    id: transaction.id,
                    operations: transaction
                        .operations
                        .into_iter()
                        .map(|operation| match operation {
                            LegacyOperation::Create(record) => Operation::Create(record.into()),
                            LegacyOperation::Set(change) => Operation::Set(change),
                            LegacyOperation::Delete(id) => Operation::Delete(id),
                        })
                        .collect(),
                })
            }
        }
    }
}

impl From<LegacyServerState> for ServerState {
    fn from(state: LegacyServerState) -> ServerState {
        ServerState(state.0.map(|state| match state {
            LegacyRecordState::Page { page, values } => RecordState::Page { page, values },
            LegacyRecordState::Database(database) => RecordState::Database(database),
            LegacyRecordState::View(view) => RecordState::View(view.into()),
        }))
    }
}

pub(in crate::store) fn add_column_widths_to_encoded_views(
    conn: &Connection,
) -> Result<(), StoreError> {
    let queued: Vec<(i64, Vec<u8>)> = conn
        .prepare("SELECT queue_order, change FROM tx_queue")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (order, bytes) in queued {
        let change: QueuedChange = decode::<LegacyQueuedChange>(&bytes)?.into();
        conn.execute(
            "UPDATE tx_queue SET change = ?2 WHERE queue_order = ?1",
            params![order, encode(&change)?],
        )?;
    }
    let confirmed: Vec<(Vec<u8>, Vec<u8>)> = conn
        .prepare("SELECT record, state FROM confirmed_record")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (record, bytes) in confirmed {
        let state: ServerState = decode::<LegacyServerState>(&bytes)?.into();
        conn.execute(
            "UPDATE confirmed_record SET state = ?2 WHERE record = ?1",
            params![record, encode(&state)?],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::open_in_memory;
    use crate::store::test_support;

    fn legacy_view() -> LegacyView {
        LegacyView {
            id: ViewId::generate(),
            database: DatabaseId::generate(),
            name: "Old".into(),
            view_type: ViewType::Table,
            filter: Vec::new(),
            sort: Vec::new(),
            visible_columns: vec![PropertyId::generate()],
        }
    }

    #[test]
    fn old_encodings_of_views_are_rewritten_with_empty_widths() {
        let store = open_in_memory();
        let view = legacy_view();
        let view_id = view.id;
        let transaction = LegacyQueuedChange::Transaction(LegacyTransaction {
            id: TransactionId::generate(),
            operations: vec![LegacyOperation::Create(LegacyRecord::View(view))],
        });
        store
            .conn
            .execute(
                "INSERT INTO tx_queue (change_id, change) VALUES (?1, ?2)",
                params![uuid::Uuid::now_v7(), encode(&transaction).unwrap()],
            )
            .unwrap();
        let state = LegacyServerState(Some(LegacyRecordState::View(legacy_view())));
        store
            .conn
            .execute(
                "INSERT INTO confirmed_record (record, state) VALUES (?1, ?2)",
                params![
                    encode(&RecordId::View(view_id)).unwrap(),
                    encode(&state).unwrap()
                ],
            )
            .unwrap();

        add_column_widths_to_encoded_views(&store.conn).unwrap();

        let queued = test_support::queued_changes(&store).unwrap();
        let [QueuedChange::Transaction(transaction)] = &queued[..] else {
            panic!("expected one transaction, got {queued:?}");
        };
        let Operation::Create(Record::View(view)) = &transaction.operations[0] else {
            panic!("expected a view create");
        };
        assert_eq!((view.id, view.column_widths.len()), (view_id, 0));
        let bytes: Vec<u8> = store
            .conn
            .query_row("SELECT state FROM confirmed_record", [], |row| row.get(0))
            .unwrap();
        let state: ServerState = decode(&bytes).unwrap();
        assert!(matches!(state.0, Some(RecordState::View(view)) if view.column_widths.is_empty()));
    }

    #[test]
    fn view_rows_written_before_the_column_existed_read_as_having_no_widths() {
        let store = open_in_memory();
        let view = legacy_view();
        store
            .conn
            .execute(
                "INSERT INTO view (id, database_id, name, view_type, filter, sort, visible_columns)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    view.id,
                    view.database,
                    view.name,
                    encode(&view.view_type).unwrap(),
                    encode(&view.filter).unwrap(),
                    encode(&view.sort).unwrap(),
                    encode(&view.visible_columns).unwrap()
                ],
            )
            .unwrap();
        let read = store.view(view.id).unwrap().unwrap();
        assert!(read.column_widths.is_empty());
    }
}
