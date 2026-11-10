//! Applying record transactions, and how local and remote changes combine.
//!
//! Last write wins per field, in the order the server commits transactions (its sequence numbers).
//! The client keeps two views of a record that has pending local changes: the server-confirmed
//! state, and the local state shown to the user, which is the confirmed state with every queued
//! local transaction replayed on top in queue order (see `confirmed`).
//!
//! This relies on one protocol guarantee: a client receives the workspace's committed transactions,
//! including the acknowledgements of its own, in sequence order. So a remote transaction that
//! arrives while ours is queued was committed before ours, and ours will be ordered after it.
//!
//! - Remote transaction: applied to the confirmed state; records with pending changes are then
//!   rebuilt, so a pending local write keeps showing over an older remote one and the remote value
//!   is kept underneath. Already seen sequence numbers (catch-up replays) are ignored.
//! - Acknowledgement: our transaction joins the confirmed state; the local state does not change.
//! - Rejection: our transaction leaves the queue and the records it touched are rebuilt from the
//!   confirmed state and the remaining queue. A rejected create disappears, a rejected delete comes
//!   back, and with nothing else queued the local state equals the server's.
//! - A delete wins over a concurrent edit: the server rejects writes to a record that is gone,
//!   and replaying a field change on a missing record does nothing.

use rusqlite::Connection;

use crate::id::TransactionId;
use crate::operation::{Operation, RecordId, Transaction};
use crate::store::kinds::{delete_record, insert_record, write_field};
use crate::store::sync::changes::{ChangeWatch, StoreChanges};
use crate::store::sync::confirmed::{
    PendingOperations, ServerState, apply_to_state, delete_confirmed_dependants, read_state,
    rebuild, stop_tracking, track_server_state, tracked_server_state,
};
use crate::store::sync::link::{is_linked, require_linked};
use crate::store::sync::queue;
use crate::store::sync::queue::QueuedChange;
use crate::store::validate::Validator;
use crate::store::{StoreError, WorkspaceStore};

impl WorkspaceStore {
    /// Validates and applies a transaction made on this device, all or nothing. In a linked
    /// workspace it is also queued for the server and stays pending until acknowledged or rejected.
    pub fn apply_local_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<StoreChanges, StoreError> {
        let tx = self.conn.transaction()?;
        let watch = ChangeWatch::before(&tx, transaction.records_touched())?;
        if is_linked(&tx)? {
            apply_and_queue(&tx, transaction)?;
        } else {
            apply_final(&tx, transaction)?;
        }
        let changes = watch.after(&tx)?;
        tx.commit()?;
        Ok(changes)
    }

    /// Applies a transaction the server committed at `seq`. Not validated: the server already did.
    /// Receiving one of our own queued transactions (for example during catch-up after a lost
    /// acknowledgement) counts as its acknowledgement.
    pub fn apply_remote_transaction(
        &mut self,
        seq: u64,
        transaction: &Transaction,
    ) -> Result<StoreChanges, StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        let changes = apply_sequenced_transaction(&tx, seq, transaction)?;
        tx.commit()?;
        Ok(changes)
    }

    /// Several acknowledgements in one write, as when a burst of them arrives after a large
    /// publish: each commit waits for the disk, so one commit for many is much cheaper.
    pub fn acknowledge_transactions(
        &mut self,
        acknowledgements: &[(TransactionId, u64)],
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        for (id, seq) in acknowledgements {
            acknowledge(&tx, *id)?;
            queue::advance_last_seen_workspace_seq(&tx, *seq)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The server refused our transaction: undo it locally, keeping later queued transactions.
    /// Rejecting twice is harmless.
    pub fn reject_transaction(&mut self, id: TransactionId) -> Result<StoreChanges, StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        let Some(transaction) = take_queued_transaction(&tx, id)? else {
            return Ok(StoreChanges::default());
        };
        let records = transaction.records_touched();
        let watch = ChangeWatch::before(&tx, records.clone())?;
        let queued = queue::queued_transactions_touching(&tx, &records)?;
        let pending = PendingOperations::new(&queued);
        for record in records {
            rebuild(&tx, record, &pending)?;
        }
        let changes = watch.after(&tx)?;
        tx.commit()?;
        Ok(changes)
    }
}

fn apply_final(conn: &Connection, transaction: &Transaction) -> Result<(), StoreError> {
    let mut validator = Validator::new(conn);
    for operation in &transaction.operations {
        validator.validate(operation)?;
        apply_confirmed_operation(conn, operation)?;
        validator.forget_schema_changed_by(operation);
    }
    Ok(())
}

