//! Committing pushed transactions in groups. Each connection pushing to a workspace queues its transaction in
//! the workspace's feed; whichever of them gets the workspace's sequencing lock commits a group of what is
//! waiting and appends what committed to the feed, in sequence order, before letting go of the lock.

use std::time::{Duration, Instant};

use scroll_api::protocol::{ConnectionId, ServerMessage};
use scroll_core::id::{DatabaseId, UserId, WorkspaceId};
use scroll_core::operation::Transaction;
use tokio::sync::oneshot;

use super::connection::encode;
use super::feed::{CommitFailed, Entry, WaitingCommit};
use super::hub::WorkspaceRoom;
use crate::records::{self, CommitOutcome, PushedTransaction};
use crate::state::AppState;

/// Every writer to a workspace waits while one commit holds its lock, so a long hold is worth knowing about.
const SLOW_COMMIT: Duration = Duration::from_secs(1);

pub struct Push {
    pub workspace_id: WorkspaceId,
    pub author: UserId,
    pub connection: ConnectionId,
    pub transaction: Transaction,
}

/// Queues the transaction, then either finds it committed by another connection, or commits a group of what is
/// waiting itself, until it is answered.
pub async fn commit(state: &AppState, push: Push) -> Result<CommitOutcome, CommitFailed> {
    let workspace_id = push.workspace_id;
    let room = state.hub.workspace_room(workspace_id);
    let (reply, mut answer) = oneshot::channel();
    room.feed().wait_to_commit(WaitingCommit {
        transaction: push.transaction,
        author: push.author,
        connection: push.connection,
        reply,
    });
    loop {
        tokio::select! {
            biased;
            // A dropped reply means the connection running the commit went away mid-way.
            answer = &mut answer => return answer.unwrap_or(Err(CommitFailed)),
            _sequencing = room.lock_sequencing() => {
                commit_next_group(state, &room, workspace_id).await;
            }
        }
    }
}

/// Runs under the room's sequencing lock, which `commit` holds.
async fn commit_next_group(state: &AppState, room: &WorkspaceRoom, workspace_id: WorkspaceId) {
    let group = room.feed().take_next_group();
    if group.is_empty() {
        return;
    }
    let started = Instant::now();
    let pushed: Vec<PushedTransaction<'_>> = group
        .iter()
        .map(|commit| PushedTransaction {
            author: commit.author,
            transaction: &commit.transaction,
        })
        .collect();
    let committed = records::commit_group(&state.pool, workspace_id, &pushed).await;
    drop(pushed);
    warn_if_slow(workspace_id, &group, started.elapsed());
    let outcomes = match committed {
        Ok(outcomes) => outcomes,
        Err(error) => {
            tracing::error!(%error, workspace_id = %workspace_id.0, transactions = group.len(), "a group commit failed");
            for commit in group {
                // A closed reply belongs to a connection that has gone; there is no one left to tell.
                let _ = commit.reply.send(Err(CommitFailed));
            }
            return;
        }
    };
    let mut entries = Vec::new();
    let mut answers = Vec::with_capacity(group.len());
    for (commit, outcome) in group.into_iter().zip(outcomes) {
        if let CommitOutcome::Committed { seq, rows_of } = outcome {
            entries.push(feed_entry(workspace_id, (seq, rows_of), &commit));
        }
        answers.push((commit.reply, outcome));
    }
    let mut feed = room.feed();
    for entry in entries {
        feed.append(entry);
    }
    drop(feed);
    for (reply, outcome) in answers {
        // As above: nobody is waiting for a reply that cannot be sent.
        let _ = reply.send(Ok(outcome));
    }
}

/// Encoded outside the feed's lock, which every connection reading the feed takes.
fn feed_entry(
    workspace_id: WorkspaceId,
    (seq, rows_of): (u64, Option<DatabaseId>),
    commit: &WaitingCommit,
) -> Entry {
    let remote = ServerMessage::RemoteTransaction {
        workspace_id,
        seq,
        author: commit.author,
        transaction: commit.transaction.clone(),
    };
    let ack = ServerMessage::TransactionAck {
        workspace_id,
        transaction_id: commit.transaction.id,
        seq,
    };
    Entry {
        seq,
        author: commit.connection,
        rows_of,
        remote: encode(&remote),
        ack: encode(&ack),
    }
}

fn warn_if_slow(workspace_id: WorkspaceId, group: &[WaitingCommit], held: Duration) {
    if held <= SLOW_COMMIT {
        return;
    }
    let operations: usize = group
        .iter()
        .map(|commit| commit.transaction.operations.len())
        .sum();
    tracing::warn!(
        workspace_id = %workspace_id.0,
        transactions = group.len(),
        operations,
        held_ms = held.as_millis(),
        "a commit held the workspace lock for long"
    );
}
