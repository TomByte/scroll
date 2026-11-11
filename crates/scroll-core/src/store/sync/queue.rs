//! Client sync bookkeeping: changes waiting for the server's acknowledgement, in the order they were made,
//! and the last workspace sequence number received.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::id::PageId;
use crate::operation::{RecordId, Transaction};
use crate::store::{StoreError, WorkspaceStore, decode, encode, sql_limit};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QueuedChange {
    Transaction(Transaction),
    PageUpdate(QueuedPageUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedPageUpdate {
    /// Matches `PushPageUpdate::update_id` and the server's acknowledgement.
    pub update_id: Uuid,
    pub page_id: PageId,
    pub update: Vec<u8>,
}

impl QueuedChange {
    /// The id the server acknowledges the change by.
    pub fn change_id(&self) -> Uuid {
        match self {
            QueuedChange::Transaction(transaction) => transaction.id.0,
            QueuedChange::PageUpdate(update) => update.update_id,
        }
    }

    fn page_id(&self) -> Option<PageId> {
        match self {
            QueuedChange::Transaction(_) => None,
            QueuedChange::PageUpdate(update) => Some(update.page_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedEntry {
    /// Increases along the queue; pass the last one seen to `queued_changes_after` for the next.
    pub position: u64,
    pub change: QueuedChange,
}

impl WorkspaceStore {
    /// At most `limit` changes not yet acknowledged that come after `position` in the queue
    /// (0 for the start), oldest first: the order to send them in.
    pub fn queued_changes_after(
        &self,
        position: u64,
        limit: usize,
    ) -> Result<Vec<QueuedEntry>, StoreError> {
        let mut statement = self.conn.prepare_cached(
            "SELECT queue_order, change FROM tx_queue WHERE queue_order > ?1
             ORDER BY queue_order LIMIT ?2",
        )?;
        let rows = statement.query_map(params![position, sql_limit(limit)], |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (position, bytes) = row?;
            Ok(QueuedEntry {
                position,
                change: decode(&bytes)?,
            })
        })
        .collect()
    }

    /// How many changes wait for the server, for showing progress.
    pub fn queued_change_count(&self) -> Result<usize, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM tx_queue", [], |row| row.get(0))?)
    }

    /// The highest workspace sequence number received; send it when reconnecting to catch up.
    pub fn last_seen_workspace_seq(&self) -> Result<u64, StoreError> {
        last_seen_workspace_seq(&self.conn)
    }
}

pub(in crate::store) fn enqueue(
    conn: &Connection,
    change: &QueuedChange,
) -> Result<(), StoreError> {
    conn.prepare_cached("INSERT INTO tx_queue (change_id, page_id, change) VALUES (?1, ?2, ?3)")?
        .execute(params![
            change.change_id(),
            change.page_id(),
            encode(change)?
        ])?;
    if let QueuedChange::Transaction(transaction) = change {
        index_records(conn, conn.last_insert_rowid(), transaction)?;
    }
    Ok(())
}

/// Notes which records a queued transaction touches, so the queue can be searched by record.
pub(in crate::store) fn index_records(
    conn: &Connection,
    queue_order: i64,
    transaction: &Transaction,
) -> Result<(), StoreError> {
    let mut statement = conn.prepare_cached(
        "INSERT OR IGNORE INTO queued_record (record, queue_order) VALUES (?1, ?2)",
    )?;
    for record in transaction.records_touched() {
        statement.execute(params![encode(&record)?, queue_order])?;
    }
    Ok(())
}

/// Removes the change and returns it, or `None` if it is not queued (for example, already acknowledged).
pub(in crate::store) fn take(
    conn: &Connection,
    change_id: Uuid,
) -> Result<Option<QueuedChange>, StoreError> {
    let taken: Option<(i64, Vec<u8>)> = conn
        .prepare_cached("DELETE FROM tx_queue WHERE change_id = ?1 RETURNING queue_order, change")?
        .query_row([change_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((queue_order, bytes)) = taken else {
        return Ok(None);
    };
    conn.prepare_cached("DELETE FROM queued_record WHERE queue_order = ?1")?
        .execute([queue_order])?;
    Ok(Some(decode(&bytes)?))
}

/// The records of the queued change `change_id` that other queued transactions also touch, so
/// they stay pending once it leaves the queue. One query, whatever the size of the queue.
pub(in crate::store) fn records_queued_after_taking(
    conn: &Connection,
    change_id: Uuid,
) -> Result<HashSet<RecordId>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT mine.record FROM tx_queue
         JOIN queued_record AS mine ON mine.queue_order = tx_queue.queue_order
         WHERE tx_queue.change_id = ?1 AND EXISTS (
             SELECT 1 FROM queued_record AS other
             WHERE other.record = mine.record AND other.queue_order != mine.queue_order)",
    )?;
    let rows = statement.query_map([change_id], |row| row.get::<_, Vec<u8>>(0))?;
    rows.map(|bytes| decode(&bytes?)).collect()
}

/// The queued transactions that touch any of `records`, oldest first. Together they hold every
/// queued operation on those records.
pub(in crate::store) fn queued_transactions_touching(
    conn: &Connection,
    records: &[RecordId],
) -> Result<Vec<Transaction>, StoreError> {
    let mut orders = std::collections::BTreeSet::new();
    let mut statement =
        conn.prepare_cached("SELECT queue_order FROM queued_record WHERE record = ?1")?;
    for record in records {
        let rows = statement.query_map([encode(record)?], |row| row.get::<_, i64>(0))?;
        for order in rows {
            orders.insert(order?);
        }
    }
    let mut read = conn.prepare_cached("SELECT change FROM tx_queue WHERE queue_order = ?1")?;
    let mut transactions = Vec::with_capacity(orders.len());
    for order in orders {
        let bytes: Vec<u8> = read.query_row([order], |row| row.get(0))?;
        match decode(&bytes)? {
            QueuedChange::Transaction(transaction) => transactions.push(transaction),
            QueuedChange::PageUpdate(update) => {
                return Err(StoreError::QueueEntryKindMismatch(update.update_id));
            }
        }
    }
    Ok(transactions)
}

/// Fills `queued_record` for a queue written before it existed.
pub(in crate::store) fn index_existing_queue(conn: &Connection) -> Result<(), StoreError> {
    let queued: Vec<(i64, Vec<u8>)> = conn
        .prepare("SELECT queue_order, change FROM tx_queue WHERE page_id IS NULL")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (order, bytes) in queued {
        if let QueuedChange::Transaction(transaction) = decode(&bytes)? {
            index_records(conn, order, &transaction)?;
        }
    }
    Ok(())
}

pub(in crate::store) fn set_last_seen_workspace_seq(
    conn: &Connection,
    seq: u64,
) -> Result<(), StoreError> {
    conn.execute("UPDATE sync_state SET last_seen_workspace_seq = ?1", [seq])?;
    Ok(())
}

/// Removes the page update and every later queued update for the same page, and returns the page.
/// `None` if the update is not queued.
pub(in crate::store) fn take_page_update_and_later(
    conn: &Connection,
    update_id: Uuid,
) -> Result<Option<PageId>, StoreError> {
    let entry: Option<(i64, Option<PageId>)> = conn
        .query_row(
            "SELECT queue_order, page_id FROM tx_queue WHERE change_id = ?1",
            [update_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((queue_order, page_id)) = entry else {
        return Ok(None);
    };
    let page_id = page_id.ok_or(StoreError::QueueEntryKindMismatch(update_id))?;
    conn.execute(
        "DELETE FROM tx_queue WHERE page_id = ?1 AND queue_order >= ?2",
        params![page_id, queue_order],
    )?;
    Ok(Some(page_id))
}

pub(in crate::store) fn is_queued(conn: &Connection, change_id: Uuid) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM tx_queue WHERE change_id = ?1)",
        [change_id],
        |row| row.get(0),
    )?)
}

pub(in crate::store) fn queued_page_updates(
    conn: &Connection,
    page_id: PageId,
) -> Result<Vec<Vec<u8>>, StoreError> {
    let mut statement =
        conn.prepare_cached("SELECT change FROM tx_queue WHERE page_id = ?1 ORDER BY queue_order")?;
    let rows = statement.query_map([page_id], |row| row.get::<_, Vec<u8>>(0))?;
    rows.map(|bytes| match decode(&bytes?)? {
        QueuedChange::PageUpdate(update) => Ok(update.update),
        QueuedChange::Transaction(transaction) => {
            Err(StoreError::QueueEntryKindMismatch(transaction.id.0))
        }
    })
    .collect()
}

pub(in crate::store) fn last_seen_workspace_seq(conn: &Connection) -> Result<u64, StoreError> {
    Ok(conn.query_row(
        "SELECT last_seen_workspace_seq FROM sync_state",
        [],
        |row| row.get(0),
    )?)
}

pub(in crate::store) fn advance_last_seen_workspace_seq(
    conn: &Connection,
    seq: u64,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sync_state SET last_seen_workspace_seq = max(last_seen_workspace_seq, ?1)",
        [seq],
    )?;
    Ok(())
}
