//! Page content: opening pages, pushing Loro updates, and presence.

use scroll_api::protocol::{RejectReason, ServerMessage};
use scroll_core::id::PageId;
use uuid::Uuid;

use super::connection::{Connection, Disconnect, Subscription, edit_refusal, encode};
use super::hub::Topic;
use crate::pages::{self, PageAddress, PageUpdate, StoreOutcome};
use crate::workspaces;

impl Connection {
    pub(super) async fn open_page(
        &mut self,
        page: PageAddress,
        last_seen_seq: u64,
    ) -> Result<(), Disconnect> {
        let pool = &self.state.pool;
        if workspaces::workspace_role(pool, self.user_id, page.workspace_id)
            .await?
            .is_none()
        {
            return self
                .reject_page(page.page_id, RejectReason::NotAMember)
                .await;
        }
        if pages::page_workspace(pool, page.page_id)
            .await?
            .is_some_and(|owner| owner != page.workspace_id)
        {
            return self
                .reject_page(page.page_id, RejectReason::PageInAnotherWorkspace)
                .await;
        }
        let topic = Topic::Page(page.page_id);
        let subscription = Subscription {
            workspace_id: page.workspace_id,
            id: self.next_subscription(),
        };
        let subscriber = self.subscriber(page.workspace_id, subscription.id);
        {
            let room = self.state.hub.page_room(page.page_id);
            let _sequencing = room.lock_sequencing().await;
            let catch_up = pages::catch_up_page(&self.state.pool, page, last_seen_seq).await?;
            let message = ServerMessage::PageCatchUp {
                page_id: page.page_id,
                catch_up,
            };
            subscriber.deliver(topic, encode(&message));
            room.subscribe(self.id, subscriber);
        }
        self.subscriptions.insert(topic, subscription);
        Ok(())
    }

    pub(super) fn close_page(&mut self, page_id: PageId) {
        let topic = Topic::Page(page_id);
        let Some(subscription) = self.unsubscribe(topic) else {
            return;
        };
        let left = ServerMessage::PresenceLeft {
            page_id,
            user_id: self.user_id,
            connection_id: self.id,
        };
        self.state
            .hub
            .page_room(page_id)
            .relay(self.id, subscription.workspace_id, &encode(&left));
    }

    pub(super) async fn push_update(
        &mut self,
        update_id: Uuid,
        page_id: PageId,
        bytes: Vec<u8>,
    ) -> Result<(), Disconnect> {
        let Some(subscription) = self.subscriptions.get(&Topic::Page(page_id)).copied() else {
            return self
                .reject_update(update_id, page_id, RejectReason::PageNotOpen)
                .await;
        };
        let page = PageAddress {
            workspace_id: subscription.workspace_id,
            page_id,
        };
        if let Some(reason) = edit_refusal(&self.state, self.user_id, page.workspace_id).await? {
            return self.reject_update(update_id, page_id, reason).await;
        }
        let update = PageUpdate {
            update_id,
            page,
            author: self.user_id,
            bytes,
        };
        let outcome = self.store_and_relay(update).await?;
        if outcome == StoreOutcome::PageInAnotherWorkspace {
            return self
                .reject_update(update_id, page_id, RejectReason::PageInAnotherWorkspace)
                .await;
        }
        Ok(())
    }

    /// The ack goes through this connection's own outbox under the page's sequencing lock, so it is ordered with
    /// the relays of other clients' updates. Takes `&mut self` only because the socket is not `Sync`, so a shared
    /// borrow of the connection cannot be held across an await.
    async fn store_and_relay(&mut self, update: PageUpdate) -> Result<StoreOutcome, Disconnect> {
        let PageAddress {
            workspace_id,
            page_id,
        } = update.page;
        let topic = Topic::Page(page_id);
        let room = self.state.hub.page_room(page_id);
        let _sequencing = room.lock_sequencing().await;
        let outcome = pages::store_page_update(&self.state.pool, &update).await?;
        let seq = match outcome {
            StoreOutcome::Stored {
                seq,
                updates_since_snapshot,
            } => {
                self.state
                    .compactor
                    .consider(update.page, updates_since_snapshot);
                let remote = ServerMessage::RemotePageUpdate {
                    page_id,
                    seq,
                    author: self.user_id,
                    update: update.bytes,
                };
                room.relay(self.id, workspace_id, &encode(&remote));
                seq
            }
            StoreOutcome::AlreadyStored { seq } => seq,
            StoreOutcome::PageInAnotherWorkspace => return Ok(outcome),
        };
        let ack = ServerMessage::PageUpdateAck {
            update_id: update.update_id,
            page_id,
            seq,
        };
        self.queue_reply(topic, encode(&ack));
        Ok(outcome)
    }

    pub(super) async fn relay_presence(
        &mut self,
        page_id: PageId,
        state: Vec<u8>,
    ) -> Result<(), Disconnect> {
        let topic = Topic::Page(page_id);
        let Some(subscription) = self.subscriptions.get(&topic).copied() else {
            return self.reject_page(page_id, RejectReason::PageNotOpen).await;
        };
        let presence = ServerMessage::Presence {
            page_id,
            user_id: self.user_id,
            connection_id: self.id,
            state,
        };
        self.state.hub.page_room(page_id).relay(
            self.id,
            subscription.workspace_id,
            &encode(&presence),
        );
        Ok(())
    }

    async fn reject_page(
        &mut self,
        page_id: PageId,
        reason: RejectReason,
    ) -> Result<(), Disconnect> {
        self.send_message(&ServerMessage::PageRejected { page_id, reason })
            .await
    }

    async fn reject_update(
        &mut self,
        update_id: Uuid,
        page_id: PageId,
        reason: RejectReason,
    ) -> Result<(), Disconnect> {
        self.send_message(&ServerMessage::PageUpdateRejected {
            update_id,
            page_id,
            reason,
        })
        .await
    }
}
