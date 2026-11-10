//! Page content: loading, saving and compacting a page's Loro document, and what a reader needs
//! to catch up. The stored log is in `log`; updates made here or received from the server in
//! `sync`.

pub(in crate::store) mod log;
mod sync;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::id::PageId;
use crate::page_doc::PageDoc;
use crate::store::content::log::{
    last_seen_page_seq, replace_snapshot, snapshot_seq, stored_page_doc, stored_updates_after,
};
use crate::store::sync::queue;
use crate::store::{StoreError, WorkspaceStore, search};

/// A Loro snapshot or update, with its position in the page's update sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequencedBytes {
    pub seq: u64,
    pub bytes: Vec<u8>,
}

/// What a reader needs to bring a page up to date: import the snapshot (if any), then the updates in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCatchUp {
    pub snapshot: Option<SequencedBytes>,
    pub updates: Vec<SequencedBytes>,
}

impl WorkspaceStore {
    /// Everything a reader that has seen up to `last_seen_seq` is missing.
    /// The snapshot is included only when it covers updates the reader has not seen.
    pub fn catch_up_page(
        &self,
        page_id: PageId,
        last_seen_seq: u64,
    ) -> Result<PageCatchUp, StoreError> {
        let snapshot = self
            .conn
            .query_row(
                "SELECT seq, bytes FROM page_doc WHERE page_id = ?1 AND seq > ?2",
                params![page_id, last_seen_seq],
                |row| {
                    Ok(SequencedBytes {
                        seq: row.get(0)?,
                        bytes: row.get(1)?,
                    })
                },
            )
            .optional()?;
        let after_seq = snapshot
            .as_ref()
            .map_or(last_seen_seq, |snapshot| snapshot.seq);
        let updates = stored_updates_after(&self.conn, page_id, after_seq)?;
        Ok(PageCatchUp { snapshot, updates })
    }

    /// The highest page sequence number stored; send it when opening the page to catch up.
    pub fn last_seen_page_seq(&self, page_id: PageId) -> Result<u64, StoreError> {
        last_seen_page_seq(&self.conn, page_id)
    }

    /// The page as stored: snapshot, later updates, and local updates still waiting for the server.
    pub fn load_page_doc(&self, page_id: PageId) -> Result<PageDoc, StoreError> {
        page_doc_with_queued_updates(&self.conn, page_id)
    }

    /// Replaces the stored snapshot with `doc`, which must contain every update up to `included_seq`.
    /// Updates up to that sequence are folded away; later ones are kept.
    pub fn save_page_doc(
        &mut self,
        page_id: PageId,
        doc: &PageDoc,
        included_seq: u64,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let stored_seq = snapshot_seq(&tx, page_id)?;
        if included_seq < stored_seq {
            return Err(StoreError::SnapshotOlderThanStored {
                offered: included_seq,
                stored: stored_seq,
            });
        }
        let snapshot = SequencedBytes {
            seq: included_seq,
            bytes: doc.export_snapshot()?,
        };
        replace_snapshot(&tx, page_id, &snapshot)?;
        search::update_body(&tx, page_id, &doc.plain_text()?)?;
        tx.commit()?;
        Ok(())
    }

    /// Folds the stored updates into a new snapshot at the latest sequence number. Readers catching
    /// up from before that number get the snapshot; readers past it are unaffected.
    pub fn compact_page(&mut self, page_id: PageId) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let latest_seq = last_seen_page_seq(&tx, page_id)?;
        if latest_seq == snapshot_seq(&tx, page_id)? {
            return Ok(());
        }
        let doc = stored_page_doc(&tx, page_id)?;
        replace_snapshot(
            &tx,
            page_id,
            &SequencedBytes {
                seq: latest_seq,
                bytes: doc.export_snapshot()?,
            },
        )?;
        search::update_body(&tx, page_id, &doc.plain_text()?)?;
        tx.commit()?;
        Ok(())
    }

    /// Re-indexes the page's text for search, local updates still waiting for the server
    /// included. For a writer that must not compact: in a linked workspace the sync engine owns
    /// the stored log, and compacting leaves queued updates out of the index.
    pub fn refresh_page_search(&self, page_id: PageId) -> Result<(), StoreError> {
        refresh_search_body(&self.conn, page_id)
    }
}

fn page_doc_with_queued_updates(conn: &Connection, page_id: PageId) -> Result<PageDoc, StoreError> {
    let doc = stored_page_doc(conn, page_id)?;
    for bytes in queue::queued_page_updates(conn, page_id)? {
        doc.import(&bytes)?;
    }
    Ok(doc)
}

/// Re-indexes the page's text, for a page row that came back (a rejected delete).
pub(in crate::store) fn refresh_search_body(
    conn: &Connection,
    page_id: PageId,
) -> Result<(), StoreError> {
    let text = page_doc_with_queued_updates(conn, page_id)?.plain_text()?;
    search::update_body(conn, page_id, &text)
}

#[cfg(test)]
mod tests;
