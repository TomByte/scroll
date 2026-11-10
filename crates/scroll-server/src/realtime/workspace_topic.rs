//! Records: opening a workspace's record stream, reading its feed, and pushing transactions.

use std::sync::Arc;

use scroll_api::limits::MAX_OPEN_DATABASES;
use scroll_api::protocol::{CatchUpPart, RejectReason, ServerMessage, TransactionRejection};
use scroll_core::id::{DatabaseId, TransactionId, WorkspaceId};
use scroll_core::operation::Transaction;
use tokio::sync::mpsc;

use super::commit_leader::{self, Push};
use super::connection::{Connection, ConnectionError, Disconnect, Subscription, edit_refusal};
use super::feed::{FellBehind, Reader};
use super::hub::Topic;
use crate::lock;
use crate::records::{self, CatchUpFrom, CommitOutcome, Pin};
use crate::workspaces;

/// Parts read ahead of the socket: enough to keep it busy, few enough to keep memory small.
const CATCH_UP_PARTS_IN_FLIGHT: usize = 2;
/// How much of the feed one read takes before the connection gets back to its other work.
const FEED_READ_BYTES: usize = 4 * 1024 * 1024;

impl Connection {
    pub(super) async fn open_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        last_seen_seq: u64,
    ) -> Result<(), Disconnect> {
        let pool = &self.state.pool;
        if workspaces::workspace_role(pool, self.user_id, workspace_id)
            .await?
            .is_none()
        {
            return self
                .reject_workspace(workspace_id, RejectReason::NotAMember)
                .await;
        }
        let topic = Topic::Workspace(workspace_id);
        // Reopening starts over from what the client says it has.
        self.unsubscribe(topic);
        let subscription = Subscription {
            workspace_id,
            id: self.next_subscription(),
        };
        let (part_sender, parts) = mpsc::channel(CATCH_UP_PARTS_IN_FLIGHT);
        let pool = self.state.pool.clone();
        let hub = Arc::clone(&self.state.hub);
        let connection_id = self.id;
        let wake = Arc::clone(&self.feed_ready);
        let read = async move {
            let room = hub.workspace_room(workspace_id);
            let pin = Pin {
                lock: room.lock_sequencing(),
                pinned: |seq, sequencing| {
                    room.feed().subscribe(
                        connection_id,
                        Reader {
                            read_seq: seq,
                            wake,
                        },
                    );
                    drop(sequencing);
                },
            };
            let from = CatchUpFrom {
                workspace_id,
                last_seen_seq,
            };
            records::stream_workspace_catch_up(&pool, from, pin, part_sender).await
        };
        let (read, last_part) = tokio::join!(read, self.send_catch_up_parts(workspace_id, parts));
        let caught_up = match (read, last_part) {
            (Ok(()), Ok(last_part)) => Ok(last_part),
            (Err(error), _) => Err(Disconnect::from(error)),
            (_, Err(disconnect)) => Err(disconnect),
        };
        let last_part = match caught_up {
            Ok(last_part) => last_part,
            Err(disconnect) => {
                self.state
                    .hub
                    .workspace_room(workspace_id)
                    .feed()
                    .unsubscribe(self.id);
                return Err(disconnect);
            }
        };
        self.subscriptions.insert(topic, subscription);
        let feed = self.state.hub.workspace_room(workspace_id).shared_feed();
        self.feeds.insert(workspace_id, feed);
        if let Some((index, part)) = last_part {
            let message = ServerMessage::WorkspaceCatchUpPart {
                workspace_id,
                index,
                is_last: true,
                part,
            };
            self.send_message(&message).await?;
        }
        self.deliver_feed(workspace_id).await
    }

    /// Writes each part straight to the socket as it is read, so the client receives the workspace's history
    /// in order, before anything from the feed. The last part is held back and returned: it is marked as the
    /// last only once the whole read has succeeded, so a failure part-way never looks complete to the client.
    async fn send_catch_up_parts(
        &mut self,
        workspace_id: WorkspaceId,
        mut parts: mpsc::Receiver<CatchUpPart>,
    ) -> Result<Option<(u32, CatchUpPart)>, Disconnect> {
        let mut held_back: Option<CatchUpPart> = None;
        let mut index = 0;
        while let Some(part) = parts.recv().await {
            let Some(previous) = held_back.replace(part) else {
                continue;
            };
            let message = ServerMessage::WorkspaceCatchUpPart {
                workspace_id,
                index,
                is_last: false,
                part: previous,
            };
            self.send_message(&message).await?;
            index += 1;
        }
        Ok(held_back.map(|part| (index, part)))
    }

    /// Sends what the feeds of the open workspaces have that this connection has not read.
    pub(super) async fn deliver_feeds(&mut self) -> Result<(), Disconnect> {
        let workspaces: Vec<WorkspaceId> = self.feeds.keys().copied().collect();
        for workspace_id in workspaces {
            self.deliver_feed(workspace_id).await?;
        }
        Ok(())
    }

    /// One read's worth; if there may be more, the connection is woken again once it has seen to its other work.
    async fn deliver_feed(&mut self, workspace_id: WorkspaceId) -> Result<(), Disconnect> {
        let Some(feed) = self.feeds.get(&workspace_id) else {
            return Ok(());
        };
        let read = lock(feed).read(self.id, FEED_READ_BYTES);
        let frames = read.map_err(|FellBehind| Disconnect::Lagged)?;
        if frames.is_empty() {
            return Ok(());
        }
        self.feed_ready.notify_one();
        self.write_frames(frames).await
    }

    pub(super) async fn open_database(
        &mut self,
        workspace_id: WorkspaceId,
        database_id: DatabaseId,
    ) -> Result<(), Disconnect> {
        let Some(feed) = self.feeds.get(&workspace_id) else {
            return self
                .reject_workspace(workspace_id, RejectReason::WorkspaceNotOpen)
                .await;
        };
        lock(feed).open_database(self.id, database_id);
        let is_new = self.open_databases.insert((workspace_id, database_id));
        if is_new && self.open_databases.len() > MAX_OPEN_DATABASES {
            return Err(Disconnect::Rejected(RejectReason::ProtocolViolation));
        }
        Ok(())
    }

    pub(super) fn close_database(&mut self, workspace_id: WorkspaceId, database_id: DatabaseId) {
        if !self.open_databases.remove(&(workspace_id, database_id)) {
            return;
        }
        if let Some(feed) = self.feeds.get(&workspace_id) {
            lock(feed).close_database(self.id, database_id);
        }
    }

    pub(super) async fn push_transaction(
        &mut self,
        workspace_id: WorkspaceId,
        transaction: Transaction,
    ) -> Result<(), Disconnect> {
        let transaction_id = transaction.id;
        let refused = TransactionRejection::Refused;
        let topic = Topic::Workspace(workspace_id);
        if !self.subscriptions.contains_key(&topic) {
            return self
                .reject_transaction(
                    workspace_id,
                    transaction_id,
                    refused(RejectReason::WorkspaceNotOpen),
                )
                .await;
        };
        if let Some(reason) = edit_refusal(&self.state, self.user_id, workspace_id).await? {
            return self
                .reject_transaction(workspace_id, transaction_id, refused(reason))
                .await;
        }
        match self.commit(workspace_id, transaction).await? {
            // Its ack is in the feed, after everything committed before it.
            CommitOutcome::Committed { .. } => self.deliver_feed(workspace_id).await,
            CommitOutcome::AlreadyCommitted { seq } => {
                self.deliver_feed(workspace_id).await?;
                let ack = ServerMessage::TransactionAck {
                    workspace_id,
                    transaction_id,
                    seq,
                };
                self.send_message(&ack).await
            }
            CommitOutcome::Invalid(error) => {
                let reason = TransactionRejection::Invalid(error);
                self.reject_transaction(workspace_id, transaction_id, reason)
                    .await
            }
        }
    }

    async fn commit(
        &mut self,
        workspace_id: WorkspaceId,
        transaction: Transaction,
    ) -> Result<CommitOutcome, Disconnect> {
        let push = Push {
            workspace_id,
            author: self.user_id,
            connection: self.id,
            transaction,
        };
        commit_leader::commit(&self.state, push)
            .await
            .map_err(|failed| ConnectionError::from(failed).into())
    }

    async fn reject_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        reason: RejectReason,
    ) -> Result<(), Disconnect> {
        self.send_message(&ServerMessage::WorkspaceRejected {
            workspace_id,
            reason,
        })
        .await
    }

    async fn reject_transaction(
        &mut self,
        workspace_id: WorkspaceId,
        transaction_id: TransactionId,
        reason: TransactionRejection,
    ) -> Result<(), Disconnect> {
        self.send_message(&ServerMessage::TransactionRejected {
            workspace_id,
            transaction_id,
            reason,
        })
        .await
    }
}
