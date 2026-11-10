//! Page updates made here and received from the server. A local-only workspace appends local
//! updates to the page's log under local sequence numbers. A linked workspace queues them; once
//! acknowledged they move into the log under the server's sequence number, next to remote
//! updates, so the local log mirrors the server's.

use rusqlite::Connection;
use uuid::Uuid;

use crate::id::PageId;
use crate::store::content::log::{
    append_update, delete_content, insert_sequenced_update, replace_snapshot, snapshot_seq,
};
use crate::store::content::{PageCatchUp, SequencedBytes};
use crate::store::sync::link::{is_linked, require_linked};
use crate::store::sync::queue::{self, QueuedChange, QueuedPageUpdate};
use crate::store::{StoreError, WorkspaceStore};

impl WorkspaceStore {
    /// Stores an update made on this device: appended to the page's log when the workspace is
    /// local-only, queued for the server when it is linked.
    pub fn record_local_page_update(
        &mut self,
        page_id: PageId,
        update: &[u8],
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        if is_linked(&tx)? {
            queue_update(&tx, page_id, update)?;
        } else {
            append_update(&tx, page_id, update)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Stores what the server sent on opening a page. A snapshot newer than the stored one replaces it.
    pub fn store_page_catch_up(
        &mut self,
        page_id: PageId,
        catch_up: &PageCatchUp,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        if let Some(snapshot) = &catch_up.snapshot
            && snapshot.seq > snapshot_seq(&tx, page_id)?
        {
            replace_snapshot(&tx, page_id, snapshot)?;
        }
        for update in &catch_up.updates {
            insert_sequenced_update(&tx, page_id, update)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Stores an update another client made, under the sequence number the server gave it.
    pub fn store_remote_page_update(
        &mut self,
        page_id: PageId,
        update: &SequencedBytes,
    ) -> Result<(), StoreError> {
        require_linked(&self.conn)?;
        insert_sequenced_update(&self.conn, page_id, update)
    }

    /// The server stored our update at `seq`: it leaves the queue and joins the page's log.
    /// Acknowledging twice is harmless.
    pub fn acknowledge_page_update(&mut self, update_id: Uuid, seq: u64) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        let update = match queue::take(&tx, update_id)? {
            None => return Ok(()),
            Some(QueuedChange::PageUpdate(update)) => update,
            Some(QueuedChange::Transaction(transaction)) => {
                return Err(StoreError::QueueEntryKindMismatch(transaction.id.0));
            }
        };
        insert_sequenced_update(
            &tx,
            update.page_id,
            &SequencedBytes {
                seq,
                bytes: update.update,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The server permanently refused one of our page updates (typically a viewer editing a page
    /// they cannot write to). Loro cannot take an operation back out of a document, and the
    /// refused one may already be folded into the stored snapshot, so the page's stored content is
    /// cleared and must be fetched again (its last seen sequence becomes 0). Later queued updates for
    /// the page are dropped too: they were made on top of the refused one. Returns the page, which
    /// the app must reload from the store after catching up, discarding its open document; `None`
    /// if the update was not queued.
    pub fn reject_page_update(&mut self, update_id: Uuid) -> Result<Option<PageId>, StoreError> {
        let tx = self.conn.transaction()?;
        require_linked(&tx)?;
        let Some(page_id) = queue::take_page_update_and_later(&tx, update_id)? else {
            return Ok(None);
        };
        delete_content(&tx, page_id)?;
        tx.commit()?;
        Ok(Some(page_id))
    }
}

fn queue_update(conn: &Connection, page_id: PageId, update: &[u8]) -> Result<(), StoreError> {
    let change = QueuedChange::PageUpdate(QueuedPageUpdate {
        update_id: Uuid::now_v7(),
        page_id,
        update: update.to_vec(),
    });
    queue::enqueue(conn, &change)
}
