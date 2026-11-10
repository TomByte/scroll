//! Applying a workspace snapshot that arrives in parts, without holding it in memory.
//!
//! `begin_workspace_snapshot(seq)` starts staging (discarding any unfinished staging);
//! `stage_workspace_snapshot_part` stores each part in staging tables that nothing else reads;
//! `finish_workspace_snapshot(seq)` makes the staged records the server-confirmed state, exactly
//! then clears staging.
//!
//! Until the finish starts, the records readers see are untouched, so a crash or a cut connection
//! while staging leaves the previous state. The finish runs as many short transactions
//! (`FINISH_BATCH` records each), so other connections to the file, such as the app's, are never
//! kept waiting long. Its progress is stored with the staging. A crash during the finish leaves
//! some records at their new state and the rest at the old, with the snapshot not yet marked as
//! seen (`last_seen_workspace_seq` unchanged) and the staging intact: calling
//! `finish_workspace_snapshot` again resumes where it stopped, and beginning the snapshot again
//! re-applies all of it. Either way the end state is the snapshot's.

use rusqlite::{Connection, OptionalExtension, params};

use crate::id::PageId;
use crate::operation::{Record, RecordId};
use crate::record::PropertyValue;
use crate::store::kinds::{records_missing_from_staging, staging_key};
use crate::store::sync::changes::StoreChanges;
use crate::store::sync::confirmed::{
    PendingOperations, RecordState, ServerState, delete_confirmed_dependants, read_state, rebuild,
    track_server_state, tracked_records, tracked_server_state, write_state,
};
use crate::store::sync::link::require_linked;
use crate::store::sync::queue;
use crate::store::{StoreError, WorkspaceStore, decode, encode};
use crate::sync::WorkspaceSnapshot;

impl WorkspaceStore {
    /// Starts staging the snapshot at `seq`, discarding any staging left unfinished.
    pub fn begin_workspace_snapshot(&mut self, seq: u64) -> Result<(), StoreError> {
        require_linked(&self.conn)?;
        self.clear_staging_in_steps()?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO staged_snapshot (singleton, seq) VALUES (1, ?1)",
            [seq],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Stores one part's records and values out of sight of readers. The part must belong to the
    /// snapshot begun.
    pub fn stage_workspace_snapshot_part(
        &mut self,
        part: &WorkspaceSnapshot,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        require_staged(&tx, part.seq)?;
        stage(&tx, part)?;
        tx.commit()?;
        Ok(())
    }

    /// The sequence number of a staged snapshot whose finish started and did not complete (for
    /// example the app quit part-way), so `finish_workspace_snapshot_reporting(seq, ..)` can
    /// resume it instead of downloading it again. `None` when nothing is staged, or when staging
    /// may still be incomplete: a snapshot whose finish never started may be missing parts and
    /// must not be finished.
    pub fn unfinished_snapshot(&self) -> Result<Option<u64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT seq FROM staged_snapshot WHERE applied_kind IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Makes the staged snapshot the server-confirmed state of every record, keeping pending local
    /// changes on top, and clears staging. A snapshot older than what this store has already seen
    /// is discarded instead.
    pub fn finish_workspace_snapshot(&mut self, seq: u64) -> Result<StoreChanges, StoreError> {
        self.finish_workspace_snapshot_reporting(seq, |_| {})
    }

    /// As `finish_workspace_snapshot`, calling `progress` after each short transaction with how
    /// many of the staged records are applied so far.
    pub fn finish_workspace_snapshot_reporting(
        &mut self,
        seq: u64,
        mut progress: impl FnMut(SnapshotFinishProgress),
    ) -> Result<StoreChanges, StoreError> {
        require_staged(&self.conn, seq)?;
        if seq < queue::last_seen_workspace_seq(&self.conn)? {
            self.clear_staging_in_steps()?;
            return Ok(StoreChanges::default());
        }
        let total = staged_record_count(&self.conn)?;
        let mut applied = applied_record_count(&self.conn)?;
        progress(SnapshotFinishProgress { applied, total });
        loop {
            let settled = self.settle_next_staged_records()?;
            if settled == 0 {
                break;
            }
            applied += settled;
            progress(SnapshotFinishProgress { applied, total });
        }
        for missing in records_missing_from_staging(&self.conn)?.chunks(FINISH_BATCH) {
            self.settle_absent(missing)?;
        }
        let tracked = tracked_records(&self.conn)?;
        for tracked in tracked.chunks(FINISH_BATCH) {
            self.settle_tracked_absent(tracked)?;
        }
        let tx = self.conn.transaction()?;
        queue::set_last_seen_workspace_seq(&tx, seq)?;
        tx.execute("DELETE FROM staged_snapshot", [])?;
        tx.commit()?;
        self.clear_staging_in_steps()?;
        Ok(StoreChanges::everything())
    }

    /// Ends any staging, then empties the staging tables a slice at a time: deleting a whole
    /// million-row snapshot at once held the write lock for seconds.
    fn clear_staging_in_steps(&mut self) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM staged_snapshot", [])?;
        for sql in [
            "DELETE FROM staged_value WHERE (page_id, property_id) IN
                 (SELECT page_id, property_id FROM staged_value LIMIT ?1)",
            "DELETE FROM staged_record WHERE (kind, key, second_key) IN
                 (SELECT kind, key, second_key FROM staged_record LIMIT ?1)",
        ] {
            while self.conn.execute(sql, [CLEAR_BATCH])? > 0 {
                let_others_write();
            }
        }
        Ok(())
    }

