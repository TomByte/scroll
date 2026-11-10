//! What the load and scale tools share beyond `scroll_server::test_support::tools`: signing up against a server
//! given only an invite code, and a bare protocol client.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use scroll_api::accounts::{
    self, AccountCreated, CreateAccountRequest, CreateInviteRequest, CreateWorkspaceRequest,
    InviteCreated, Membership, SignInRequest,
};
use scroll_api::limits::MAX_SERVER_FRAME_BYTES;
use scroll_api::paths;
use scroll_api::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use scroll_core::id::{Role, UserId, WorkspaceId};
use scroll_server::test_support::tools::argument;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

/// A sync connection, with the rest of a `Batch` the last receive opened.
pub struct Socket {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    unpacked: VecDeque<ServerMessage>,
}

pub const PASSWORD: &str = "load test password";

pub fn number_argument(name: &str, default: usize) -> usize {
    argument(name, &default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a whole number"))
}

pub struct Session {
    pub user_id: UserId,
    pub token: String,
}

/// The server's HTTP API, enough to set up users and a workspace.
pub struct Accounts {
    pub url: String,
    http: reqwest::Client,
}

impl Accounts {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> T {
        let mut request = self.http.post(format!("{}{path}", self.url)).json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.expect("the server is reachable");
        let status = response.status();
        let body = response.bytes().await.expect("the server answers");
        assert!(
            status.is_success(),
            "POST {path} failed with {status}: {}",
            String::from_utf8_lossy(&body)
        );
        serde_json::from_slice(&body).expect("the answer has the documented shape")
    }

    /// A new account from the invite, signed in.
    pub async fn sign_up(&self, invite_code: &str) -> Session {
        let email = format!("load-{}@example.test", Uuid::now_v7());
        let account = CreateAccountRequest {
            email: email.clone(),
            password: PASSWORD.to_owned(),
            invite_code: Some(invite_code.to_owned()),
        };
        let _: AccountCreated = self.post(paths::ACCOUNTS, None, &account).await;
        let sign_in = SignInRequest {
            email,
            password: PASSWORD.to_owned(),
        };
        let session: accounts::Session = self.post(paths::SESSIONS, None, &sign_in).await;
        Session {
            user_id: session.user_id,
            token: session.session_token,
        }
    }

    pub async fn create_workspace(&self, owner: &Session, name: &str) -> WorkspaceId {
        let workspace = CreateWorkspaceRequest {
            id: WorkspaceId::generate(),
            name: name.to_owned(),
        };
        let membership: Membership = self
            .post(paths::WORKSPACES, Some(&owner.token), &workspace)
            .await;
        membership.workspace_id
    }

    /// A new member of the workspace, signed in.
    pub async fn add_member(&self, owner: &Session, workspace_id: WorkspaceId) -> Session {
        let path = paths::workspace_invites(workspace_id);
        let invite = CreateInviteRequest { role: Role::Editor };
        let created: InviteCreated = self.post(&path, Some(&owner.token), &invite).await;
        self.sign_up(&created.invite_code).await
    }
}

/// A connection that has said hello.
pub async fn connect(url: &str, session: &Session) -> Socket {
    let socket_url = paths::socket_url(url).expect("the server URL is http:// or https://");
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_SERVER_FRAME_BYTES))
        .max_frame_size(Some(MAX_SERVER_FRAME_BYTES));
    let (stream, _) = tokio_tungstenite::connect_async_with_config(socket_url, Some(config), true)
        .await
        .expect("the server accepts WebSocket connections");
    let mut socket = Socket {
        stream,
        unpacked: VecDeque::new(),
    };
    let hello = ClientMessage::Hello {
        protocol_version: PROTOCOL_VERSION,
        session_token: session.token.clone(),
    };
    send(&mut socket, &hello).await;
    match receive(&mut socket).await {
        Some(ServerMessage::Welcome { .. }) => socket,
        other => panic!("expected a welcome, got {other:?}"),
    }
}

pub async fn send(socket: &mut Socket, message: &ClientMessage) {
    socket
        .stream
        .send(Message::binary(protocol::encode(message)))
        .await
        .expect("the connection stays open");
}

/// The next protocol message, unpacking batches, or `None` once the connection has closed. Safe to cancel: a
/// message is only taken from the socket in the same step that returns it.
pub async fn receive(socket: &mut Socket) -> Option<ServerMessage> {
    if let Some(message) = socket.unpacked.pop_front() {
        return Some(message);
    }
    loop {
        match socket.stream.next().await? {
            Ok(Message::Binary(bytes)) => {
                let message = protocol::decode(&bytes).expect("a valid message");
                let ServerMessage::Batch { messages } = message else {
                    return Some(message);
                };
                socket.unpacked = messages
                    .iter()
                    .map(|bytes| protocol::decode(bytes).expect("a valid batched message"))
                    .collect();
                if let Some(message) = socket.unpacked.pop_front() {
                    return Some(message);
                }
            }
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// Percentiles of a set of durations, in milliseconds.
pub fn percentiles(label: &str, durations: &mut [Duration]) -> String {
    if durations.is_empty() {
        return format!("{label}: none");
    }
    durations.sort_unstable();
    let at = |fraction: f64| {
        let index = ((durations.len() - 1) as f64 * fraction).round() as usize;
        durations[index].as_secs_f64() * 1000.0
    };
    format!(
        "{label}: n={} p50={:.1}ms p90={:.1}ms p99={:.1}ms max={:.1}ms",
        durations.len(),
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0)
    )
}

/// A cheap random number generator; quality does not matter here.
pub struct Random(u64);

impl Random {
    pub fn new() -> Self {
        Self(Uuid::now_v7().as_u64_pair().1 | 1)
    }

    pub fn below(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % limit.max(1) as u64) as usize
    }

    pub fn duration_around(&mut self, mean: Duration) -> Duration {
        mean / 2 + mean.mul_f64(self.below(1000) as f64 / 1000.0)
    }
}

pub fn elapsed_since(start: Instant) -> String {
    format!("{:.1?}", start.elapsed())
}
