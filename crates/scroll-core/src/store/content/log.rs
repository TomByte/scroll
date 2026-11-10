//! The stored log of each page: a Loro snapshot (`page_doc`) plus the updates after it
//! (`page_update`), each stamped with the page's sequence number.

use rusqlite::{Connection, OptionalExtension, params};

use crate::id::PageId;
use crate::page_doc::PageDoc;
use crate::store::StoreError;
use crate::store::content::SequencedBytes;

pub(in crate::store) fn append_update(
    conn: &Connection,
    page_id: PageId,
    update: &[u8],
) -> Result<(), StoreError> {
    let seq = last_seen_page_seq(conn, page_id)? + 1;
    conn.execute(
        "INSERT INTO page_update (page_id, seq, bytes) VALUES (?1, ?2, ?3)",
        params![page_id, seq, update],
    )?;
    Ok(())
}

pub(in crate::store) fn pages_with_content(conn: &Connection) -> Result<Vec<PageId>, StoreError> {
    let mut statement =
        conn.prepare_cached("SELECT page_id FROM page_doc UNION SELECT page_id FROM page_update")?;
    let pages = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(pages)
}

/// The page's stored snapshot and updates, without queued local updates.
pub(in crate::store) fn stored_page_doc(
    conn: &Connection,
    page_id: PageId,
) -> Result<PageDoc, StoreError> {
    let doc = PageDoc::new();
    for bytes in stored_content(conn, page_id)? {
        doc.import(&bytes)?;
    }
    Ok(doc)
}

pub(in crate::store) fn delete_content(
    conn: &Connection,
    page_id: PageId,
) -> Result<(), StoreError> {
    conn.execute("DELETE FROM page_doc WHERE page_id = ?1", [page_id])?;
    conn.execute("DELETE FROM page_update WHERE page_id = ?1", [page_id])?;
    Ok(())
}

pub(in crate::store) fn last_seen_page_seq(
    conn: &Connection,
    page_id: PageId,
) -> Result<u64, StoreError> {
    Ok(conn.query_row(
        "SELECT max(
            coalesce((SELECT max(seq) FROM page_update WHERE page_id = ?1), 0),
            coalesce((SELECT seq FROM page_doc WHERE page_id = ?1), 0)
        )",
        [page_id],
        |row| row.get(0),
    )?)
}

pub(in crate::store) fn snapshot_seq(
    conn: &Connection,
    page_id: PageId,
) -> Result<u64, StoreError> {
    Ok(conn
        .query_row(
            "SELECT seq FROM page_doc WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

/// The snapshot (if any) followed by the updates after it, in order.
pub(in crate::store) fn stored_content(
    conn: &Connection,
    page_id: PageId,
) -> Result<Vec<Vec<u8>>, StoreError> {
    let snapshot: Option<Vec<u8>> = conn
        .query_row(
            "SELECT bytes FROM page_doc WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .optional()?;
    let updates = stored_updates_after(conn, page_id, snapshot_seq(conn, page_id)?)?;
    Ok(snapshot
        .into_iter()
        .chain(updates.into_iter().map(|update| update.bytes))
        .collect())
}

pub(in crate::store) fn stored_updates_after(
    conn: &Connection,
    page_id: PageId,
    after_seq: u64,
) -> Result<Vec<SequencedBytes>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT seq, bytes FROM page_update WHERE page_id = ?1 AND seq > ?2 ORDER BY seq",
    )?;
    let updates = statement
        .query_map(params![page_id, after_seq], |row| {
            Ok(SequencedBytes {
                seq: row.get(0)?,
                bytes: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(updates)
}

pub(in crate::store) fn replace_snapshot(
    conn: &Connection,
    page_id: PageId,
    snapshot: &SequencedBytes,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO page_doc (page_id, seq, bytes) VALUES (?1, ?2, ?3)",
        params![page_id, snapshot.seq, snapshot.bytes],
    )?;
    conn.execute(
        "DELETE FROM page_update WHERE page_id = ?1 AND seq <= ?2",
        params![page_id, snapshot.seq],
    )?;
    Ok(())
}

/// Ignores updates already stored or already folded into the snapshot, so redelivery is harmless.
pub(in crate::store) fn insert_sequenced_update(
    conn: &Connection,
    page_id: PageId,
    update: &SequencedBytes,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO page_update (page_id, seq, bytes)
         SELECT ?1, ?2, ?3 WHERE ?2 > coalesce((SELECT seq FROM page_doc WHERE page_id = ?1), 0)",
        params![page_id, update.seq, update.bytes],
    )?;
    Ok(())
}
