//! Uploading the store's queue of local changes, oldest first, within the connection's window.

use scroll_core::store::QueuedChange;

use super::SyncEvent;
use super::connection::Connection;
use super::worker::{Interrupt, Worker};

/// Queued changes read from the store at a time: the queue after publishing a large workspace holds thousands of
/// transactions of up to a megabyte each, and only what fits the window is needed at once.
const QUEUE_READ: usize = 32;

impl Worker {
    /// Reports how much is left to upload, then sends as much as the window allows, reading changes the app has
    /// queued since.
    pub(super) async fn send_queue(
        &mut self,
        connection: &mut Connection,
    ) -> Result<(), Interrupt> {
        let remaining = self.store.queued_change_count()?;
        self.report_remaining(connection, remaining);
        self.fill_window(connection).await
    }

    /// Reads the next queued changes after those already read, leaving out any in flight on this connection.
    fn read_more_of_queue(&mut self, connection: &mut Connection) -> Result<(), Interrupt> {
        let entries = self
            .store
            .queued_changes_after(connection.read_position, QUEUE_READ)?;
        for entry in entries {
            connection.read_position = entry.position;
            if !connection.in_flight.contains_key(&entry.change.change_id()) {
                connection.unsent.push_back(entry.change);
            }
        }
        Ok(())
    }

    /// Sends queued changes, oldest first, until about `WINDOW_BYTES` are unanswered, and then returns to
    /// reading: a client that only writes while the server writes back would leave both stuck. Each answer
    /// tops the window up again.
    pub(super) async fn fill_window(
        &mut self,
        connection: &mut Connection,
    ) -> Result<(), Interrupt> {
        while connection.has_window_room() {
            if connection.unsent.is_empty() {
                self.read_more_of_queue(connection)?;
            }
            let Some(change) = connection.unsent.pop_front() else {
                return Ok(());
            };
            match change {
                QueuedChange::Transaction(transaction) => {
                    self.push_transaction(connection, transaction).await?
                }
                QueuedChange::PageUpdate(update) => {
                    self.push_page_update(connection, update).await?
                }
            }
        }
        Ok(())
    }

    pub(super) fn count_upload_done(&self, connection: &mut Connection) {
        if let Some(remaining) = connection.remaining_uploads {
            self.report_remaining(connection, remaining.saturating_sub(1));
        }
    }

    fn report_remaining(&self, connection: &mut Connection, remaining: usize) {
        if connection.remaining_uploads != Some(remaining) {
            connection.remaining_uploads = Some(remaining);
            self.emit(SyncEvent::UploadProgress { remaining });
        }
    }
}
