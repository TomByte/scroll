//! The handle the app holds while a linked workspace is open, and the events it drains.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use scroll_api::protocol::{ConnectionId, RejectReason, TransactionRejection};
use scroll_core::id::{DatabaseId, PageId, TransactionId, UserId};
use scroll_core::operation::RecordId;
use scroll_core::store::{
    PageCatchUp, SequencedBytes, StoreChanges, StoreError, WorkspaceLink, WorkspaceStore,
};
use tokio::sync::{Notify, mpsc};

mod backoff;
mod commands;
mod connection;
mod pages;
mod records;
mod snapshot;
mod uploads;
mod worker;

use crate::transport;
pub(crate) use commands::Command;
use worker::Worker;

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the workspace is not linked to a server")]
    NotLinked,
    #[error("the workspace's server URL must start with http:// or https://, not {0:?}")]
    InvalidServerUrl(String),
    #[error("the extra root certificate is not usable: {0}")]
    InvalidCertificate(native_tls::Error),
    #[error("could not start the sync thread: {0}")]
    Thread(#[from] io::Error),
}

/// Failures the engine recovers from by reconnecting, reported for logging.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("a server message could not be decoded: {0}")]
    Decoding(#[from] postcard::Error),
    #[error("the server sent {0} unexpectedly")]
    UnexpectedMessage(&'static str),
    #[error("the server does not support this app's protocol version; update the app")]
    UnsupportedProtocolVersion,
    #[error("could not start the sync runtime: {0}")]
    Runtime(io::Error),
    #[error("the server's certificate is not trusted, or the secure connection keeps failing: {0}")]
    ServerNotTrusted(tokio_tungstenite::tungstenite::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Not connected; it retries on its own. Local changes stay queued meanwhile.
    Offline,
    Connecting,
    Online,
    /// The session ended (signed out or expired). The engine has stopped: sign in and start a new one.
    SignedOut,
    /// The user is no longer a member of the workspace. The engine has stopped.
    AccessLost,
    /// Secure connections to the server keep failing, most likely because its certificate is not trusted.
    /// The engine has stopped; an `Error` event before this one has the details.
    ServerNotTrusted,
    /// The engine has stopped: it was shut down, or met a failure it cannot recover from.
    Stopped,
}

#[derive(Debug)]
pub enum SyncEvent {
    Status(ConnectionStatus),
    /// The server's snapshot replaced every record: refresh everything shown.
    AllRecordsChanged,
    /// A catch-up arriving in several parts of about a megabyte (joining a large workspace). The server streams
    /// them without knowing the total in advance, so show the amount received rather than a fraction.
    DownloadProgress {
        received_parts: u32,
        is_complete: bool,
    },
    /// The first catch-up of this workspace file is in place: the records are as on the server when it
    /// connected, even when that is nothing. Sent once per file, never again after a restart; see
    /// `SyncEngine::is_first_catch_up_pending`.
    FirstCatchUpComplete,
    /// A downloaded snapshot (a first join) is being applied: `applied` of its `total` records so far. Follows
    /// the last `DownloadProgress`; sent at most about four times a second and once at `applied == total`;
    /// `AllRecordsChanged` follows when the records are all in place.
    ApplyProgress {
        applied: usize,
        total: usize,
    },
    /// Local changes the server has not yet acknowledged, for showing upload progress; 0 when all are up.
    UploadProgress {
        remaining: usize,
    },
    /// Remote transactions changed these records and page lists (`StoreChanges::child_lists`, for the sidebar):
    /// refresh them and any list showing them. Never `is_everything`: that comes as `AllRecordsChanged`.
    RecordsChanged(StoreChanges),
    /// The server refused a local transaction and the store has already undone it: refresh `records` and the
    /// page lists in `child_lists` (see `StoreChanges`).
    TransactionRejected {
        transaction_id: TransactionId,
        reason: TransactionRejection,
        records: Vec<RecordId>,
        child_lists: Vec<Option<PageId>>,
    },
    /// A local transaction is larger than the server accepts, so it was undone without being sent: refresh as for
    /// `TransactionRejected`.
    TransactionTooLarge {
        transaction_id: TransactionId,
        records: Vec<RecordId>,
        child_lists: Vec<Option<PageId>>,
    },
    /// For an open page: content from the server, already stored. Import the snapshot, then the updates,
    /// into the live document. Every `open_page` is answered by one of these.
    PageCatchUp {
        page_id: PageId,
        catch_up: PageCatchUp,
    },
    /// For an open page: another client's update, already stored. Import its bytes into the live document.
    RemotePageUpdate {
        page_id: PageId,
        update: SequencedBytes,
    },
    /// For an open page: one of this device's updates is now on the server at `seq`.
    PageUpdateAcknowledged {
        page_id: PageId,
        seq: u64,
    },
    /// The server will not take this device's edits to the page. The store has dropped them and cleared the
    /// page's stored content. If the page is open, discard its live document, load it again from the store and
    /// keep importing events: a `PageCatchUp` with the server's content follows. If `reason` is
    /// `EditRefusal::ReadOnly`, show the page read-only.
    PageEditsRefused {
        page_id: PageId,
        reason: EditRefusal,
    },
    /// The server closed an open page: the user lost access, or the id belongs to another workspace.
    PageRejected {
        page_id: PageId,
        reason: RejectReason,
    },
    Presence {
        page_id: PageId,
        user_id: UserId,
        connection_id: ConnectionId,
        state: Vec<u8>,
    },
    PresenceLeft {
        page_id: PageId,
        user_id: UserId,
        connection_id: ConnectionId,
    },
    /// Something failed and the engine is reconnecting to recover. For logs.
    Error(SyncError),
}

/// Why the server will not take this device's edits to a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRefusal {
    /// The user may only view this workspace.
    ReadOnly,
    /// The user is no longer a member of the workspace.
    NoAccess,
    /// The page id already belongs to another workspace.
    PageInAnotherWorkspace,
    /// An update was larger than the server accepts.
    TooLarge,
}

impl EditRefusal {
    /// `None` for refusals that pass: the update is sent again after reconnecting.
    pub(crate) fn from_reason(reason: RejectReason) -> Option<EditRefusal> {
        match reason {
            RejectReason::ReadOnly => Some(EditRefusal::ReadOnly),
            RejectReason::NotAMember => Some(EditRefusal::NoAccess),
            RejectReason::PageInAnotherWorkspace => Some(EditRefusal::PageInAnotherWorkspace),
            _ => None,
        }
    }
}

/// Keeps one linked workspace file in step with its server, on its own thread.
pub struct SyncEngine {
    commands: mpsc::UnboundedSender<Command>,
    local_work: Arc<Notify>,
    first_catch_up_pending: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

/// The engine's events, in the order they happened. Drain them from the app's own executor.
pub struct SyncEvents {
    receiver: mpsc::UnboundedReceiver<SyncEvent>,
}

impl SyncEngine {
    /// Call when the app opens a linked workspace, with the session token from the Keychain. The engine opens
    /// its own connection to the file; the server comes from the file's link. The system's trusted
    /// certificates apply, plus `root_certificate` (PEM) if given, as for `AccountClient::new`.
    pub fn start(
        workspace_path: &Path,
        session_token: String,
        root_certificate: Option<&[u8]>,
    ) -> Result<(SyncEngine, SyncEvents), StartError> {
        let tls = root_certificate
            .map(transport::tls_connector)
            .transpose()
            .map_err(StartError::InvalidCertificate)?;
        let store = WorkspaceStore::open(workspace_path)?;
        let WorkspaceLink::Linked {
            server_url,
            workspace_id,
        } = store.link()?
        else {
            return Err(StartError::NotLinked);
        };
        let socket_url = scroll_api::paths::socket_url(&server_url)
            .ok_or(StartError::InvalidServerUrl(server_url))?;
        let first_catch_up_pending = Arc::new(AtomicBool::new(
            store.last_seen_workspace_seq()? == 0 || store.unfinished_snapshot()?.is_some(),
        ));
        let (commands, command_receiver) = mpsc::unbounded_channel();
        let (event_sender, receiver) = mpsc::unbounded_channel();
        let local_work = Arc::new(Notify::new());
        let worker = Worker {
            store,
            workspace_id,
            socket_url,
            session_token,
            tls,
            commands: command_receiver,
            local_work: Arc::clone(&local_work),
            events: event_sender,
            open_pages: Default::default(),
            open_databases: Default::default(),
            first_catch_up_pending: Arc::clone(&first_catch_up_pending),
        };
        let thread = thread::Builder::new()
            .name("scroll-sync".into())
            .spawn(move || worker.run_on_own_runtime())?;
        let engine = SyncEngine {
            commands,
            local_work,
            first_catch_up_pending,
            thread: Some(thread),
        };
        Ok((engine, SyncEvents { receiver }))
    }

    /// Whether this file still waits for its first catch-up from the server: true until `FirstCatchUpComplete`.
    /// Read from what the file has stored, so it is right across restarts; a file that has never received a
    /// transaction (its workspace on the server is empty) stays pending until the engine first connects.
    pub fn is_first_catch_up_pending(&self) -> bool {
        self.first_catch_up_pending.load(Ordering::Acquire)
    }

    /// Call after each local change written to the store (`apply_local_transaction`,
    /// `record_local_page_update`). Cheap; repeated calls before the engine wakes are merged.
    pub fn local_change_queued(&self) {
        self.local_work.notify_one();
    }

    /// Call when the editor opens a page. Answered by a `PageCatchUp` event; remote updates and presence for
    /// the page follow until `close_page`.
    pub fn open_page(&self, page_id: PageId) {
        self.send(Command::OpenPage(page_id));
    }

    /// Call when the editor closes a page. The engine then compacts its stored content.
    pub fn close_page(&self, page_id: PageId) {
        self.send(Command::ClosePage(page_id));
    }

    /// Call when a database's rows come into view (a table, or a row's page). The server then sends changes to
    /// its rows at once; otherwise they arrive within a second or so, with other changes. A database is open
    /// or not: with several views of one, close it when the last one goes.
    pub fn open_database(&self, database_id: DatabaseId) {
        self.send(Command::OpenDatabase(database_id));
    }

    /// Call when no view of the database's rows remains.
    pub fn close_database(&self, database_id: DatabaseId) {
        self.send(Command::CloseDatabase(database_id));
    }

    /// Call when the local cursor or selection on an open page changes. Dropped while offline.
    pub fn send_presence(&self, page_id: PageId, state: Vec<u8>) {
        self.send(Command::Presence { page_id, state });
    }

    /// Call when the workspace is closed. Waits for the engine to stop; dropping the engine does the same.
    pub fn shut_down(mut self) {
        self.stop();
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            tracing::debug!("the sync engine has already stopped; command ignored");
        }
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.send(Command::ShutDown);
        if thread.join().is_err() {
            tracing::error!("the sync thread panicked");
        }
    }
}

impl Drop for SyncEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

impl SyncEvents {
    /// Call in a loop on the app's executor. `None` once the engine has stopped and every event is drained.
    pub async fn next(&mut self) -> Option<SyncEvent> {
        self.receiver.recv().await
    }

    /// Call to drain without waiting, e.g. once per frame. `None` when nothing is waiting.
    pub fn try_next(&mut self) -> Option<SyncEvent> {
        self.receiver.try_recv().ok()
    }
}
