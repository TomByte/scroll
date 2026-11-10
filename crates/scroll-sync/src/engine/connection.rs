//! One connection to the server: the socket, what has been sent on it and not yet answered, and what is open on
//! it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use scroll_api::connection::SILENCE_LIMIT;
use scroll_api::limits::MAX_SERVER_FRAME_BYTES;
use scroll_api::protocol::{self, ClientMessage, PROTOCOL_VERSION, RejectReason, ServerMessage};
use scroll_core::id::PageId;
use scroll_core::store::QueuedChange;
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use super::SyncError;
use super::worker::Interrupt;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
/// How much may be sent and not yet answered: enough to keep the server busy, small enough that the engine
/// gets back to reading soon.
const WINDOW_BYTES: usize = 2 * 1024 * 1024;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Where and as whom to connect.
pub(super) struct Endpoint<'a> {
    pub socket_url: &'a str,
    pub session_token: &'a str,
    /// `None` uses the system's trusted certificates only.
    pub tls: Option<&'a native_tls::TlsConnector>,
}

pub(super) struct Connection {
    pub(super) socket: Socket,
    pub(super) last_heard: Instant,
    /// Queued changes read from the store and not yet sent on this connection, oldest first.
    pub(super) unsent: VecDeque<QueuedChange>,
    /// The queue position of the last change read into `unsent`.
    pub(super) read_position: u64,
    /// Changes sent on this connection and not yet answered, with their size on the wire; not sent again until
    /// reconnecting. One the server refused for a passing reason stays here with no size.
    pub(super) in_flight: HashMap<Uuid, usize>,
    pub(super) in_flight_bytes: usize,
    pub(super) open_pages: HashSet<PageId>,
    /// Updates in flight per page, so a page opened only to upload them can be closed afterwards.
    pub(super) uploads: HashMap<PageId, usize>,
    /// The sequence number of the snapshot being staged part by part, until its last part arrives.
    pub(super) staging_snapshot: Option<u64>,
    /// Queued changes not yet acknowledged, as last reported to the app.
    pub(super) remaining_uploads: Option<usize>,
}

impl Connection {
    /// Connects and says hello; the server welcomes the session or says why not.
    pub(super) async fn open(endpoint: &Endpoint<'_>) -> Result<Connection, Interrupt> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_SERVER_FRAME_BYTES))
            .max_frame_size(Some(MAX_SERVER_FRAME_BYTES));
        let connector = endpoint.tls.cloned().map(Connector::NativeTls);
        let connecting = tokio_tungstenite::connect_async_tls_with_config(
            endpoint.socket_url,
            Some(config),
            true,
            connector,
        );
        let (socket, _) =
            timeout(CONNECT_TIMEOUT, connecting)
                .await
                .map_err(|_| Interrupt::Lost {
                    reconnect_promptly: false,
                })??;
        let mut connection = Connection::new(socket);
        let hello = ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            session_token: endpoint.session_token.to_owned(),
        };
        connection.send(&hello).await?;
        match connection.receive().await? {
            ServerMessage::Welcome { .. } => Ok(connection),
            ServerMessage::Rejected {
                reason: RejectReason::InvalidSession,
            } => Err(Interrupt::SignedOut),
            ServerMessage::Rejected {
                reason: RejectReason::UnsupportedProtocolVersion,
            } => Err(SyncError::UnsupportedProtocolVersion.into()),
            _ => Err(SyncError::UnexpectedMessage("something other than a welcome").into()),
        }
    }

    fn new(socket: Socket) -> Self {
        Self {
            socket,
            last_heard: Instant::now(),
            unsent: VecDeque::new(),
            read_position: 0,
            in_flight: HashMap::new(),
            in_flight_bytes: 0,
            open_pages: HashSet::new(),
            uploads: HashMap::new(),
            staging_snapshot: None,
            remaining_uploads: None,
        }
    }

    pub(super) fn has_window_room(&self) -> bool {
        self.in_flight_bytes < WINDOW_BYTES
    }

    pub(super) fn track_sent(&mut self, change_id: Uuid, bytes: usize) {
        self.in_flight.insert(change_id, bytes);
        self.in_flight_bytes += bytes;
    }

    /// Releases the change's share of the window.
    pub(super) fn finish(&mut self, change_id: Uuid) {
        if let Some(bytes) = self.in_flight.remove(&change_id) {
            self.in_flight_bytes -= bytes;
        }
    }

    /// Releases the change's share of the window but keeps it from being sent again on this connection.
    pub(super) fn hold_back(&mut self, change_id: Uuid) {
        self.finish(change_id);
        self.in_flight.insert(change_id, 0);
    }

    pub(super) async fn send(&mut self, message: &ClientMessage) -> Result<(), Interrupt> {
        self.send_frame(protocol::encode(message)).await
    }

    pub(super) async fn send_frame(&mut self, frame: Vec<u8>) -> Result<(), Interrupt> {
        timeout(SEND_TIMEOUT, self.socket.send(Message::binary(frame)))
            .await
            .map_err(|_| Interrupt::Lost {
                reconnect_promptly: false,
            })??;
        Ok(())
    }

    /// The next protocol message, skipping control frames.
    pub(super) async fn receive(&mut self) -> Result<ServerMessage, Interrupt> {
        loop {
            let frame = timeout(SILENCE_LIMIT, self.socket.next())
                .await
                .map_err(|_| Interrupt::Lost {
                    reconnect_promptly: false,
                })?;
            match frame {
                None | Some(Ok(Message::Close(_))) => {
                    return Err(Interrupt::Lost {
                        reconnect_promptly: false,
                    });
                }
                Some(Err(error)) => return Err(error.into()),
                Some(Ok(Message::Binary(bytes))) => return Ok(protocol::decode(&bytes)?),
                Some(Ok(_)) => {}
            }
        }
    }

    /// Best effort: the engine is stopping either way.
    pub(super) async fn close(&mut self) {
        match timeout(CLOSE_TIMEOUT, self.socket.close(None)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(%error, "could not close the sync connection cleanly")
            }
            Err(_) => tracing::debug!("timed out closing the sync connection"),
        }
    }
}
