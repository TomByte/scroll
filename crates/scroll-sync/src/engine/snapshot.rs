//! Catching up: the log after what this device has seen, or a whole snapshot staged part by part.

use std::sync::atomic::Ordering;
use std::time::Duration;

use scroll_api::protocol::CatchUpPart;
use tokio::time::Instant;

use super::connection::Connection;
use super::worker::{Interrupt, Worker};
use super::{SyncError, SyncEvent};

/// How often `ApplyProgress` may be reported while a large snapshot is applied.
const APPLY_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// One part of a workspace catch-up, as it arrived.
pub(super) struct ReceivedPart {
    pub index: u32,
    pub is_last: bool,
    pub part: CatchUpPart,
}

impl Worker {
    pub(super) fn receive_catch_up_part(
        &mut self,
        connection: &mut Connection,
        received: ReceivedPart,
    ) -> Result<(), Interrupt> {
        let ReceivedPart {
            index,
            is_last,
            part,
        } = received;
        if index > 0 || !is_last {
            self.emit(SyncEvent::DownloadProgress {
                received_parts: index + 1,
                is_complete: is_last,
            });
        }
        self.apply_catch_up_part(connection, part)?;
        if is_last {
            self.finish_staged_snapshot(connection)?;
            self.complete_first_catch_up();
        }
        Ok(())
    }

    fn complete_first_catch_up(&self) {
        if self.first_catch_up_pending.swap(false, Ordering::AcqRel) {
            self.emit(SyncEvent::FirstCatchUpComplete);
        }
    }

    /// Log parts apply as they arrive, so a connection cut mid-way resumes after the last one applied. Snapshot
    /// parts are staged in the store as they arrive, out of sight of readers, and replace the records when the
    /// last arrives (`finish_staged_snapshot`); a cut connection starts the snapshot again. Either way memory
    /// holds one part at a time.
    fn apply_catch_up_part(
        &mut self,
        connection: &mut Connection,
        part: CatchUpPart,
    ) -> Result<(), Interrupt> {
        let snapshot_part = match part {
            CatchUpPart::Transactions(transactions) => {
                let changes = self.store.apply_catch_up_transactions(&transactions)?;
                self.emit_changes(changes);
                return Ok(());
            }
            CatchUpPart::Snapshot(snapshot_part) => snapshot_part,
        };
        let seq = snapshot_part.seq;
        match connection.staging_snapshot {
            None => {
                self.store.begin_workspace_snapshot(seq)?;
                connection.staging_snapshot = Some(seq);
            }
            Some(staging) if staging != seq => {
                return Err(
                    SyncError::UnexpectedMessage("snapshot parts at different seqs").into(),
                );
            }
            Some(_) => {}
        }
        self.store.stage_workspace_snapshot_part(&snapshot_part)?;
        Ok(())
    }

    /// After the last part of a catch-up: a snapshot staged on this connection replaces the records.
    fn finish_staged_snapshot(&mut self, connection: &mut Connection) -> Result<(), Interrupt> {
        match connection.staging_snapshot.take() {
            Some(seq) => self.finish_snapshot(seq),
            None => Ok(()),
        }
    }

    /// A first join stopped while applying its snapshot resumes here, rather than downloading it again; the log
    /// then brings it up to date.
    pub(super) fn resume_unfinished_snapshot(&mut self) -> Result<(), Interrupt> {
        match self.store.unfinished_snapshot()? {
            Some(seq) => self.finish_snapshot(seq),
            None => Ok(()),
        }
    }

    /// Makes the staged snapshot the records, reporting `ApplyProgress` on the way. Called again after an
    /// interruption, it carries on where the store stopped.
    fn finish_snapshot(&mut self, seq: u64) -> Result<(), Interrupt> {
        let events = self.events.clone();
        let mut last_report: Option<Instant> = None;
        let changes = self
            .store
            .finish_workspace_snapshot_reporting(seq, |progress| {
                let is_due = last_report.is_none_or(|at| at.elapsed() >= APPLY_PROGRESS_INTERVAL)
                    || progress.applied == progress.total;
                if !is_due {
                    return;
                }
                last_report = Some(Instant::now());
                let event = SyncEvent::ApplyProgress {
                    applied: progress.applied,
                    total: progress.total,
                };
                if events.send(event).is_err() {
                    tracing::debug!("the app has stopped listening for sync events");
                }
            })?;
        self.emit_changes(changes);
        Ok(())
    }
}
