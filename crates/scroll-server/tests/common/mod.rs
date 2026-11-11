//! A real server on an ephemeral port against `scroll_test`, with HTTP and WebSocket clients to drive it.
//! Every test makes its own users, workspaces and pages, so tests can share the database and run in parallel.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use deadpool_postgres::Pool;
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, Response, StatusCode};
use scroll_api::protocol::CatchUpPart;
use scroll_api::protocol::TransactionRejection;
use scroll_api::protocol::{self, ClientMessage, ConnectionId, PROTOCOL_VERSION, ServerMessage};
use scroll_core::id::{PageId, UserId, WorkspaceId};
use scroll_core::operation::Transaction;
use scroll_core::store::PageCatchUp;
use scroll_core::sync::{WorkspaceCatchUp, WorkspaceSnapshot};
use scroll_server::{AttemptLimits, Limit, Server, Settings, test_support};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{Error as SocketError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
const SILENCE: Duration = Duration::from_millis(300);
pub const PASSWORD: &str = "correct horse battery staple";

pub struct TestServer {
    pub address: SocketAddr,
    pub pool: Pool,
    pub http: Client,
    stop: Option<oneshot::Sender<()>>,
}

pub struct TestUser {
    pub email: String,
    pub user_id: UserId,
    pub token: String,
}

pub fn unique_email() -> String {
    format!("user-{}@example.test", Uuid::now_v7())
}

impl TestServer {
    /// Trusts `x-forwarded-for`, effectively never rate-limits or compacts, and revalidates connections often.
    pub fn test_settings() -> Settings {
        let generous = Limit {
            attempts: 10_000,
            window: Duration::from_secs(60),
        };
        Settings {
            client_ip_header: Some("x-forwarded-for".parse().unwrap()),
            sign_in_limits: AttemptLimits {
                per_address: generous,
                failures_per_address_and_email: generous,
            },
            compaction_threshold: u64::MAX,
            revalidate_interval: Duration::from_millis(200),
            ..Settings::default()
        }
    }

    pub async fn start() -> Self {
        Self::start_with(Self::test_settings()).await
    }

    pub async fn start_with(settings: Settings) -> Self {
        let pool = test_support::test_pool().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let shutdown_signal = async move {
            let _ = stopped.await;
        };
        let server = Server {
            pool: pool.clone(),
            settings,
        };
        tokio::spawn(server.serve(listener, shutdown_signal));
        Self {
            address,
            pool,
            http: Client::new(),
            stop: Some(stop),
        }
    }

    pub fn shut_down(&mut self) {
        self.stop.take().unwrap().send(()).unwrap();
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }

    pub async fn post(&self, path: &str, token: Option<&str>, body: Value) -> Response {
        let mut request = self.http.post(self.url(path)).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    pub async fn get(&self, path: &str, token: &str) -> Response {
        self.http
            .get(self.url(path))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }

    pub async fn signup_invite(&self) -> String {
        scroll_server::create_signup_invite(&self.pool)
            .await
            .unwrap()
    }

    pub async fn sign_up(&self, invite_code: &str) -> TestUser {
        let email = unique_email();
        let response = self
            .post(
                "/accounts",
                None,
                json!({ "email": email, "password": PASSWORD, "invite_code": invite_code }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        self.sign_in(&email).await
    }

    pub async fn sign_in(&self, email: &str) -> TestUser {
        let response = self
            .post(
                "/sessions",
                None,
                json!({ "email": email, "password": PASSWORD }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = response.json().await.unwrap();
        TestUser {
            email: email.to_owned(),
            user_id: serde_json::from_value(body["user_id"].clone()).unwrap(),
            token: body["session_token"].as_str().unwrap().to_owned(),
        }
    }

    pub async fn new_user(&self) -> TestUser {
        let invite_code = self.signup_invite().await;
        self.sign_up(&invite_code).await
    }

    pub async fn create_workspace(&self, owner: &TestUser) -> WorkspaceId {
        let workspace_id = WorkspaceId::generate();
        let response = self
            .post(
                "/workspaces",
                Some(&owner.token),
                json!({ "id": workspace_id, "name": "Test workspace" }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        workspace_id
    }

    pub async fn invite(&self, owner: &TestUser, workspace_id: WorkspaceId, role: &str) -> String {
        let path = format!("/workspaces/{}/invites", workspace_id.0);
        let response = self
            .post(&path, Some(&owner.token), json!({ "role": role }))
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = response.json().await.unwrap();
        body["invite_code"].as_str().unwrap().to_owned()
    }

    pub async fn new_member(
        &self,
        owner: &TestUser,
        workspace_id: WorkspaceId,
        role: &str,
    ) -> TestUser {
        let invite_code = self.invite(owner, workspace_id, role).await;
        self.sign_up(&invite_code).await
    }

    /// Connects and completes the `Hello`/`Welcome` handshake.
    pub async fn connect(&self, user: &TestUser) -> SyncClient {
        let mut client = self.connect_without_hello().await;
        client.hello(PROTOCOL_VERSION, &user.token).await;
        match client.receive().await {
            ServerMessage::Welcome {
                user_id,
                connection_id,
            } if user_id == user.user_id => client.connection_id = Some(connection_id),
            other => panic!("expected a welcome, got {other:?}"),
        }
        client
    }

    pub async fn connect_without_hello(&self) -> SyncClient {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", self.address))
            .await
            .unwrap();
        SyncClient {
            socket,
            connection_id: None,
            unpacked: VecDeque::new(),
        }
    }
}

pub struct SyncClient {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    /// Set once the server has welcomed the connection.
    pub connection_id: Option<ConnectionId>,
    /// The rest of the last `Batch` received.
    unpacked: VecDeque<ServerMessage>,
}

impl SyncClient {
    pub async fn send(&mut self, message: &ClientMessage) {
        self.socket
            .send(Message::binary(protocol::encode(message)))
            .await
            .unwrap();
    }

    pub async fn hello(&mut self, protocol_version: u16, session_token: &str) {
        let hello = ClientMessage::Hello {
            protocol_version,
            session_token: session_token.to_owned(),
        };
        self.send(&hello).await;
    }

    /// The next protocol message, skipping control frames and unpacking batches. Panics on close or timeout.
    pub async fn receive(&mut self) -> ServerMessage {
        if let Some(message) = self.unpacked.pop_front() {
            return message;
        }
        loop {
            let frame = tokio::time::timeout(RECEIVE_TIMEOUT, self.socket.next())
                .await
                .expect("timed out waiting for a server message")
                .expect("connection ended")
                .unwrap();
            match frame {
                Message::Binary(bytes) => match protocol::decode(&bytes).unwrap() {
                    ServerMessage::Batch { messages } => {
                        self.unpacked = messages
                            .iter()
                            .map(|bytes| protocol::decode(bytes).unwrap())
                            .collect();
                        if let Some(message) = self.unpacked.pop_front() {
                            return message;
                        }
                    }
                    message => return message,
                },
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected a binary frame, got {other:?}"),
            }
        }
    }

    /// The next message other than presence. Presence is relayed whenever another connection on an open page moves
    /// its cursor or leaves it, including when the server drops one for falling behind, so it can come before any
    /// reply.
    pub async fn receive_skipping_presence(&mut self) -> ServerMessage {
        loop {
            match self.receive().await {
                ServerMessage::Presence { .. } | ServerMessage::PresenceLeft { .. } => {}
                message => return message,
            }
        }
    }

    /// Waits for the server to close the connection and returns its close frame.
    pub async fn receive_close(&mut self) -> Option<CloseFrame> {
        loop {
            let frame = tokio::time::timeout(RECEIVE_TIMEOUT, self.socket.next())
                .await
                .expect("timed out waiting for the server to close");
            match frame {
                Some(Ok(Message::Close(close_frame))) => return close_frame,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(other)) => panic!("expected a close frame, got {other:?}"),
                Some(Err(error)) => panic!("connection failed instead of closing: {error}"),
                None => return None,
            }
        }
    }

    /// Reads past any messages until the server closes the connection; `None` if it ended without a close frame.
    pub async fn skip_to_close(&mut self) -> Option<CloseFrame> {
        loop {
            let frame = tokio::time::timeout(RECEIVE_TIMEOUT, self.socket.next())
                .await
                .expect("timed out waiting for the server to close");
            match frame {
                Some(Ok(Message::Close(close_frame))) => return close_frame,
                Some(Ok(_)) => {}
                Some(Err(SocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake))) => {
                    return None;
                }
                Some(Err(error)) => panic!("connection failed instead of closing: {error}"),
                None => return None,
            }
        }
    }

    pub async fn assert_silent(&mut self) {
        assert!(
            self.unpacked.is_empty(),
            "expected no message, got {:?}",
            self.unpacked
        );
        if let Ok(frame) = tokio::time::timeout(SILENCE, self.socket.next()).await {
            panic!("expected no message, got {frame:?}");
        }
    }

    pub async fn open_page(
        &mut self,
        workspace_id: WorkspaceId,
        page_id: PageId,
        last_seen_seq: u64,
    ) -> PageCatchUp {
        self.send(&ClientMessage::OpenPage {
            workspace_id,
            page_id,
            last_seen_seq,
        })
        .await;
        match self.receive_skipping_presence().await {
            ServerMessage::PageCatchUp {
                page_id: caught_up,
                catch_up,
            } if caught_up == page_id => catch_up,
            other => panic!("expected a catch-up for the page, got {other:?}"),
        }
    }

    pub async fn send_update(&mut self, update_id: Uuid, page_id: PageId, update: &[u8]) {
        self.send(&ClientMessage::PushPageUpdate {
            update_id,
            page_id,
            update: update.to_vec(),
        })
        .await;
    }

    /// Pushes and waits for the ack, returning the assigned sequence number.
    pub async fn push(&mut self, page_id: PageId, update: &[u8]) -> u64 {
        let update_id = Uuid::now_v7();
        self.send_update(update_id, page_id, update).await;
        self.receive_ack(update_id).await
    }

    pub async fn receive_ack(&mut self, update_id: Uuid) -> u64 {
        match self.receive_skipping_presence().await {
            ServerMessage::PageUpdateAck {
                update_id: acked,
                seq,
                ..
            } if acked == update_id => seq,
            other => panic!("expected an ack for {update_id}, got {other:?}"),
        }
    }

    pub async fn open_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        last_seen_seq: u64,
    ) -> WorkspaceCatchUp {
        self.send(&ClientMessage::OpenWorkspace {
            workspace_id,
            last_seen_seq,
        })
        .await;
        self.receive_workspace_catch_up(workspace_id).await.0
    }

    /// Every part of a workspace catch-up, assembled, with the number of parts it came in.
    pub async fn receive_workspace_catch_up(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> (WorkspaceCatchUp, u32) {
        let mut catch_up = WorkspaceCatchUp {
            snapshot: None,
            transactions: Vec::new(),
        };
        let mut expected_index = 0;
        loop {
            let (index, is_last, part) = match self.receive_skipping_presence().await {
                ServerMessage::WorkspaceCatchUpPart {
                    workspace_id: caught_up,
                    index,
                    is_last,
                    part,
                } if caught_up == workspace_id => (index, is_last, part),
                other => panic!("expected a catch-up part for the workspace, got {other:?}"),
            };
            assert_eq!(index, expected_index, "parts arrive in order");
            assert!(
                protocol::encode(&part).len() < 2 * 1024 * 1024,
                "a part stays near a megabyte"
            );
            match part {
                CatchUpPart::Snapshot(snapshot_part) => {
                    let snapshot = catch_up.snapshot.get_or_insert(WorkspaceSnapshot {
                        seq: snapshot_part.seq,
                        records: Vec::new(),
                        property_values: Vec::new(),
                    });
                    assert_eq!(snapshot.seq, snapshot_part.seq);
                    snapshot.records.extend(snapshot_part.records);
                    snapshot
                        .property_values
                        .extend(snapshot_part.property_values);
                }
                CatchUpPart::Transactions(transactions) => {
                    catch_up.transactions.extend(transactions)
                }
            }
            expected_index += 1;
            if is_last {
                return (catch_up, expected_index);
            }
        }
    }

    pub async fn send_transaction(&mut self, workspace_id: WorkspaceId, transaction: &Transaction) {
        self.send(&ClientMessage::PushTransaction {
            workspace_id,
            transaction: transaction.clone(),
        })
        .await;
    }

    /// Pushes and waits for the ack, returning the assigned workspace sequence number.
    pub async fn commit(&mut self, workspace_id: WorkspaceId, transaction: &Transaction) -> u64 {
        self.send_transaction(workspace_id, transaction).await;
        match self.receive_skipping_presence().await {
            ServerMessage::TransactionAck {
                transaction_id,
                seq,
                ..
            } if transaction_id == transaction.id => seq,
            other => panic!("expected an ack for {:?}, got {other:?}", transaction.id),
        }
    }

    /// Pushes and expects a rejection, returning its reason.
    pub async fn commit_rejected(
        &mut self,
        workspace_id: WorkspaceId,
        transaction: &Transaction,
    ) -> TransactionRejection {
        self.send_transaction(workspace_id, transaction).await;
        match self.receive_skipping_presence().await {
            ServerMessage::TransactionRejected {
                transaction_id,
                reason,
                ..
            } if transaction_id == transaction.id => reason,
            other => panic!(
                "expected a rejection of {:?}, got {other:?}",
                transaction.id
            ),
        }
    }

    pub async fn close(mut self) {
        self.socket.close(None).await.unwrap();
    }
}
