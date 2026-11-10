//! Catching a client up on a workspace: a snapshot of every record, or the log after what it has seen, read
//! through cursors and sent in parts.

use deadpool_postgres::{Pool, Transaction as DbTransaction};
use scroll_api::limits::CATCH_UP_PART_BYTES;
use scroll_api::protocol::CatchUpPart;
use scroll_core::id::WorkspaceId;
use scroll_core::operation::Record;
use scroll_core::sync::{PropertyValueEntry, SequencedTransaction, WorkspaceSnapshot};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio_postgres::types::ToSql;
use tokio_postgres::{IsolationLevel, Row};

use super::apply::SNAPSHOT_SOURCES;
use super::{RecordError, decode};
use crate::db::{db_seq, wire_seq};

/// Lines a catch-up up with the live feed. `lock` is awaited once the database transaction is ready, so a crowd
/// of connections opening at once does not queue for database connections while holding it; the first query
/// runs under it and fixes the transaction's view; `pinned` then gets that sequence number and the guard.
pub struct Pin<Lock, Pinned> {
    pub lock: Lock,
    pub pinned: Pinned,
}

/// Where a client's workspace catch-up starts.
#[derive(Debug, Clone, Copy)]
pub struct CatchUpFrom {
    pub workspace_id: WorkspaceId,
    pub last_seen_seq: u64,
}

/// Rows fetched from the database at a time while streaming a catch-up.
const FETCH_ROWS: i32 = 5_000;
/// What two ids add to a stored property value, and a page-tag membership's size: near enough for sizing parts.
pub(super) const ID_PAIR_BYTES: usize = 33;

/// Streams what a client that has seen up to `from.last_seen_seq` is missing into `parts`, in parts of about
/// `CATCH_UP_PART_BYTES`: a snapshot when it has seen nothing and there is something, else the logged transactions after
/// it (the log is never truncated). At least one part is sent, even when there is nothing, so the client learns where it stands.
///
/// Everything is read in one repeatable-read transaction, whose view of the database is fixed by its first
/// query, which runs under `pin.lock`. `pin.pinned` is called with the sequence number it reads right after
/// that query, before the rest is read: the caller subscribes to later transactions there and releases the
/// lock, and the parts cover exactly what came before. Rows
/// are read through cursors, so memory stays at a few parts however large the workspace.
pub async fn stream_workspace_catch_up<Guard>(
    pool: &Pool,
    from: CatchUpFrom,
    pin: Pin<impl Future<Output = Guard>, impl FnOnce(u64, Guard)>,
    parts: mpsc::Sender<CatchUpPart>,
) -> Result<(), RecordError> {
    let mut client = pool.get().await?;
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let guard = pin.lock.await;
    // Under the lock: in a repeatable-read transaction, even preparing a query fixes the snapshot.
    let statement = tx
        .prepare_cached("SELECT last_record_seq FROM workspaces WHERE id = $1")
        .await?;
    let seq: i64 = tx
        .query_one(&statement, &[&from.workspace_id.0])
        .await?
        .get("last_record_seq");
    (pin.pinned)(wire_seq(seq), guard);
    let mut assembler = PartAssembler::new(parts);
    // With nothing committed, the log after 0 (empty) says the same as a snapshot, and costs a client that is
    // publishing its records nothing: applying an empty snapshot would weigh every one of them against it.
    if from.last_seen_seq == 0 && seq > 0 {
        assembler.snapshot_seq = Some(wire_seq(seq));
        stream_snapshot(&tx, from.workspace_id, &mut assembler).await?;
    } else {
        stream_log(&tx, from, &mut assembler).await?;
    }
    assembler.finish().await?;
    tx.commit().await?;
    Ok(())
}

async fn stream_snapshot(
    tx: &DbTransaction<'_>,
    workspace_id: WorkspaceId,
    assembler: &mut PartAssembler,
) -> Result<(), RecordError> {
    let workspace = &[&workspace_id.0 as &(dyn ToSql + Sync)];
    for kind in SNAPSHOT_SOURCES {
        for source in *kind {
            stream_rows(tx, (source.sql, workspace), assembler, source.read).await?;
        }
    }
    Ok(())
}