fn apply_and_queue(conn: &Connection, transaction: &Transaction) -> Result<(), StoreError> {
    let mut validator = Validator::new(conn);
    for operation in &transaction.operations {
        validator.validate(operation)?;
        remember_server_state(conn, operation.record_id())?;
        apply_pending_operation(conn, operation)?;
        validator.forget_schema_changed_by(operation);
    }
    queue::enqueue(conn, &QueuedChange::Transaction(transaction.clone()))
}

/// Our own transaction coming back changes nothing readers see: it was already applied.
pub(in crate::store) fn apply_sequenced_transaction(
    conn: &Connection,
    seq: u64,
    transaction: &Transaction,
) -> Result<StoreChanges, StoreError> {
    if seq <= queue::last_seen_workspace_seq(conn)? {
        return Ok(StoreChanges::default());
    }
    let mut changes = StoreChanges::default();
    if queue::is_queued(conn, transaction.id.0)? {
        acknowledge(conn, transaction.id)?;
    } else {
        let watch = ChangeWatch::before(conn, transaction.records_touched())?;
        apply_remote_operations(conn, transaction)?;
        changes = watch.after(conn)?;
    }
    queue::advance_last_seen_workspace_seq(conn, seq)?;
    Ok(changes)
}

fn apply_remote_operations(conn: &Connection, transaction: &Transaction) -> Result<(), StoreError> {
    let mut tracked_records = Vec::new();
    for operation in &transaction.operations {
        let id = operation.record_id();
        match tracked_server_state(conn, id)? {
            Some(ServerState(state)) => {
                track_server_state(conn, id, &ServerState(apply_to_state(state, operation)))?;
                if !tracked_records.contains(&id) {
                    tracked_records.push(id);
                }
            }
            None => apply_confirmed_operation(conn, operation)?,
        }
    }
    if tracked_records.is_empty() {
        return Ok(());
    }
    let queued = queue::queued_transactions_touching(conn, &tracked_records)?;
    let pending = PendingOperations::new(&queued);
    for id in tracked_records {
        rebuild(conn, id, &pending)?;
    }
    Ok(())
}

/// Folds our transaction into the confirmed state of each record it touched. The local state
/// already includes it, so the tables do not change. Costs in proportion to the transaction.
fn acknowledge(conn: &Connection, id: TransactionId) -> Result<(), StoreError> {
    let still_queued = queue::records_queued_after_taking(conn, id.0)?;
    let Some(transaction) = take_queued_transaction(conn, id)? else {
        return Ok(());
    };
    let operations = PendingOperations::new(std::slice::from_ref(&transaction));
    for record in transaction.records_touched() {
        let Some(ServerState(state)) = tracked_server_state(conn, record)? else {
            continue;
        };
        let applied = operations.on(record).iter().copied();
        let confirmed = ServerState(applied.fold(state, apply_to_state));
        if still_queued.contains(&record) {
            track_server_state(conn, record, &confirmed)?;
        } else {
            stop_tracking(conn, record, &confirmed)?;
        }
    }
    Ok(())
}

fn take_queued_transaction(
    conn: &Connection,
    id: TransactionId,
) -> Result<Option<Transaction>, StoreError> {
    match queue::take(conn, id.0)? {
        None => Ok(None),
        Some(QueuedChange::Transaction(transaction)) => Ok(Some(transaction)),
        Some(QueuedChange::PageUpdate(update)) => {
            Err(StoreError::QueueEntryKindMismatch(update.update_id))
        }
    }
}

/// Keeps the server's version of a record before the first pending change to it.
fn remember_server_state(conn: &Connection, id: RecordId) -> Result<(), StoreError> {
    if tracked_server_state(conn, id)?.is_some() {
        return Ok(());
    }
    track_server_state(conn, id, &ServerState(read_state(conn, id)?))
}

/// A change the server has (or that will never go to one): a deleted page or tag loses what
/// depends on it too.
fn apply_confirmed_operation(conn: &Connection, operation: &Operation) -> Result<(), StoreError> {
    apply_pending_operation(conn, operation)?;
    match operation {
        Operation::Delete(id) => delete_confirmed_dependants(conn, *id),
        _ => Ok(()),
    }
}

/// A local change that may still be rejected, so a deleted page keeps its content for now.
fn apply_pending_operation(conn: &Connection, operation: &Operation) -> Result<(), StoreError> {
    match operation {
        Operation::Create(record) => insert_record(conn, record),
        Operation::Set(change) => write_field(conn, change),
        Operation::Delete(id) => delete_record(conn, *id),
    }
}

#[cfg(test)]
mod tests;
