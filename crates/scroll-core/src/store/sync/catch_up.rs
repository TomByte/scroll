//! Applying the transactions a reconnect missed, and exporting every record.

use rusqlite::Connection;

use crate::store::kinds::all_records;
use crate::store::kinds::value::all_property_values;
use crate::store::sync::apply::apply_sequenced_transaction;
use crate::store::sync::changes::StoreChanges;
use crate::store::sync::link::require_linked;
use crate::store::sync::queue;
use crate::store::{StoreError, WorkspaceStore};
use crate::sync::{PropertyValueEntry, SequencedTransaction, WorkspaceSnapshot};

impl WorkspaceStore {
    /// Applies transactions the server committed, in sequence order, such as those a reconnect
    /// missed. One already seen is skipped, so a repeat is harmless. Pending local changes stay
    /// on top. A snapshot arrives through the staged calls (`begin_workspace_snapshot` and on).
    pub fn apply_catch_up_transactions(
        &mut self,
        transactions: &[SequencedTransaction],
    ) -> Result<StoreChanges, StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        let mut changes = StoreChanges::default();
        for sequenced in transactions {
            changes.merge(apply_sequenced_transaction(
                &tx,
                sequenced.seq,
                &sequenced.transaction,
            )?);
        }
        tx.commit()?;
        Ok(changes)
    }

    /// Every record and property value as the local store has it (pending changes included),
    /// stamped with the last workspace sequence number seen.
    pub fn export_workspace_snapshot(&self) -> Result<WorkspaceSnapshot, StoreError> {
        export_snapshot(&self.conn)
    }
}

pub(in crate::store) fn export_snapshot(
    conn: &Connection,
) -> Result<WorkspaceSnapshot, StoreError> {
    let property_values = all_property_values(conn)?
        .into_iter()
        .map(|(page, property, value)| PropertyValueEntry {
            page,
            property,
            value,
        })
        .collect();
    Ok(WorkspaceSnapshot {
        seq: queue::last_seen_workspace_seq(conn)?,
        records: all_records(conn)?,
        property_values,
    })
}