async fn stream_log(
    tx: &DbTransaction<'_>,
    from: CatchUpFrom,
    assembler: &mut PartAssembler,
) -> Result<(), RecordError> {
    let after_seq = db_seq(from.last_seen_seq);
    let query = (
        "SELECT seq, transaction FROM record_transactions WHERE workspace_id = $1 AND seq > $2 ORDER BY seq",
        &[&from.workspace_id.0 as &(dyn ToSql + Sync), &after_seq][..],
    );
    stream_rows(tx, query, assembler, |row| {
        let bytes: &[u8] = row.get("transaction");
        let sequenced = SequencedTransaction {
            seq: wire_seq(row.get("seq")),
            transaction: decode(bytes)?,
        };
        Ok((Item::Transaction(sequenced), bytes.len()))
    })
    .await
}

/// A row holding one encoded record, as most kinds keep theirs.
pub(super) fn stored_record<T: DeserializeOwned>(
    row: &Row,
    record: fn(T) -> Record,
) -> Result<(Item, usize), RecordError> {
    let bytes: &[u8] = row.get("record");
    Ok((Item::Record(record(decode(bytes)?)), bytes.len()))
}

/// Feeds every row of the query, a batch at a time through a cursor, to the assembler.
async fn stream_rows(
    tx: &DbTransaction<'_>,
    (sql, params): (&str, &[&(dyn ToSql + Sync)]),
    assembler: &mut PartAssembler,
    convert: impl Fn(&Row) -> Result<(Item, usize), RecordError>,
) -> Result<(), RecordError> {
    let statement = tx.prepare(sql).await?;
    let portal = tx.bind(&statement, params).await?;
    loop {
        let rows = tx.query_portal(&portal, FETCH_ROWS).await?;
        if rows.is_empty() {
            return Ok(());
        }
        for row in &rows {
            let (item, size) = convert(row)?;
            assembler.push(item, size).await?;
        }
    }
}

/// Turns one row into a catch-up item and its size in bytes.
pub(super) type ReadRow = fn(&Row) -> Result<(Item, usize), RecordError>;

/// Where a snapshot reads one kind from: a query taking the workspace id, and how to read each row.
pub(super) struct SnapshotSource {
    pub sql: &'static str,
    pub read: ReadRow,
}

pub(super) enum Item {
    Record(Record),
    Value(PropertyValueEntry),
    Transaction(SequencedTransaction),
}

/// Gathers items into parts of about `CATCH_UP_PART_BYTES` and hands each on as it fills.
struct PartAssembler {
    parts: mpsc::Sender<CatchUpPart>,
    /// `Some` for a snapshot catch-up, `None` for a log one.
    snapshot_seq: Option<u64>,
    records: Vec<Record>,
    values: Vec<PropertyValueEntry>,
    transactions: Vec<SequencedTransaction>,
    bytes: usize,
    has_sent: bool,
}

impl PartAssembler {
    fn new(parts: mpsc::Sender<CatchUpPart>) -> Self {
        Self {
            parts,
            snapshot_seq: None,
            records: Vec::new(),
            values: Vec::new(),
            transactions: Vec::new(),
            bytes: 0,
            has_sent: false,
        }
    }

    async fn push(&mut self, item: Item, size: usize) -> Result<(), RecordError> {
        if self.bytes > 0 && self.bytes + size > CATCH_UP_PART_BYTES {
            self.send().await?;
        }
        match item {
            Item::Record(record) => self.records.push(record),
            Item::Value(value) => self.values.push(value),
            Item::Transaction(transaction) => self.transactions.push(transaction),
        }
        self.bytes += size;
        Ok(())
    }

    /// Sends what is left; for a catch-up with nothing in it, one empty part.
    async fn finish(mut self) -> Result<(), RecordError> {
        if self.bytes > 0 || !self.has_sent {
            self.send().await?;
        }
        Ok(())
    }

    async fn send(&mut self) -> Result<(), RecordError> {
        let part = match self.snapshot_seq {
            Some(seq) => CatchUpPart::Snapshot(WorkspaceSnapshot {
                seq,
                records: std::mem::take(&mut self.records),
                property_values: std::mem::take(&mut self.values),
            }),
            None => CatchUpPart::Transactions(std::mem::take(&mut self.transactions)),
        };
        self.bytes = 0;
        self.has_sent = true;
        self.parts
            .send(part)
            .await
            .map_err(|_| RecordError::CatchUpAbandoned)
    }
}
