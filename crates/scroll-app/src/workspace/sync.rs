//! Syncing with the server: the engine's lifetime and what its events change here.

use gpui::Context;
use scroll_core::id::PageId;
use scroll_sync::{ConnectionStatus, EditRefusal, SyncEngine, SyncEvent};
use std::sync::Arc;

use super::loading::with_parents;
use super::*;

impl Workspace {
    /// Starts keeping this linked workspace in step with its server.
    pub fn start_sync(&mut self, session_token: String, cx: &mut Context<Self>) {
        let (engine, mut events) = match SyncEngine::start(&self.path, session_token, None) {
            Ok(started) => started,
            Err(error) => {
                log::error!("could not start syncing: {error}");
                cx.emit(WorkspaceEvent::Notice(format!(
                    "Could not connect: {error}"
                )));
                return;
            }
        };
        // What is already on screen is open on the new connection too.
        for page in self.open_pages.keys() {
            engine.open_page(*page);
        }
        for database in self.open_databases.keys() {
            engine.open_database(*database);
        }
        self.is_first_catch_up_pending = engine.is_first_catch_up_pending();
        if let Some(previous) = self.sync.replace(Arc::new(engine)) {
            retire_engine(previous);
        }
        self.connection = Some(ConnectionStatus::Connecting);
        self._sync_events = cx.spawn(async move |workspace, cx| {
            while let Some(event) = events.next().await {
                if workspace
                    .update(cx, |workspace, cx| workspace.handle_sync_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        cx.notify();
    }

    /// Marks a linked workspace whose session is gone, so the UI can ask to sign in.
    pub fn mark_signed_out(&mut self, cx: &mut Context<Self>) {
        self.connection = Some(ConnectionStatus::SignedOut);
        cx.emit(WorkspaceEvent::ConnectionChanged);
        cx.notify();
    }

    pub fn send_presence(&self, page: PageId, state: Vec<u8>) {
        if let Some(engine) = self.sync.as_ref() {
            engine.send_presence(page, state);
        }
    }

    /// Stops syncing. The engine finishes what it is doing on a thread of its own, so the
    /// window never waits for it (a large upload can take a while to reach a stopping point);
    /// what it had not sent stays queued in the file for next time.
    pub fn shut_down(&mut self) {
        if let Some(engine) = self.sync.take() {
            retire_engine(engine);
        }
    }

    pub(crate) fn handle_sync_event(&mut self, event: SyncEvent, cx: &mut Context<Self>) {
        match event {
            SyncEvent::Status(status) => {
                self.connection = Some(status);
                cx.emit(WorkspaceEvent::ConnectionChanged);
                cx.notify();
            }
            SyncEvent::AllRecordsChanged => {
                let was_finishing = std::mem::take(&mut self.is_finishing_download);
                self.finish_progress = None;
                self.reload_everything(cx);
                if was_finishing {
                    self.optimize_soon(cx);
                }
            }
            SyncEvent::FirstCatchUpComplete => {
                self.is_first_catch_up_pending = false;
                cx.emit(WorkspaceEvent::ConnectionChanged);
                cx.notify();
            }
            SyncEvent::RecordsChanged(changes) => self
                .records_changed_elsewhere(with_parents(changes.records, changes.child_lists), cx),
            SyncEvent::DownloadProgress {
                received_parts,
                is_complete,
            } => {
                self.downloaded_parts = (!is_complete).then_some(received_parts);
                // Everything has arrived; the engine now writes it into the file, which takes a
                // while for a large workspace and ends with `AllRecordsChanged`.
                self.is_finishing_download = is_complete;
                cx.notify();
            }
            SyncEvent::ApplyProgress { applied, total } => {
                self.is_finishing_download = true;
                self.finish_progress = Some((applied, total));
                cx.notify();
            }
            SyncEvent::UploadProgress { remaining } => {
                let has_drained = self.upload_remaining > 0 && remaining == 0;
                self.upload_remaining = remaining;
                if has_drained {
                    self.reclaim_space_in_background();
                }
                cx.notify();
            }
            SyncEvent::TransactionRejected {
                records,
                child_lists,
                ..
            } => {
                let records = with_parents(records, child_lists);
                cx.emit(WorkspaceEvent::Notice(
                    "The server did not accept a change; it was undone.".into(),
                ));
                self.records_changed_elsewhere(records, cx);
            }
            SyncEvent::TransactionTooLarge {
                records,
                child_lists,
                ..
            } => {
                let records = with_parents(records, child_lists);
                cx.emit(WorkspaceEvent::Notice(
                    "A change was too large to sync and was undone.".into(),
                ));
                self.records_changed_elsewhere(records, cx);
            }
            SyncEvent::PageCatchUp { page_id, catch_up } => {
                let mut updates: Vec<Vec<u8>> = catch_up
                    .snapshot
                    .into_iter()
                    .map(|snapshot| snapshot.bytes)
                    .collect();
                updates.extend(catch_up.updates.into_iter().map(|update| update.bytes));
                cx.emit(WorkspaceEvent::Page(
                    page_id,
                    PageSyncEvent::Updates(updates),
                ));
            }
            SyncEvent::RemotePageUpdate { page_id, update } => {
                cx.emit(WorkspaceEvent::Page(
                    page_id,
                    PageSyncEvent::Updates(vec![update.bytes]),
                ));
            }
            SyncEvent::PageUpdateAcknowledged { .. } => {}
            SyncEvent::PageEditsRefused { page_id, reason } => {
                let notice = match reason {
                    // The server says this person only views: everything follows the role.
                    EditRefusal::ReadOnly => {
                        self.set_role(Some(Role::Viewer), cx);
                        "You can view this page but not edit it."
                    }
                    EditRefusal::NoAccess
                    | EditRefusal::PageInAnotherWorkspace
                    | EditRefusal::TooLarge => {
                        "The server did not accept edits to this page; it was reloaded."
                    }
                };
                cx.emit(WorkspaceEvent::Notice(notice.into()));
                cx.emit(WorkspaceEvent::Page(page_id, PageSyncEvent::EditsRefused));
            }
            SyncEvent::PageRejected { page_id, .. } => {
                log::warn!("the server closed page {page_id:?}");
                cx.emit(WorkspaceEvent::Notice(
                    "This page is no longer available.".into(),
                ));
            }
            SyncEvent::Presence {
                page_id,
                user_id,
                connection_id,
                state,
            } => {
                cx.emit(WorkspaceEvent::Page(
                    page_id,
                    PageSyncEvent::Presence {
                        user: user_id,
                        connection: connection_id,
                        state,
                    },
                ));
            }
            SyncEvent::PresenceLeft {
                page_id,
                connection_id,
                ..
            } => {
                cx.emit(WorkspaceEvent::Page(
                    page_id,
                    PageSyncEvent::PresenceLeft {
                        connection: connection_id,
                    },
                ));
            }
            SyncEvent::Error(error) => log::warn!("sync is recovering from: {error}"),
        }
    }
}

pub(super) fn retire_engine(engine: Arc<SyncEngine>) {
    let stopping = std::thread::Builder::new()
        .name("sync engine shutdown".into())
        .spawn(move || match Arc::try_unwrap(engine) {
            Ok(engine) => engine.shut_down(),
            Err(_) => log::warn!("the sync engine is still in use; it stops when released"),
        });
    if let Err(error) = stopping {
        log::error!("could not start the thread that stops syncing: {error}");
    }
}
