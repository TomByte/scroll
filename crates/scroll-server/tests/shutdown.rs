//! The real binary stops promptly on SIGTERM, even with sync clients connected: idle, and mid-upload.

mod common;

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::{PASSWORD, TestServer};
use futures_util::{SinkExt, StreamExt};
use scroll_api::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use scroll_core::id::{PageId, WorkspaceId};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

/// The server's promise is to close every socket and exit within about five seconds.
const EXIT_DEADLINE: Duration = Duration::from_secs(7);
const CLOSE_RESTART: u16 = 1012;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Kills the process if the test ends early, so a failed run leaves nothing behind.
struct ServerProcess(Child);

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

async fn start_binary() -> (ServerProcess, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let database_url = scroll_server::test_support::test_database_url();
    let child = Command::new(env!("CARGO_BIN_EXE_scroll-server"))
        .env("DATABASE_URL", database_url)
        .env("BIND_ADDR", format!("127.0.0.1:{port}"))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let process = ServerProcess(child);
    let url = format!("http://127.0.0.1:{port}");
    let http = reqwest::Client::new();
    for _ in 0..100 {
        if http.get(format!("{url}/healthz")).send().await.is_ok() {
            return (process, url);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the server did not start");
}

/// A signed-in user with a workspace, on the server at `url`.
async fn user_with_workspace(helper: &TestServer, url: &str) -> (String, WorkspaceId) {
    let invite = scroll_server::create_signup_invite(&helper.pool)
        .await
        .unwrap();
    let http = reqwest::Client::new();
    let email = format!("shutdown-{}@example.test", Uuid::now_v7());
    let account = json!({ "email": email, "password": PASSWORD, "invite_code": invite });
    http.post(format!("{url}/accounts"))
        .json(&account)
        .send()
        .await
        .unwrap();
    let session: Value = http
        .post(format!("{url}/sessions"))
        .json(&json!({ "email": email, "password": PASSWORD }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = session["session_token"].as_str().unwrap().to_owned();
    let workspace_id = WorkspaceId::generate();
    http.post(format!("{url}/workspaces"))
        .bearer_auth(&token)
        .json(&json!({ "id": workspace_id, "name": "Shutdown" }))
        .send()
        .await
        .unwrap();
    (token, workspace_id)
}

async fn connect(url: &str, token: &str) -> Socket {
    let socket_url = format!("ws://{}/ws", url.trim_start_matches("http://"));
    let (mut socket, _) = tokio_tungstenite::connect_async(socket_url).await.unwrap();
    let hello = ClientMessage::Hello {
        protocol_version: PROTOCOL_VERSION,
        session_token: token.to_owned(),
    };
    socket
        .send(Message::binary(protocol::encode(&hello)))
        .await
        .unwrap();
    loop {
        if let Some(Ok(Message::Binary(bytes))) = socket.next().await
            && let Ok(ServerMessage::Welcome { .. }) = protocol::decode(&bytes)
        {
            return socket;
        }
    }
}

/// The close code the server sent, or `None` if the connection ended without a close frame.
async fn close_code(
    mut socket: impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
) -> Option<u16> {
    while let Some(frame) = socket.next().await {
        match frame {
            Ok(Message::Close(close)) => return close.map(|close| u16::from(close.code)),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_closes_connected_clients_and_exits_promptly() {
    // The in-process server only lends its database pool, to make an invite.
    let helper = TestServer::start().await;
    let (mut process, url) = start_binary().await;
    let (token, workspace_id) = user_with_workspace(&helper, &url).await;
    let idle = connect(&url, &token).await;
    let mut uploader = connect(&url, &token).await;
    let page_id = PageId::generate();
    let open = ClientMessage::OpenPage {
        workspace_id,
        page_id,
        last_seen_seq: 0,
    };
    // Connected but never says hello.
    let socket_url = format!("ws://{}/ws", url.trim_start_matches("http://"));
    let (_silent, _) = tokio_tungstenite::connect_async(&socket_url).await.unwrap();
    // Receives every update on the busy page but never reads, so the server's writes to it block.
    let mut deaf = connect(&url, &token).await;
    deaf.send(Message::binary(protocol::encode(&open)))
        .await
        .unwrap();
    uploader
        .send(Message::binary(protocol::encode(&open)))
        .await
        .unwrap();
    let (mut writer, reader) = uploader.split();
    let uploading = tokio::spawn(async move {
        let mut sent = 0;
        loop {
            let push = ClientMessage::PushPageUpdate {
                update_id: Uuid::now_v7(),
                page_id,
                update: vec![0u8; 4_096],
            };
            if writer
                .send(Message::binary(protocol::encode(&push)))
                .await
                .is_err()
            {
                return sent;
            }
            sent += 1;
        }
    });
    let uploader_closed = tokio::spawn(close_code(reader));
    let idle_closed = tokio::spawn(close_code(idle));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let signalled = Instant::now();
    let killed = Command::new("kill")
        .args(["-TERM", &process.0.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let exit = loop {
        if let Some(exit) = process.0.try_wait().unwrap() {
            break exit;
        }
        assert!(
            signalled.elapsed() < EXIT_DEADLINE,
            "the server was still running {EXIT_DEADLINE:?} after SIGTERM"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    println!("exited {:.1?} after SIGTERM: {exit}", signalled.elapsed());
    assert!(exit.success());
    assert_eq!(idle_closed.await.unwrap(), Some(CLOSE_RESTART));
    assert!(uploading.await.unwrap() > 0);
    drop(deaf);
    let uploader_close = uploader_closed.await.unwrap();
    assert!(
        uploader_close.is_none() || uploader_close == Some(CLOSE_RESTART),
        "{uploader_close:?}"
    );
}
