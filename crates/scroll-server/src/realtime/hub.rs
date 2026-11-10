//! In-process fan out: the room of each open page (its subscribers) and of each open workspace (its feed),
//! each with the lock that keeps its changes in sequence order.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Bytes;
use scroll_api::protocol::ConnectionId;
use scroll_core::id::{PageId, WorkspaceId};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Notify, mpsc};

use super::feed::Feed;
use crate::lock;

/// What a client subscribes to. Each has its own sequence: a page's updates, or a workspace's transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    Page(PageId),
    Workspace(WorkspaceId),
}

/// One encoded `ServerMessage` for a connection. A relay is tagged with the subscription it belongs to, so the
/// connection can drop it once that topic is closed or reopened; a reply to the connection's own request (an
/// ack) has no subscription and is always delivered.
pub struct Outgoing {
    pub topic: Topic,
    pub subscription: Option<u64>,
    pub frame: Bytes,
}

#[derive(Clone)]
pub struct Subscriber {
    pub subscription: u64,
    /// The workspace the topic was opened in. Relays are limited to it, so a page id that turns out to belong to
    /// another workspace never leaks that workspace's content.
    pub workspace_id: WorkspaceId,
    pub outbox: mpsc::Sender<Outgoing>,
    /// Signalled when the outbox is full. The connection then closes, and the client reconnects and catches up,
    /// rather than one slow reader holding up everyone else.
    pub lagged: Arc<Notify>,
}

impl Subscriber {
    pub fn deliver(&self, topic: Topic, frame: Bytes) {
        let outgoing = Outgoing {
            topic,
            subscription: Some(self.subscription),
            frame,
        };
        queue(&self.outbox, &self.lagged, outgoing);
    }
}

/// Queues without waiting; a full outbox signals `lagged`. A closed one belongs to a connection that is already
/// cleaning up.
pub fn queue(outbox: &mpsc::Sender<Outgoing>, lagged: &Notify, outgoing: Outgoing) {
    if let Err(TrySendError::Full(_)) = outbox.try_send(outgoing) {
        lagged.notify_one();
    }
}

/// A room goes when nobody is in it.
pub trait Room: Default {
    fn is_empty(&self) -> bool;
}

#[derive(Default)]
pub struct PageRoom {
    sequencing: tokio::sync::Mutex<()>,
    subscribers: Mutex<HashMap<ConnectionId, Subscriber>>,
}

#[derive(Default)]
pub struct WorkspaceRoom {
    sequencing: tokio::sync::Mutex<()>,
    feed: Arc<Mutex<Feed>>,
}

impl Room for PageRoom {
    fn is_empty(&self) -> bool {
        lock(&self.subscribers).is_empty()
    }
}

impl Room for WorkspaceRoom {
    fn is_empty(&self) -> bool {
        !lock(&self.feed).has_readers()
    }
}

struct Rooms<K, R> {
    rooms: Mutex<HashMap<K, Arc<R>>>,
}

impl<K, R> Default for Rooms<K, R> {
    fn default() -> Self {
        Self {
            rooms: Mutex::default(),
        }
    }
}

#[derive(Default)]
pub struct Hub {
    pages: Rooms<PageId, PageRoom>,
    workspaces: Rooms<WorkspaceId, WorkspaceRoom>,
}

impl Hub {
    pub fn page_room(&self, page_id: PageId) -> RoomHandle<'_, PageId, PageRoom> {
        RoomHandle::new(&self.pages, page_id)
    }

    pub fn workspace_room(
        &self,
        workspace_id: WorkspaceId,
    ) -> RoomHandle<'_, WorkspaceId, WorkspaceRoom> {
        RoomHandle::new(&self.workspaces, workspace_id)
    }
}

/// Access to one room. Dropping the last handle of an empty room removes it.
pub struct RoomHandle<'hub, K: Copy + Eq + Hash, R: Room> {
    rooms: &'hub Rooms<K, R>,
    key: K,
    room: Arc<R>,
}

impl<'hub, K: Copy + Eq + Hash, R: Room> RoomHandle<'hub, K, R> {
    fn new(rooms: &'hub Rooms<K, R>, key: K) -> Self {
        let room = lock(&rooms.rooms).entry(key).or_default().clone();
        Self { rooms, key, room }
    }
}