    /// Settles the next staged records after the stored progress, in one short transaction, and
    /// returns how many there were.
    fn settle_next_staged_records(&mut self) -> Result<usize, StoreError> {
        let tx = self.conn.transaction()?;
        let settled = settle_staged_batch(&tx)?;
        tx.commit()?;
        let_others_write();
        Ok(settled)
    }

    fn settle_absent(&mut self, records: &[RecordId]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for id in records {
            settle(&tx, *id, None)?;
        }
        tx.commit()?;
        let_others_write();
        Ok(())
    }

    /// Tracked records the snapshot lacks and that have no local row, such as a page whose delete
    /// is pending: the server no longer has them either.
    fn settle_tracked_absent(&mut self, records: &[RecordId]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for id in records {
            if !is_staged(&tx, *id)? && read_state(&tx, *id)?.is_none() {
                settle(&tx, *id, None)?;
            }
        }
        tx.commit()?;
        let_others_write();
        Ok(())
    }
}

/// How far a snapshot's finish has got, for showing progress during a first join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotFinishProgress {
    /// Staged records applied so far, including any applied before a resumed finish.
    pub applied: usize,
    pub total: usize,
}

fn staged_record_count(conn: &Connection) -> Result<usize, StoreError> {
    Ok(conn.query_row("SELECT count(*) FROM staged_record", [], |row| row.get(0))?)
}

/// Staged records at or before the stored progress, so a resumed finish reports from there.
fn applied_record_count(conn: &Connection) -> Result<usize, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM staged_record, staged_snapshot
         WHERE staged_snapshot.applied_kind IS NOT NULL
           AND (staged_record.kind, staged_record.key, staged_record.second_key)
               <= (staged_snapshot.applied_kind, staged_snapshot.applied_key,
                   staged_snapshot.applied_second_key)",
        [],
        |row| row.get(0),
    )?)
}

/// A pause between two of the finish's transactions, long enough for another connection's
/// waiting write (retried every millisecond) to take the lock.
fn let_others_write() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

/// Staged rows deleted per transaction when clearing staging.
const CLEAR_BATCH: usize = 50_000;

/// Records settled per transaction while finishing: about 150 ms of work at a million rows.
pub(in crate::store) const FINISH_BATCH: usize = 5_000;

fn require_staged(conn: &Connection, seq: u64) -> Result<(), StoreError> {
    require_linked(conn)?;
    let staged: Option<u64> = conn
        .query_row("SELECT seq FROM staged_snapshot", [], |row| row.get(0))
        .optional()?;
    if staged != Some(seq) {
        return Err(StoreError::NoStagedSnapshot { seq, staged });
    }
    Ok(())
}

