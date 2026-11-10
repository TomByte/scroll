//! The `/ws` endpoint: one task per client connection, speaking `scroll_api::protocol`.
//!
//! Everything a client receives about a page after subscribing goes through the connection's outbox, queued under
//! the page's sequencing lock, so it arrives in sequence order. A workspace's committed transactions are read
//! from its feed (see `feed`), which holds them in sequence order.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade, close_code};
use axum::response::Response;
use futures_util::SinkExt;
use scroll_api::connection::{PING_INTERVAL, RECONNECT_PROMPTLY, SILENCE_LIMIT};
use scroll_api::protocol::{
    self, ClientMessage, ConnectionId, PROTOCOL_VERSION, RejectReason, ServerMessage,
};
use scroll_core::id::{DatabaseId, UserId, WorkspaceId};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, MissedTickBehavior, interval_at, timeout};
use uuid::Uuid;

use scroll_api::limits::{BATCH_BYTES, MAX_MESSAGE_BYTES};

use super::feed::{CommitFailed, Feed};
use super::hub::{self, Outgoing, Subscriber, Topic};
use crate::auth::sessions;
use crate::db::DbError;
use crate::pages::PageAddress;
use crate::records::RecordError;
use crate::state::AppState;
use crate::workspaces;

/// Frames waiting for a client; past this the client is too slow and is disconnected to catch up later.
const OUTBOX_CAPACITY: usize = 256;
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// The most queued messages written before one flush.
const FORWARD_BATCH: usize = 256;

