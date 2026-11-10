//! The engine's thread: connects, keeps the store in step with the server, and reconnects when cut off.
//!
//! Every message from the server is written to the store before it is reported to the app, and the server
//! delivers each topic in sequence order, so the store's last-seen sequences never run ahead of what is
//! stored. After any interruption the engine reconnects, catches up from those sequences and resends
//! everything still queued; the server recognises resends by id.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use futures_util::StreamExt;
use scroll_api::connection::{RECONNECT_PROMPTLY, SILENCE_LIMIT};
use scroll_api::protocol::{self, ClientMessage, RejectReason, ServerMessage};
use scroll_core::id::{PageId, WorkspaceId};
use scroll_core::store::{SequencedBytes, StoreChanges, StoreError, WorkspaceStore};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, sleep_until};
use tokio_tungstenite::tungstenite::{self, Message};

use super::backoff::Backoff;
use super::commands::{Command, OpenDatabases};
use super::connection::{Connection, Endpoint};
use super::pages::SentUpdate;
use super::snapshot::ReceivedPart;
use super::{ConnectionStatus, SyncError, SyncEvent};
use crate::transport;

const TLS_FAILURES_BEFORE_STOPPING: u32 = 3;
/// After the server says "reconnect and catch up" (`RECONNECT_PROMPTLY`), there is no reason to wait long.
const PROMPT_RECONNECT: Duration = Duration::from_millis(100);

pub(crate) struct Worker {
    pub(crate) store: WorkspaceStore,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) socket_url: String,
    pub(crate) session_token: String,
    /// `None` uses the system's trusted certificates only.
    pub(crate) tls: Option<native_tls::TlsConnector>,
    pub(crate) commands: mpsc::UnboundedReceiver<Command>,
    pub(crate) local_work: Arc<Notify>,
    pub(crate) events: mpsc::UnboundedSender<SyncEvent>,
    /// Pages the app has open, kept across reconnects.
    pub(crate) open_pages: HashSet<PageId>,
    pub(crate) open_databases: OpenDatabases,
    /// Shared with `SyncEngine::is_first_catch_up_pending`.
    pub(crate) first_catch_up_pending: Arc<AtomicBool>,
}

/// Why a connection ended, or why the engine is not connected.
pub(super) enum Interrupt {
    ShutDown,
    SignedOut,
    AccessLost,
    Lost {
        reconnect_promptly: bool,
    },
    /// The secure connection could not be set up.
    TlsFailed(tungstenite::Error),
    Failed(SyncError),
}

impl From<SyncError> for Interrupt {
    fn from(error: SyncError) -> Self {
        Interrupt::Failed(error)
    }
}

impl From<StoreError> for Interrupt {
    fn from(error: StoreError) -> Self {
        Interrupt::Failed(error.into())
    }
}

impl From<postcard::Error> for Interrupt {
    fn from(error: postcard::Error) -> Self {
        Interrupt::Failed(error.into())
    }
}

impl From<tungstenite::Error> for Interrupt {
    fn from(error: tungstenite::Error) -> Self {
        if transport::is_tls_failure(&error) {
            return Interrupt::TlsFailed(error);
        }
        tracing::debug!(%error, "sync connection lost");
        Interrupt::Lost {
            reconnect_promptly: false,
        }
    }
}

