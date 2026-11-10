//! The handle the app holds while the server runs, and the events it drains.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use scroll_core::id::PageId;
use scroll_core::store::{StoreChanges, StoreError, WorkspaceStore};
use tokio::sync::{mpsc, watch};

use crate::http::{self, ServerState};
use crate::tools::Workspace;

/// The port the app serves on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7465;
/// The one endpoint.
pub const MCP_PATH: &str = "/mcp";
/// How long requests still running at shutdown may take before they are dropped, so stopping
/// never waits on a stuck client.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

/// Whether AI tools may change the workspace. Read-only servers do not offer the tools that
/// write, and refuse them if called anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadWrite,
    ReadOnly,
}

pub struct McpConfig {
    pub workspace_path: PathBuf,
    /// 0 picks a free port (for tests).
    pub port: u16,
    /// The secret every request must carry, from `load_or_create_token`.
    pub token: String,
    pub access: Access,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("could not listen on 127.0.0.1:{port}; another program may be using it ({source})")]
    Listen { port: u16, source: io::Error },
    #[error("could not start the MCP server's runtime: {0}")]
    Runtime(io::Error),
    #[error("could not start the MCP server's thread: {0}")]
    Thread(io::Error),
}

/// What a tool changed, for the app to show. Each is already in the workspace file.
#[derive(Debug)]
pub enum McpEvent {
    /// Records changed through `apply_local_transaction`: re-read them, and tell the sync engine.
    RecordsChanged(StoreChanges),
    /// An update to the page's content, already stored: import it into the page's live document,
    /// if it is open, and tell the sync engine.
    PageContentChanged { page_id: PageId, update: Vec<u8> },
}

/// Serves one workspace file until shut down or dropped.
pub struct McpServer {
    address: SocketAddr,
    stop: watch::Sender<bool>,
    thread: Option<thread::JoinHandle<()>>,
}

/// The server's events, in the order its writes happened. Drain them from the app's executor.
pub struct McpEvents {
    receiver: mpsc::UnboundedReceiver<McpEvent>,
}

impl McpServer {
    /// Opens a connection to the workspace file and starts listening on 127.0.0.1. Fails at once,
    /// with nothing left running, if the port cannot be had.
    pub fn start(config: McpConfig) -> Result<(McpServer, McpEvents), StartError> {
        let store = WorkspaceStore::open(&config.workspace_path)?;
        let listen_error = |source| StartError::Listen {
            port: config.port,
            source,
        };
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
            .map_err(listen_error)?;
        listener.set_nonblocking(true).map_err(listen_error)?;
        let address = listener.local_addr().map_err(listen_error)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(StartError::Runtime)?;
        let listener = {
            let _runtime = runtime.enter();
            tokio::net::TcpListener::from_std(listener).map_err(listen_error)?
        };
        let (events, receiver) = mpsc::unbounded_channel();
        let state = Arc::new(ServerState {
            workspace: Workspace::new(store, config.access, events),
            token: config.token,
        });
        let (stop, stopping) = watch::channel(false);
        let thread = thread::Builder::new()
            .name("scroll-mcp".into())
            .spawn(move || runtime.block_on(serve(listener, state, stopping)))
            .map_err(StartError::Thread)?;
        let server = McpServer {
            address,
            stop,
            thread: Some(thread),
        };
        Ok((server, McpEvents { receiver }))
    }

    /// Where it listens; the port is the one asked for, or the one picked for port 0.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stops listening and waits, at most about a second, for requests still running.
    pub fn shut_down(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.stop.send_replace(true);
        if thread.join().is_err() {
            log::error!("the MCP server's thread panicked");
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl McpEvents {
    /// Call in a loop on the app's executor. `None` once the server has stopped and every event
    /// is drained.
    pub async fn next(&mut self) -> Option<McpEvent> {
        self.receiver.recv().await
    }
}

async fn serve(
    listener: tokio::net::TcpListener,
    state: Arc<ServerState>,
    stopping: watch::Receiver<bool>,
) {
    let mut asked_to_stop = stopping.clone();
    let server = axum::serve(listener, http::router(state)).with_graceful_shutdown(async move {
        if asked_to_stop.wait_for(|stop| *stop).await.is_err() {
            log::debug!("the MCP server's handle went away; stopping");
        }
    });
    let mut asked_to_stop = stopping;
    let grace_over = async move {
        if asked_to_stop.wait_for(|stop| *stop).await.is_ok() {
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        }
    };
    tokio::select! {
        served = server => {
            if let Err(error) = served {
                log::error!("the MCP server stopped: {error}");
            }
        }
        () = grace_over => log::warn!("the MCP server stopped with requests still running"),
    }
}
