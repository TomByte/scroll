//! Folds a page's updates into a Loro snapshot once enough have built up, so catch-up stays small.
//!
//! The work runs in the background: the updates are read in one consistent view, merged on the blocking pool,
//! and the result stored only if no other compaction replaced the snapshot meanwhile. Pushes carry on
//! throughout; updates committed after the read stay in the log.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use deadpool_postgres::Pool;
use scroll_core::id::PageId;
use scroll_core::page_doc::{PageDoc, PageDocError};
use scroll_core::store::{PageCatchUp, SequencedBytes};
use tokio::task::JoinError;

use crate::db::DbError;
use crate::lock;
use crate::pages::{self, NewSnapshot, PageAddress};

#[derive(Debug, thiserror::Error)]
enum CompactionError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the page's snapshot could not be loaded or exported: {0}")]
    PageDoc(#[from] PageDocError),
    #[error("the merge task did not finish: {0}")]
    Task(#[from] JoinError),
}

pub struct Compactor {
    pool: Pool,
    threshold: u64,
    in_progress: Mutex<HashSet<PageId>>,
}

impl Compactor {
    /// Compacts a page once a catch-up from zero would replay at least `threshold` updates.
    pub fn new(pool: Pool, threshold: u64) -> Self {
        Self {
            pool,
            threshold,
            in_progress: Mutex::default(),
        }
    }

    /// Starts compacting in the background if the page is over the threshold and not already being compacted.
    pub fn consider(self: &Arc<Self>, page: PageAddress, updates_since_snapshot: u64) {
        if updates_since_snapshot < self.threshold {
            return;
        }
        if !lock(&self.in_progress).insert(page.page_id) {
            return;
        }
        let compactor = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = compact(&compactor.pool, page).await {
                tracing::error!(page_id = %page.page_id.0, %error, "page compaction failed");
            }
            lock(&compactor.in_progress).remove(&page.page_id);
        });
    }
}

async fn compact(pool: &Pool, page: PageAddress) -> Result<(), CompactionError> {
    let content = pages::catch_up_page(pool, page, 0).await?;
    let Some(latest_seq) = content.updates.last().map(|update| update.seq) else {
        return Ok(());
    };
    let replaces_seq = content.snapshot.as_ref().map_or(0, |snapshot| snapshot.seq);
    let bytes = tokio::task::spawn_blocking(move || merge(page.page_id, &content)).await??;
    let new = NewSnapshot {
        page_id: page.page_id,
        replaces_seq,
        snapshot: SequencedBytes {
            seq: latest_seq,
            bytes,
        },
    };
    if !pages::save_snapshot(pool, &new).await? {
        tracing::debug!(page_id = %page.page_id.0, "another compaction replaced the snapshot first");
    }
    Ok(())
}

/// The snapshot with every update imported, exported as a new snapshot.
fn merge(page_id: PageId, content: &PageCatchUp) -> Result<Vec<u8>, PageDocError> {
    let doc = match &content.snapshot {
        Some(snapshot) => PageDoc::from_snapshot(&snapshot.bytes)?,
        None => PageDoc::new(),
    };
    for update in &content.updates {
        // The server cannot vet updates when they are pushed. Bytes that are not a Loro update fail for every
        // reader too, so leaving them out loses nothing, while keeping them would stop the page compacting.
        if let Err(error) = doc.import(&update.bytes) {
            tracing::warn!(page_id = %page_id.0, seq = update.seq, %error, "left an unreadable update out of the page snapshot");
        }
    }
    doc.export_snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_update_is_skipped_rather_than_failing_the_merge() {
        let garbage = b"not a loro update".to_vec();
        assert!(PageDoc::new().import(&garbage).is_err());
        let content = PageCatchUp {
            snapshot: None,
            updates: vec![SequencedBytes {
                seq: 1,
                bytes: garbage,
            }],
        };

        let snapshot = merge(PageId::generate(), &content).unwrap();

        assert!(PageDoc::from_snapshot(&snapshot).is_ok());
    }
}