fn stage(conn: &Connection, part: &WorkspaceSnapshot) -> Result<(), StoreError> {
    let mut records = conn.prepare_cached(
        "INSERT OR REPLACE INTO staged_record (kind, key, second_key, record) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for record in &part.records {
        let (kind, key, second_key) = staging_key(record.id());
        records.execute(params![kind, key, second_key, encode(record)?])?;
    }
    let mut values = conn.prepare_cached(
        "INSERT OR REPLACE INTO staged_value (page_id, property_id, value) VALUES (?1, ?2, ?3)",
    )?;
    for entry in &part.property_values {
        values.execute(params![entry.page, entry.property, encode(&entry.value)?])?;
    }
    Ok(())
}

/// Settles up to `FINISH_BATCH` staged records after the stored progress, in key order, and
/// stores the new progress. Returns how many it settled.
fn settle_staged_batch(conn: &Connection) -> Result<usize, StoreError> {
    let progress: (Option<u8>, Option<uuid::Uuid>, Option<uuid::Uuid>) = conn.query_row(
        "SELECT applied_kind, applied_key, applied_second_key FROM staged_snapshot",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let mut statement = conn.prepare_cached(
        "SELECT kind, key, second_key, record FROM staged_record
         WHERE ?1 IS NULL OR (kind, key, second_key) > (?1, ?2, ?3)
         ORDER BY kind, key, second_key LIMIT ?4",
    )?;
    let mut rows = statement.query(params![progress.0, progress.1, progress.2, FINISH_BATCH])?;
    let mut settled = 0;
    let mut last = None;
    while let Some(row) = rows.next()? {
        let record: Record = decode(row.get_ref(3)?.as_blob().map_err(rusqlite::Error::from)?)?;
        let state = staged_state(conn, &record)?;
        settle(conn, record.id(), Some(state))?;
        last = Some((
            row.get::<_, u8>(0)?,
            row.get::<_, uuid::Uuid>(1)?,
            row.get::<_, uuid::Uuid>(2)?,
        ));
        settled += 1;
    }
    if let Some((kind, key, second_key)) = last {
        conn.prepare_cached(
            "UPDATE staged_snapshot SET applied_kind = ?1, applied_key = ?2, applied_second_key = ?3",
        )?
        .execute(params![kind, key, second_key])?;
    }
    Ok(settled)
}

/// Makes `state` the confirmed state of one record: written straight to the tables, or, for a
/// record with pending local changes, kept as its confirmed copy and rebuilt under them.
fn settle(conn: &Connection, id: RecordId, state: Option<RecordState>) -> Result<(), StoreError> {
    if tracked_server_state(conn, id)?.is_some() {
        track_server_state(conn, id, &ServerState(state))?;
        let queued = queue::queued_transactions_touching(conn, &[id])?;
        return rebuild(conn, id, &PendingOperations::new(&queued));
    }
    write_state(conn, id, state.as_ref())?;
    if state.is_none() {
        delete_confirmed_dependants(conn, id)?;
    }
    Ok(())
}

fn staged_state(conn: &Connection, record: &Record) -> Result<RecordState, StoreError> {
    let mut state = RecordState::from_record(record);
    if let RecordState::Page { page, values } = &mut state {
        *values = staged_values(conn, page.id)?;
    }
    Ok(state)
}

fn staged_values(
    conn: &Connection,
    page: PageId,
) -> Result<Vec<(crate::id::PropertyId, PropertyValue)>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT property_id, value FROM staged_value WHERE page_id = ?1 ORDER BY property_id",
    )?;
    let rows = statement.query_map([page], |row| Ok((row.get(0)?, row.get::<_, Vec<u8>>(1)?)))?;
    rows.map(|row| {
        let (property, bytes) = row?;
        Ok((property, decode(&bytes)?))
    })
    .collect()
}

fn is_staged(conn: &Connection, id: RecordId) -> Result<bool, StoreError> {
    let (kind, key, second_key) = staging_key(id);
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM staged_record WHERE kind = ?1 AND key = ?2 AND second_key = ?3)",
        )?
        .query_row(params![kind, key, second_key], |row| row.get(0))?)
}

#[cfg(test)]
mod tests;