impl Worker {
    pub(crate) fn run_on_own_runtime(mut self) {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(self.run()),
            Err(error) => {
                self.emit(SyncEvent::Error(SyncError::Runtime(error)));
                self.report(ConnectionStatus::Stopped);
            }
        }
    }

    async fn run(&mut self) {
        let mut backoff = Backoff::default();
        let mut tls_failures = 0;
        loop {
            self.report(ConnectionStatus::Connecting);
            let interrupt = self.connect_and_sync(&mut backoff).await;
            tls_failures = match interrupt {
                Interrupt::TlsFailed(_) => tls_failures + 1,
                _ => 0,
            };
            let delay = match interrupt {
                // One failed handshake can be a network hiccup; several in a row will not fix themselves.
                Interrupt::TlsFailed(error) if tls_failures >= TLS_FAILURES_BEFORE_STOPPING => {
                    self.emit(SyncEvent::Error(SyncError::ServerNotTrusted(error)));
                    return self.report(ConnectionStatus::ServerNotTrusted);
                }
                Interrupt::TlsFailed(error) => {
                    tracing::warn!(%error, "secure connection failed; retrying");
                    backoff.next_delay()
                }
                Interrupt::ShutDown => break,
                Interrupt::SignedOut => return self.report(ConnectionStatus::SignedOut),
                Interrupt::AccessLost => return self.report(ConnectionStatus::AccessLost),
                Interrupt::Failed(SyncError::UnsupportedProtocolVersion) => {
                    self.emit(SyncEvent::Error(SyncError::UnsupportedProtocolVersion));
                    break;
                }
                Interrupt::Failed(error) => {
                    tracing::warn!(%error, "sync failed; reconnecting");
                    self.emit(SyncEvent::Error(error));
                    backoff.next_delay()
                }
                Interrupt::Lost {
                    reconnect_promptly: true,
                } => PROMPT_RECONNECT,
                Interrupt::Lost { .. } => backoff.next_delay(),
            };
            self.report(ConnectionStatus::Offline);
            if let Err(Interrupt::ShutDown) = self.wait_offline(delay).await {
                break;
            }
        }
        self.report(ConnectionStatus::Stopped);
    }

    async fn connect_and_sync(&mut self, backoff: &mut Backoff) -> Interrupt {
        let endpoint = Endpoint {
            socket_url: &self.socket_url,
            session_token: &self.session_token,
            tls: self.tls.as_ref(),
        };
        let mut connection = match Connection::open(&endpoint).await {
            Ok(connection) => connection,
            Err(interrupt) => return interrupt,
        };
        backoff.reset();
        self.report(ConnectionStatus::Online);
        let Err(interrupt) = self.sync(&mut connection).await;
        if let Interrupt::ShutDown = interrupt {
            connection.close().await;
        }
        interrupt
    }

    /// Runs until the connection ends; the `Err` says why.
    async fn sync(
        &mut self,
        connection: &mut Connection,
    ) -> Result<std::convert::Infallible, Interrupt> {
        self.resume_unfinished_snapshot()?;
        let last_seen_seq = self.store.last_seen_workspace_seq()?;
        let open_workspace = ClientMessage::OpenWorkspace {
            workspace_id: self.workspace_id,
            last_seen_seq,
        };
        connection.send(&open_workspace).await?;
        for database_id in self.open_databases.to_vec() {
            self.open_database_on_server(connection, database_id)
                .await?;
        }
        for page_id in self.open_pages.clone() {
            self.open_on_server(connection, page_id).await?;
        }
        self.send_queue(connection).await?;
        loop {
            tokio::select! {
                frame = connection.socket.next() => self.receive(connection, frame).await?,
                command = self.commands.recv() => self.obey(command, Some(connection)).await?,
                () = self.local_work.notified() => self.send_queue(connection).await?,
                () = sleep_until(connection.last_heard + SILENCE_LIMIT) => {
                    tracing::debug!("the server went silent");
                    return Err(Interrupt::Lost { reconnect_promptly: false });
                }
            }
        }
    }

    async fn receive(
        &mut self,
        connection: &mut Connection,
        frame: Option<Result<Message, tungstenite::Error>>,
    ) -> Result<(), Interrupt> {
        connection.last_heard = Instant::now();
        match frame {
            None => Err(Interrupt::Lost {
                reconnect_promptly: false,
            }),
            Some(Err(error)) => Err(error.into()),
            Some(Ok(Message::Close(close))) => Err(Interrupt::Lost {
                reconnect_promptly: close
                    .is_some_and(|close| u16::from(close.code) == RECONNECT_PROMPTLY),
            }),
            Some(Ok(Message::Binary(bytes))) => match protocol::decode(&bytes)? {
                ServerMessage::Batch { messages } => {
                    let mut acks = Vec::new();
                    for bytes in messages {
                        match protocol::decode(&bytes)? {
                            ServerMessage::TransactionAck {
                                transaction_id,
                                seq,
                                ..
                            } => acks.push((transaction_id, seq)),
                            message => {
                                // In order: the acknowledgements before this message apply first.
                                self.acknowledge(connection, std::mem::take(&mut acks))
                                    .await?;
                                self.handle(connection, message).await?;
                            }
                        }
                    }
                    self.acknowledge(connection, acks).await
                }
                message => self.handle(connection, message).await,
            },
            Some(Ok(_)) => Ok(()),
        }
    }

    /// Every kind of message the server sends, each handed to its topic: a new kind does not compile until it
    /// is handled here.
    async fn handle(
        &mut self,
        connection: &mut Connection,
        message: ServerMessage,
    ) -> Result<(), Interrupt> {
        match message {
            ServerMessage::WorkspaceCatchUpPart {
                index,
                is_last,
                part,
                ..
            } => {
                let received = ReceivedPart {
                    index,
                    is_last,
                    part,
                };
                self.receive_catch_up_part(connection, received)
            }
            ServerMessage::RemoteTransaction {
                seq, transaction, ..
            } => self.apply_remote_transaction(seq, &transaction),
            ServerMessage::TransactionAck {
                transaction_id,
                seq,
                ..
            } => {
                self.acknowledge(connection, vec![(transaction_id, seq)])
                    .await
            }
            ServerMessage::TransactionRejected {
                transaction_id,
                reason,
                ..
            } => {
                self.reject_transaction(connection, transaction_id, reason)
                    .await
            }
            ServerMessage::PageCatchUp { page_id, catch_up } => {
                self.store_page_catch_up(page_id, catch_up)
            }
            ServerMessage::RemotePageUpdate {
                page_id,
                seq,
                update,
                ..
            } => self.store_remote_page_update(page_id, SequencedBytes { seq, bytes: update }),
            ServerMessage::PageUpdateAck {
                update_id,
                page_id,
                seq,
            } => {
                let update = SentUpdate { page_id, update_id };
                self.acknowledge_page_update(connection, update, seq).await
            }
            ServerMessage::PageUpdateRejected {
                update_id,
                page_id,
                reason,
            } => {
                let update = SentUpdate { page_id, update_id };
                self.page_update_rejected(connection, update, reason).await
            }
            ServerMessage::PageRejected { page_id, reason } => {
                self.page_rejected(connection, page_id, reason);
                Ok(())
            }
            ServerMessage::Presence {
                page_id,
                user_id,
                connection_id,
                state,
            } => {
                let presence = SyncEvent::Presence {
                    page_id,
                    user_id,
                    connection_id,
                    state,
                };
                self.emit_for_open_page(page_id, presence);
                Ok(())
            }
            ServerMessage::PresenceLeft {
                page_id,
                user_id,
                connection_id,
            } => {
                let left = SyncEvent::PresenceLeft {
                    page_id,
                    user_id,
                    connection_id,
                };
                self.emit_for_open_page(page_id, left);
                Ok(())
            }
            ServerMessage::WorkspaceRejected { reason, .. } => match reason {
                RejectReason::NotAMember => Err(Interrupt::AccessLost),
                _ => Err(SyncError::UnexpectedMessage("a workspace rejection").into()),
            },
            ServerMessage::Rejected { reason } => match reason {
                RejectReason::InvalidSession => Err(Interrupt::SignedOut),
                _ => Err(Interrupt::Lost {
                    reconnect_promptly: false,
                }),
            },
            ServerMessage::Welcome { .. } => {
                Err(SyncError::UnexpectedMessage("a second welcome").into())
            }
            ServerMessage::Batch { .. } => {
                Err(SyncError::UnexpectedMessage("a batch inside a batch").into())
            }
        }
    }

    pub(super) fn emit_changes(&self, changes: StoreChanges) {
        if changes.is_everything {
            self.emit(SyncEvent::AllRecordsChanged);
        } else if !changes.records.is_empty() || !changes.child_lists.is_empty() {
            self.emit(SyncEvent::RecordsChanged(changes));
        }
    }

    pub(super) fn report(&self, status: ConnectionStatus) {
        self.emit(SyncEvent::Status(status));
    }

    pub(super) fn emit(&self, event: SyncEvent) {
        if self.events.send(event).is_err() {
            tracing::debug!("the app has stopped listening for sync events");
        }
    }
}