impl<K: Copy + Eq + Hash, R: Room> Deref for RoomHandle<'_, K, R> {
    type Target = R;

    fn deref(&self) -> &R {
        &self.room
    }
}

impl<K: Copy + Eq + Hash, R: Room> Drop for RoomHandle<'_, K, R> {
    fn drop(&mut self) {
        let mut rooms = lock(&self.rooms.rooms);
        // Handles are only made under the rooms lock, so a count of two (the map's and ours) means nobody else can
        // be using this room, and an empty one can go without losing a concurrent push's relay.
        if Arc::strong_count(&self.room) == 2 && self.room.is_empty() {
            rooms.remove(&self.key);
        }
    }
}

impl PageRoom {
    /// Held from storing an update until it is relayed, and while a subscriber joins, so every subscriber sees the
    /// page's updates in sequence order with nothing missed between its catch-up and the live stream.
    pub async fn lock_sequencing(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.sequencing.lock().await
    }

    pub fn subscribe(&self, connection_id: ConnectionId, subscriber: Subscriber) {
        lock(&self.subscribers).insert(connection_id, subscriber);
    }

    pub fn unsubscribe(&self, connection_id: ConnectionId) {
        lock(&self.subscribers).remove(&connection_id);
    }
}

impl RoomHandle<'_, PageId, PageRoom> {
    /// Sends to every subscriber in `workspace_id` except `sender`.
    pub fn relay(&self, sender: ConnectionId, workspace_id: WorkspaceId, frame: &Bytes) {
        for (connection_id, subscriber) in lock(&self.subscribers).iter() {
            if *connection_id != sender && subscriber.workspace_id == workspace_id {
                subscriber.deliver(Topic::Page(self.key), frame.clone());
            }
        }
    }
}

impl WorkspaceRoom {
    /// Held while a group commits and its transactions join the feed, and while a reader joins, so readers see
    /// the workspace's transactions in sequence order with nothing missed between catch-up and feed.
    pub async fn lock_sequencing(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.sequencing.lock().await
    }

    pub fn feed(&self) -> MutexGuard<'_, Feed> {
        lock(&self.feed)
    }

    /// For a connection reading the feed often, without the hub's lock each time. The room stays while the
    /// connection is one of the feed's readers.
    pub fn shared_feed(&self) -> Arc<Mutex<Feed>> {
        Arc::clone(&self.feed)
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt;

    use super::*;

    fn room_count(hub: &Hub) -> usize {
        lock(&hub.pages.rooms).len()
    }

    #[test]
    fn a_room_is_removed_only_when_it_has_no_subscribers_and_no_other_handles() {
        let hub = Hub::default();
        let page_id = PageId::generate();
        let (outbox, _inbox) = mpsc::channel(1);
        let subscriber = Subscriber {
            subscription: 1,
            workspace_id: WorkspaceId::generate(),
            outbox,
            lagged: Arc::default(),
        };
        let connection_id = ConnectionId(uuid::Uuid::now_v7());
        hub.page_room(page_id).subscribe(connection_id, subscriber);
        assert_eq!(room_count(&hub), 1);

        let pushing = hub.page_room(page_id);
        let leaving = hub.page_room(page_id);
        leaving.unsubscribe(connection_id);
        drop(leaving);
        assert_eq!(room_count(&hub), 1, "a push still holds the room");

        drop(pushing);
        assert_eq!(room_count(&hub), 0);
    }

    #[test]
    fn a_full_outbox_signals_lag_instead_of_waiting() {
        let (outbox, _inbox) = mpsc::channel(1);
        let lagged = Arc::new(Notify::new());
        let subscriber = Subscriber {
            subscription: 1,
            workspace_id: WorkspaceId::generate(),
            outbox,
            lagged: lagged.clone(),
        };
        let page_id = PageId::generate();

        subscriber.deliver(Topic::Page(page_id), Bytes::from_static(b"fits"));
        subscriber.deliver(Topic::Page(page_id), Bytes::from_static(b"overflows"));

        assert!(lagged.notified().now_or_never().is_some());
    }
}
