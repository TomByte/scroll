//! The workspace's records: applying what the server sends, uploading this device's transactions, and which
//! databases' rows the server sends at once.

use scroll_api::limits::MAX_MESSAGE_BYTES;
use scroll_api::protocol::{self, ClientMessage, RejectReason, TransactionRejection};
use scroll_core::id::{DatabaseId, TransactionId};
use scroll_core::operation::Transaction;

use super::connection::Connection;
use super::worker::{Interrupt, Worker};
use super::{SyncError, SyncEvent};

impl Worker {
    pub(super) fn apply_remote_transaction(
        &mut self,
        seq: u64,
        transaction: &Transaction,
    ) -> Result<(), Interrupt> {
        let changes = self.store.apply_remote_transaction(seq, transaction)?;
        self.emit_changes(changes);
        Ok(())
    }

    /// Several at once share one store transaction, so one write to disk rather than one each.
    pub(super) async fn acknowledge(
        &mut self,
        connection: &mut Connection,
        acks: Vec<(TransactionId, u64)>,
    ) -> Result<(), Interrupt> {
        if acks.is_empty() {
            return Ok(());
        }
        self.store.acknowledge_transactions(&acks)?;
        for (transaction_id, _) in &acks {
            connection.finish(transaction_id.0);
            self.count_upload_done(connection);
        }
        self.fill_window(connection).await
    }

    /// The store undoes the transaction. A refusal because the workspace is not open cannot happen after
    /// `OpenWorkspace`; reconnecting resends it.
    pub(super) async fn reject_transaction(
        &mut self,
        connection: &mut Connection,
        transaction_id: TransactionId,
        reason: TransactionRejection,
    ) -> Result<(), Interrupt> {
        if reason == TransactionRejection::Refused(RejectReason::WorkspaceNotOpen) {
            return Err(SyncError::UnexpectedMessage("a refusal for an unopened workspace").into());
        }
        let changes = self.store.reject_transaction(transaction_id)?;
        connection.finish(transaction_id.0);
        self.emit(SyncEvent::TransactionRejected {
            transaction_id,
            reason,
            records: changes.records,
            child_lists: changes.child_lists,
        });
        self.count_upload_done(connection);
        self.fill_window(connection).await
    }

    pub(super) async fn push_transaction(
        &mut self,
        connection: &mut Connection,
        transaction: Transaction,
    ) -> Result<(), Interrupt> {
        let transaction_id = transaction.id;
        let frame = protocol::encode(&ClientMessage::PushTransaction {
            workspace_id: self.workspace_id,
            transaction,
        });
        if frame.len() > MAX_MESSAGE_BYTES {
            let changes = self.store.reject_transaction(transaction_id)?;
            self.emit(SyncEvent::TransactionTooLarge {
                transaction_id,
                records: changes.records,
                child_lists: changes.child_lists,
            });
            self.count_upload_done(connection);
            return Ok(());
        }
        connection.track_sent(transaction_id.0, frame.len());
        connection.send_frame(frame).await
    }

    pub(super) async fn open_database_on_server(
        &mut self,
        connection: &mut Connection,
        database_id: DatabaseId,
    ) -> Result<(), Interrupt> {
        let open = ClientMessage::OpenDatabase {
            workspace_id: self.workspace_id,
            database_id,
        };
        connection.send(&open).await
    }

    pub(super) async fn close_database_on_server(
        &mut self,
        connection: &mut Connection,
        database_id: DatabaseId,
    ) -> Result<(), Interrupt> {
        let close = ClientMessage::CloseDatabase {
            workspace_id: self.workspace_id,
            database_id,
        };
        connection.send(&close).await
    }
}
