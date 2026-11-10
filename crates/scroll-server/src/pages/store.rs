//! Page content storage. Updates are opaque Loro bytes, numbered per page; compaction folds them into a
//! snapshot kept on the page row.

use deadpool_postgres::Pool;
use scroll_core::id::{PageId, UserId, WorkspaceId};
use scroll_core::store::{PageCatchUp, SequencedBytes};
use tokio_postgres::IsolationLevel;
use uuid::Uuid;

use crate::db::{DbError, db_seq, wire_seq};

/// A page id is only meaningful together with the workspace it was opened in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageAddress {
    pub workspace_id: WorkspaceId,
    pub page_id: PageId,
}

pub struct PageUpdate {
    pub update_id: Uuid,
    pub page: PageAddress,
    pub author: UserId,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOutcome {
    Stored {
        seq: u64,
        /// How many updates a catch-up from zero would replay after the snapshot; drives compaction.
        updates_since_snapshot: u64,
    },
    /// A resend of an update stored earlier; nothing was written.
    AlreadyStored {
        seq: u64,
    },
    PageInAnotherWorkspace,
}

/// A snapshot covering every update up to `snapshot.seq`, to replace the one at `replaces_seq`.
pub struct NewSnapshot {
    pub page_id: PageId,
    pub replaces_seq: u64,
    pub snapshot: SequencedBytes,
}

/// The workspace a page belongs to, or `None` if no update has been pushed to it yet.
pub async fn page_workspace(pool: &Pool, page_id: PageId) -> Result<Option<WorkspaceId>, DbError> {
    let client = pool.get().await?;
    let row = client
        .query_opt(
            "SELECT workspace_id FROM pages WHERE id = $1",
            &[&page_id.0],
        )
        .await?;
    Ok(row.map(|row| WorkspaceId(row.get("workspace_id"))))
}

/// Everything after `last_seen_seq`, with the semantics of `WorkspaceStore::catch_up_page`: the snapshot only
/// when it covers updates the reader has not seen, then the updates after whichever is later.
/// Empty if the page belongs to another workspace.
pub async fn catch_up_page(
    pool: &Pool,
    page: PageAddress,
    last_seen_seq: u64,
) -> Result<PageCatchUp, DbError> {
    let last_seen_seq = db_seq(last_seen_seq);
    let mut client = pool.get().await?;
    // One consistent view: a compaction committing between the two reads would otherwise strip the bytes of
    // updates this reader still needs.
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let snapshot = tx
        .query_opt(
            "SELECT snapshot_seq, snapshot FROM pages
             WHERE id = $1 AND workspace_id = $2 AND snapshot_seq > $3 AND snapshot IS NOT NULL",
            &[&page.page_id.0, &page.workspace_id.0, &last_seen_seq],
        )
        .await?
        .map(|row| SequencedBytes {
            seq: wire_seq(row.get("snapshot_seq")),
            bytes: row.get("snapshot"),
        });
    let after_seq = snapshot
        .as_ref()
        .map_or(last_seen_seq, |snapshot| db_seq(snapshot.seq));
    // Updates folded into the snapshot, or of a deleted page, keep their rows without bytes.
    let rows = tx
        .query(
            "SELECT u.seq, u.bytes
             FROM page_updates u JOIN pages p ON p.id = u.page_id
             WHERE u.page_id = $1 AND p.workspace_id = $2 AND u.seq > $3 AND u.bytes IS NOT NULL
             ORDER BY u.seq",
            &[&page.page_id.0, &page.workspace_id.0, &after_seq],
        )
        .await?;
    tx.commit().await?;
    let updates = rows
        .iter()
        .map(|row| SequencedBytes {
            seq: wire_seq(row.get("seq")),
            bytes: row.get("bytes"),
        })
        .collect();
    Ok(PageCatchUp { snapshot, updates })
}

/// Creates the page in the update's workspace on its first update. Locking the page row makes concurrent pushes
/// take the next sequence number one at a time, and makes the resend check race-free.
pub async fn store_page_update(pool: &Pool, update: &PageUpdate) -> Result<StoreOutcome, DbError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    tx.execute(
        "INSERT INTO pages (id, workspace_id) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
        &[&update.page.page_id.0, &update.page.workspace_id.0],
    )
    .await?;
    let page = tx
        .query_one(
            "SELECT workspace_id FROM pages WHERE id = $1 FOR UPDATE",
            &[&update.page.page_id.0],
        )
        .await?;
    if WorkspaceId(page.get("workspace_id")) != update.page.workspace_id {
        return Ok(StoreOutcome::PageInAnotherWorkspace);
    }
    let earlier = tx
        .query_opt(
            "SELECT seq FROM page_updates WHERE page_id = $1 AND update_id = $2",
            &[&update.page.page_id.0, &update.update_id],
        )
        .await?;
    if let Some(earlier) = earlier {
        return Ok(StoreOutcome::AlreadyStored {
            seq: wire_seq(earlier.get("seq")),
        });
    }
    let counters = tx
        .query_one(
            "UPDATE pages SET last_seq = last_seq + 1 WHERE id = $1 RETURNING last_seq, snapshot_seq",
            &[&update.page.page_id.0],
        )
        .await?;
    let seq: i64 = counters.get("last_seq");
    let snapshot_seq: i64 = counters.get("snapshot_seq");
    tx.execute(
        "INSERT INTO page_updates (page_id, seq, update_id, author_id, bytes) VALUES ($1, $2, $3, $4, $5)",
        &[
            &update.page.page_id.0,
            &seq,
            &update.update_id,
            &update.author.0,
            &update.bytes,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(StoreOutcome::Stored {
        seq: wire_seq(seq),
        updates_since_snapshot: wire_seq(seq - snapshot_seq),
    })
}

/// Stores the snapshot and drops the bytes of the updates it covers, unless another compaction replaced the
/// snapshot first. Returns whether it was stored. Holds the page row lock only for these writes.
pub async fn save_snapshot(pool: &Pool, new: &NewSnapshot) -> Result<bool, DbError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    let replaced = tx
        .execute(
            "UPDATE pages SET snapshot_seq = $3, snapshot = $4 WHERE id = $1 AND snapshot_seq = $2",
            &[
                &new.page_id.0,
                &db_seq(new.replaces_seq),
                &db_seq(new.snapshot.seq),
                &new.snapshot.bytes,
            ],
        )
        .await?;
    if replaced == 0 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE page_updates SET bytes = NULL WHERE page_id = $1 AND seq <= $2 AND bytes IS NOT NULL",
        &[&new.page_id.0, &db_seq(new.snapshot.seq)],
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// When a page's record is deleted: its updates keep their rows without bytes, so a late resend is still
/// recognised rather than stored again, and moving `snapshot_seq` up to `last_seq` makes a compaction already
/// under way fail to save.
pub async fn drop_content(
    tx: &deadpool_postgres::Transaction<'_>,
    page_id: PageId,
) -> Result<(), tokio_postgres::Error> {
    tx.execute(
        "UPDATE pages SET snapshot = NULL, snapshot_seq = last_seq WHERE id = $1",
        &[&page_id.0],
    )
    .await?;
    tx.execute(
        "UPDATE page_updates SET bytes = NULL WHERE page_id = $1",
        &[&page_id.0],
    )
    .await?;
    Ok(())
}
