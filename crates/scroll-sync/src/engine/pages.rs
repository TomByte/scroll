//! Page content: catch-up and live updates for open pages, presence, and uploading this device's updates.

use scroll_api::limits::MAX_MESSAGE_BYTES;
use scroll_api::protocol::{self, ClientMessage, RejectReason};
use scroll_core::id::PageId;
use scroll_core::store::{PageCatchUp, QueuedChange, QueuedPageUpdate, SequencedBytes};
use uuid::Uuid;

use super::connection::Connection;
use super::worker::{Interrupt, Worker};
use super::{EditRefusal, SyncEvent};

/// Which of this device's updates the server answered.
#[derive(Clone, Copy)]
pub(super) struct SentUpdate {
    pub page_id: PageId,
    pub update_id: Uuid,
}

impl Worker {
    pub(super) fn store_page_catch_up(
        &mut self,
        page_id: PageId,
        catch_up: PageCatchUp,
    ) -> Result<(), Interrupt> {
        self.store.store_page_catch_up(page_id, &catch_up)?;
        self.emit_for_open_page(page_id, SyncEvent::PageCatchUp { page_id, catch_up });
        Ok(())
    }

    pub(super) fn store_remote_page_update(
        &mut self,
        page_id: PageId,
        update: SequencedBytes,
    ) -> Result<(), Interrupt> {
        self.store.store_remote_page_update(page_id, &update)?;
        self.emit_for_open_page(page_id, SyncEvent::RemotePageUpdate { page_id, update });
        Ok(())
    }

    pub(super) async fn acknowledge_page_update(
        &mut self,
        connection: &mut Connection,
        update: SentUpdate,
        seq: u64,
    ) -> Result<(), Interrupt> {
        let page_id = update.page_id;
        self.store.acknowledge_page_update(update.update_id, seq)?;
        self.emit_for_open_page(page_id, SyncEvent::PageUpdateAcknowledged { page_id, seq });
        self.finish_upload(connection, update).await
    }

    /// A refusal that passes (`EditRefusal::from_reason` is `None`) keeps the update for the next connection.
    pub(super) async fn page_update_rejected(
        &mut self,
        connection: &mut Connection,
        update: SentUpdate,
        reason: RejectReason,
    ) -> Result<(), Interrupt> {
        let Some(refusal) = EditRefusal::from_reason(reason) else {
            connection.hold_back(update.update_id);
            return self.fill_window(connection).await;
        };
        self.refuse_page_edits(connection, update, refusal).await?;
        self.finish_upload(connection, update).await
    }

    pub(super) fn page_rejected(
        &mut self,
        connection: &mut Connection,
        page_id: PageId,
        reason: RejectReason,
    ) {
        connection.open_pages.remove(&page_id);
        self.emit_for_open_page(page_id, SyncEvent::PageRejected { page_id, reason });
    }

    pub(super) async fn push_page_update(
        &mut self,
        connection: &mut Connection,
        update: QueuedPageUpdate,
    ) -> Result<(), Interrupt> {
        let QueuedPageUpdate {
            update_id,
            page_id,
            update,
        } = update;
        let frame = protocol::encode(&ClientMessage::PushPageUpdate {
            update_id,
            page_id,
            update,
        });
        if frame.len() > MAX_MESSAGE_BYTES {
            let update = SentUpdate { page_id, update_id };
            return self
                .refuse_page_edits(connection, update, EditRefusal::TooLarge)
                .await;
        }
        if !connection.open_pages.contains(&page_id) {
            self.open_on_server(connection, page_id).await?;
        }
        *connection.uploads.entry(page_id).or_default() += 1;
        connection.track_sent(update_id, frame.len());
        connection.send_frame(frame).await
    }

    /// Drops the refused update and the page's later ones, whose content the store clears. An open page is
    /// fetched again so the app can reload it.
    async fn refuse_page_edits(
        &mut self,
        connection: &mut Connection,
        update: SentUpdate,
        reason: EditRefusal,
    ) -> Result<(), Interrupt> {
        let SentUpdate { page_id, update_id } = update;
        if self.store.reject_page_update(update_id)?.is_none() {
            // Already dropped along with an earlier refused update.
            return Ok(());
        }
        // The store dropped the page's later updates too.
        connection.unsent.retain(
            |change| !matches!(change, QueuedChange::PageUpdate(later) if later.page_id == page_id),
        );
        self.emit(SyncEvent::PageEditsRefused { page_id, reason });
        if self.open_pages.contains(&page_id) {
            self.open_on_server(connection, page_id).await?;
        }
        Ok(())
    }

    async fn finish_upload(
        &mut self,
        connection: &mut Connection,
        update: SentUpdate,
    ) -> Result<(), Interrupt> {
        let SentUpdate { page_id, update_id } = update;
        connection.finish(update_id);
        if let Some(count) = connection.uploads.get_mut(&page_id) {
            *count = count.saturating_sub(1);
        }
        self.count_upload_done(connection);
        self.close_on_server_if_idle(connection, page_id).await?;
        self.fill_window(connection).await
    }

    /// The server needs a page open to accept updates for it; once the app has closed it and nothing is left
    /// to upload, the subscription goes.
    pub(super) async fn close_on_server_if_idle(
        &mut self,
        connection: &mut Connection,
        page_id: PageId,
    ) -> Result<(), Interrupt> {
        let is_uploading = connection
            .uploads
            .get(&page_id)
            .is_some_and(|count| *count > 0);
        if self.open_pages.contains(&page_id)
            || is_uploading
            || !connection.open_pages.remove(&page_id)
        {
            return Ok(());
        }
        connection.uploads.remove(&page_id);
        connection.send(&ClientMessage::ClosePage { page_id }).await
    }

    pub(super) async fn open_on_server(
        &mut self,
        connection: &mut Connection,
        page_id: PageId,
    ) -> Result<(), Interrupt> {
        let last_seen_seq = self.store.last_seen_page_seq(page_id)?;
        connection.open_pages.insert(page_id);
        let open = ClientMessage::OpenPage {
            workspace_id: self.workspace_id,
            page_id,
            last_seen_seq,
        };
        connection.send(&open).await
    }

    /// The app has closed the page: compacting folds its stored updates into the snapshot, which also
    /// refreshes the search index. It reads only the store, so it cannot lose an update.
    pub(super) fn forget_page(&mut self, page_id: PageId) -> Result<(), Interrupt> {
        self.open_pages.remove(&page_id);
        self.store.compact_page(page_id)?;
        Ok(())
    }

    pub(super) fn emit_for_open_page(&self, page_id: PageId, event: SyncEvent) {
        if self.open_pages.contains(&page_id) {
            self.emit(event);
        }
    }
}