pub async fn upgrade(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve_connection(state, socket))
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ConnectionError {
    #[error("websocket error: {0}")]
    Socket(#[from] axum::Error),
    #[error("timed out sending to the client")]
    SendTimedOut,
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Records(#[from] RecordError),
    #[error(transparent)]
    CommitFailed(#[from] CommitFailed),
}

/// Why the server stops serving a connection.
pub(super) enum Disconnect {
    ClientClosed,
    Rejected(RejectReason),
    Lagged,
    Idle,
    ShuttingDown,
    Failed(ConnectionError),
}

impl<E: Into<ConnectionError>> From<E> for Disconnect {
    fn from(error: E) -> Self {
        Self::Failed(error.into())
    }
}

#[derive(Clone, Copy)]
pub(super) struct Subscription {
    pub(super) workspace_id: WorkspaceId,
    /// Distinguishes this subscription from an earlier one to the same topic on this connection.
    pub(super) id: u64,
}

pub(super) struct Connection {
    pub(super) state: AppState,
    pub(super) socket: WebSocket,
    pub(super) id: ConnectionId,
    pub(super) user_id: UserId,
    pub(super) session_token: String,
    pub(super) outbox: mpsc::Sender<Outgoing>,
    pub(super) lagged: Arc<Notify>,
    /// Signalled when a workspace feed has something this connection should read now.
    pub(super) feed_ready: Arc<Notify>,
    /// Signalled when the session is signed out.
    pub(super) session_ended: Arc<Notify>,
    pub(super) subscriptions: HashMap<Topic, Subscription>,
    pub(super) feeds: HashMap<WorkspaceId, Arc<std::sync::Mutex<Feed>>>,
    pub(super) open_databases: HashSet<(WorkspaceId, DatabaseId)>,
    pub(super) subscriptions_made: u64,
    pub(super) last_heard: Instant,
}

/// A session that passed `Hello`.
struct Session {
    user_id: UserId,
    token: String,
}

async fn serve_connection(state: AppState, mut socket: WebSocket) {
    let mut shutdown = state.shutdown.clone();
    let hello = tokio::select! {
        hello = timeout(HELLO_TIMEOUT, accept_hello(&state, &mut socket)) => hello,
        _ = shutdown.changed() => Ok(Err(Disconnect::ShuttingDown)),
    };
    let session = match hello {
        Ok(Ok(session)) => session,
        Ok(Err(disconnect)) => return say_goodbye(&mut socket, disconnect).await,
        Err(_) => return say_goodbye(&mut socket, Disconnect::Idle).await,
    };
    let (outbox, mut inbox) = mpsc::channel(OUTBOX_CAPACITY);
    let mut connection = Connection {
        id: ConnectionId(Uuid::now_v7()),
        state,
        socket,
        user_id: session.user_id,
        session_token: session.token,
        outbox,
        lagged: Arc::new(Notify::new()),
        feed_ready: Arc::new(Notify::new()),
        session_ended: Arc::new(Notify::new()),
        subscriptions: HashMap::new(),
        feeds: HashMap::new(),
        open_databases: HashSet::new(),
        subscriptions_made: 0,
        last_heard: Instant::now(),
    };
    let live_sessions = Arc::clone(&connection.state.live_sessions);
    live_sessions.register(
        &connection.session_token,
        connection.id,
        Arc::clone(&connection.session_ended),
    );
    let welcome = ServerMessage::Welcome {
        user_id: connection.user_id,
        connection_id: connection.id,
    };
    let disconnect = match connection.send_message(&welcome).await {
        Ok(()) => connection.run(&mut inbox).await,
        Err(disconnect) => disconnect,
    };
    live_sessions.unregister(&connection.session_token, connection.id);
    connection.close_everything();
    say_goodbye(&mut connection.socket, disconnect).await;
}

async fn accept_hello(state: &AppState, socket: &mut WebSocket) -> Result<Session, Disconnect> {
    let hello = loop {
        if let Some(message) = decode_frame(socket.recv().await)? {
            break message;
        }
    };
    let ClientMessage::Hello {
        protocol_version,
        session_token,
    } = hello
    else {
        return Err(Disconnect::Rejected(RejectReason::ProtocolViolation));
    };
    if protocol_version != PROTOCOL_VERSION {
        return Err(Disconnect::Rejected(
            RejectReason::UnsupportedProtocolVersion,
        ));
    }
    let user_id = sessions::session_user(&state.pool, &session_token)
        .await?
        .ok_or(Disconnect::Rejected(RejectReason::InvalidSession))?;
    Ok(Session {
        user_id,
        token: session_token,
    })
}

/// `Ok(None)` for control frames, which carry no message.
fn decode_frame(
    frame: Option<Result<Message, axum::Error>>,
) -> Result<Option<ClientMessage>, Disconnect> {
    match frame {
        None | Some(Ok(Message::Close(_))) => Err(Disconnect::ClientClosed),
        Some(Err(error)) => Err(error.into()),
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(None),
        Some(Ok(Message::Text(_))) => Err(Disconnect::Rejected(RejectReason::ProtocolViolation)),
        Some(Ok(Message::Binary(bytes))) => protocol::decode(&bytes)
            .map(Some)
            .map_err(|_| Disconnect::Rejected(RejectReason::ProtocolViolation)),
    }
}

async fn say_goodbye(socket: &mut WebSocket, disconnect: Disconnect) {
    let (code, reason) = match &disconnect {
        Disconnect::ClientClosed => return,
        Disconnect::Rejected(RejectReason::ProtocolViolation) => {
            (close_code::PROTOCOL, "protocol violation")
        }
        Disconnect::Rejected(_) => (close_code::POLICY, "rejected"),
        Disconnect::Lagged => (RECONNECT_PROMPTLY, "too slow; reconnect and catch up"),
        Disconnect::Idle => (close_code::AWAY, "idle"),
        Disconnect::ShuttingDown => (close_code::RESTART, "server shutting down"),
        Disconnect::Failed(
            error @ (ConnectionError::Database(_)
            | ConnectionError::Records(_)
            | ConnectionError::CommitFailed(_)),
        ) => {
            tracing::error!(%error, "closing a sync connection after a storage failure");
            (close_code::ERROR, "internal error")
        }
        Disconnect::Failed(error) => {
            tracing::debug!(%error, "sync connection failed");
            return;
        }
    };
    if let Disconnect::Rejected(reason) = disconnect {
        let rejected = protocol::encode(&ServerMessage::Rejected { reason });
        if let Err(error) = send_within_timeout(socket, Message::Binary(rejected.into())).await {
            tracing::debug!(%error, "could not tell a client why it was rejected");
            return;
        }
    }
    let close = Message::Close(Some(CloseFrame {
        code,
        reason: Utf8Bytes::from_static(reason),
    }));
    if let Err(error) = send_within_timeout(socket, close).await {
        tracing::debug!(%error, "could not send a close frame");
    }
}

impl Connection {
    async fn run(&mut self, inbox: &mut mpsc::Receiver<Outgoing>) -> Disconnect {
        let mut shutdown = self.state.shutdown.clone();
        let mut keep_alive = interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
        let revalidate_interval = self.state.settings.revalidate_interval;
        let mut revalidate = interval_at(Instant::now() + revalidate_interval, revalidate_interval);
        let deferred_delivery = self.state.settings.deferred_delivery;
        let mut deferred = interval_at(Instant::now() + deferred_delivery, deferred_delivery);
        deferred.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let feed_ready = Arc::clone(&self.feed_ready);
        loop {
            // In this order: what is waiting to go out is written before the client's next message is read, so a
            // client sending fast is slowed by its own replies rather than cut off when they pile up.
            let step = tokio::select! {
                biased;
                () = self.lagged.notified() => Err(Disconnect::Lagged),
                () = self.session_ended.notified() => Err(Disconnect::Rejected(RejectReason::InvalidSession)),
                _ = shutdown.changed() => Err(Disconnect::ShuttingDown),
                Some(outgoing) = inbox.recv() => self.forward(outgoing, inbox).await,
                () = feed_ready.notified() => self.deliver_feeds().await,
                frame = self.socket.recv() => self.receive(frame).await,
                _ = deferred.tick() => self.deliver_feeds().await,
                _ = keep_alive.tick() => self.ping().await,
                _ = revalidate.tick() => self.revalidate().await,
            };
            if let Err(disconnect) = step {
                return disconnect;
            }
        }
    }

    async fn receive(
        &mut self,
        frame: Option<Result<Message, axum::Error>>,
    ) -> Result<(), Disconnect> {
        self.last_heard = Instant::now();
        let Some(message) = decode_frame(frame)? else {
            return Ok(());
        };
        match message {
            ClientMessage::Hello { .. } => {
                Err(Disconnect::Rejected(RejectReason::ProtocolViolation))
            }
            ClientMessage::OpenPage {
                workspace_id,
                page_id,
                last_seen_seq,
            } => {
                let page = PageAddress {
                    workspace_id,
                    page_id,
                };
                self.open_page(page, last_seen_seq).await
            }
            ClientMessage::ClosePage { page_id } => {
                self.close_page(page_id);
                Ok(())
            }
            ClientMessage::PushPageUpdate {
                update_id,
                page_id,
                update,
            } => self.push_update(update_id, page_id, update).await,
            ClientMessage::Presence { page_id, state } => self.relay_presence(page_id, state).await,
            ClientMessage::OpenWorkspace {
                workspace_id,
                last_seen_seq,
            } => self.open_workspace(workspace_id, last_seen_seq).await,
            ClientMessage::CloseWorkspace { workspace_id } => {
                self.unsubscribe(Topic::Workspace(workspace_id));
                Ok(())
            }
            ClientMessage::PushTransaction {
                workspace_id,
                transaction,
            } => self.push_transaction(workspace_id, transaction).await,
            ClientMessage::OpenDatabase {
                workspace_id,
                database_id,
            } => self.open_database(workspace_id, database_id).await,
            ClientMessage::CloseDatabase {
                workspace_id,
                database_id,
            } => {
                self.close_database(workspace_id, database_id);
                Ok(())
            }
        }
    }

    pub(super) fn next_subscription(&mut self) -> u64 {
        self.subscriptions_made += 1;
        self.subscriptions_made
    }

    pub(super) fn subscriber(&self, workspace_id: WorkspaceId, subscription: u64) -> Subscriber {
        Subscriber {
            subscription,
            workspace_id,
            outbox: self.outbox.clone(),
            lagged: self.lagged.clone(),
        }
    }

    /// Closing a workspace closes its databases too.
    pub(super) fn unsubscribe(&mut self, topic: Topic) -> Option<Subscription> {
        let subscription = self.subscriptions.remove(&topic)?;
        match topic {
            Topic::Page(page_id) => self.state.hub.page_room(page_id).unsubscribe(self.id),
            Topic::Workspace(workspace_id) => {
                self.state
                    .hub
                    .workspace_room(workspace_id)
                    .feed()
                    .unsubscribe(self.id);
                self.feeds.remove(&workspace_id);
                self.open_databases
                    .retain(|(workspace, _)| *workspace != workspace_id);
            }
        }
        Some(subscription)
    }

    fn close_topic(&mut self, topic: Topic) {
        match topic {
            Topic::Page(page_id) => self.close_page(page_id),
            Topic::Workspace(_) => {
                self.unsubscribe(topic);
            }
        }
    }

    fn close_everything(&mut self) {
        let topics: Vec<Topic> = self.subscriptions.keys().copied().collect();
        for topic in topics {
            self.close_topic(topic);
        }
    }

    /// Queues a reply to this connection's own request behind everything already queued, so it keeps its place
    /// in the topic's sequence order, and so it is delivered even if the topic is reopened meanwhile.
    pub(super) fn queue_reply(&self, topic: Topic, frame: Bytes) {
        let outgoing = Outgoing {
            topic,
            subscription: None,
            frame,
        };
        hub::queue(&self.outbox, &self.lagged, outgoing);
    }

    /// Writes the frame and whatever else is already waiting, dropping relays for topics this connection has since
    /// closed or reopened.
    async fn forward(
        &mut self,
        first: Outgoing,
        inbox: &mut mpsc::Receiver<Outgoing>,
    ) -> Result<(), Disconnect> {
        let mut frames = Vec::new();
        let mut next = Some(first);
        while let Some(outgoing) = next {
            if self.is_deliverable(&outgoing) {
                frames.push(outgoing.frame);
            }
            next = (frames.len() < FORWARD_BATCH)
                .then(|| inbox.try_recv().ok())
                .flatten();
        }
        self.write_frames(frames).await
    }

    /// Writes the frames in order, runs of them gathered into `Batch` frames, with one flush.
    pub(super) async fn write_frames(&mut self, frames: Vec<Bytes>) -> Result<(), Disconnect> {
        if frames.is_empty() {
            return Ok(());
        }
        let socket = &mut self.socket;
        let write = async move {
            for frame in batch(frames) {
                socket.feed(Message::Binary(frame)).await?;
            }
            socket.flush().await
        };
        match timeout(SEND_TIMEOUT, write).await {
            Ok(result) => Ok(result.map_err(ConnectionError::from)?),
            Err(_) => Err(ConnectionError::SendTimedOut.into()),
        }
    }

    /// Relays for a topic this connection has since closed or reopened are dropped; replies always go out.
    fn is_deliverable(&self, outgoing: &Outgoing) -> bool {
        match outgoing.subscription {
            None => true,
            Some(id) => self
                .subscriptions
                .get(&outgoing.topic)
                .is_some_and(|subscription| subscription.id == id),
        }
    }

    /// Closes the connection if its session has ended, and the topics of workspaces the user has left. Role
    /// changes need no check here: every push and open checks the role.
    async fn revalidate(&mut self) -> Result<(), Disconnect> {
        let pool = &self.state.pool;
        if sessions::session_user(pool, &self.session_token).await? != Some(self.user_id) {
            return Err(Disconnect::Rejected(RejectReason::InvalidSession));
        }
        let member_of: HashSet<WorkspaceId> = workspaces::list_workspaces(pool, self.user_id)
            .await?
            .into_iter()
            .map(|membership| membership.workspace_id)
            .collect();
        let lost_topics: Vec<Topic> = self
            .subscriptions
            .iter()
            .filter(|(_, subscription)| !member_of.contains(&subscription.workspace_id))
            .map(|(topic, _)| *topic)
            .collect();
        for topic in lost_topics {
            self.close_topic(topic);
            let rejected = match topic {
                Topic::Page(page_id) => ServerMessage::PageRejected {
                    page_id,
                    reason: RejectReason::NotAMember,
                },
                Topic::Workspace(workspace_id) => ServerMessage::WorkspaceRejected {
                    workspace_id,
                    reason: RejectReason::NotAMember,
                },
            };
            self.send_message(&rejected).await?;
        }
        Ok(())
    }

    async fn ping(&mut self) -> Result<(), Disconnect> {
        if self.last_heard.elapsed() > SILENCE_LIMIT {
            return Err(Disconnect::Idle);
        }
        self.send(Message::Ping(Bytes::new())).await
    }

    pub(super) async fn send_message(&mut self, message: &ServerMessage) -> Result<(), Disconnect> {
        self.send_frame(encode(message)).await
    }

    pub(super) async fn send_frame(&mut self, frame: Bytes) -> Result<(), Disconnect> {
        self.send(Message::Binary(frame)).await
    }

    async fn send(&mut self, message: Message) -> Result<(), Disconnect> {
        Ok(send_within_timeout(&mut self.socket, message).await?)
    }
}

/// Bounded, so a client that stops reading cannot hold its connection task forever.
async fn send_within_timeout(
    socket: &mut WebSocket,
    message: Message,
) -> Result<(), ConnectionError> {
    match timeout(SEND_TIMEOUT, socket.send(message)).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(ConnectionError::SendTimedOut),
    }
}

pub(super) fn encode(message: &ServerMessage) -> Bytes {
    Bytes::from(protocol::encode(message))
}

/// Gathers consecutive frames into `Batch` frames of about `BATCH_BYTES`. A frame that is alone in its run, or
/// larger than that, goes as it is.
fn batch(frames: Vec<Bytes>) -> Vec<Bytes> {
    let mut batched = Vec::new();
    let mut run: Vec<Bytes> = Vec::new();
    let mut run_bytes = 0;
    for frame in frames {
        if !run.is_empty() && run_bytes + frame.len() > BATCH_BYTES {
            batched.push(seal(std::mem::take(&mut run)));
            run_bytes = 0;
        }
        run_bytes += frame.len();
        run.push(frame);
    }
    if !run.is_empty() {
        batched.push(seal(run));
    }
    batched
}

fn seal(mut run: Vec<Bytes>) -> Bytes {
    if run.len() == 1 {
        return run.pop().expect("a run of one has a frame");
    }
    let messages = run.iter().map(|frame| frame.to_vec()).collect();
    encode(&ServerMessage::Batch { messages })
}

/// Why `user_id` may not change the workspace's records or pages, if they may not. A free function rather than a
/// method because a borrow of the connection is not `Send` (its socket is not `Sync`).
pub(super) async fn edit_refusal(
    state: &AppState,
    user_id: UserId,
    workspace_id: WorkspaceId,
) -> Result<Option<RejectReason>, DbError> {
    let role = workspaces::workspace_role(&state.pool, user_id, workspace_id).await?;
    Ok(match role {
        None => Some(RejectReason::NotAMember),
        Some(role) if !role.can_edit() => Some(RejectReason::ReadOnly),
        Some(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(bytes: usize) -> Bytes {
        Bytes::from(vec![7; bytes])
    }

    #[test]
    fn runs_of_frames_become_batches_in_order_and_large_or_lone_frames_go_as_they_are() {
        let large = frame(BATCH_BYTES + 1);
        let frames = vec![frame(10), frame(20), large.clone(), frame(30)];
        let batched = batch(frames);
        assert_eq!(batched.len(), 3);
        let first: ServerMessage = protocol::decode(&batched[0]).unwrap();
        assert_eq!(
            first,
            ServerMessage::Batch {
                messages: vec![vec![7; 10], vec![7; 20]]
            }
        );
        assert_eq!(batched[1], large);
        assert_eq!(batched[2], frame(30));
    }
}
