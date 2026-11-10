//! The Scroll server: accounts, workspaces and sign-in over JSON, and live sync of page content and records over
//! a WebSocket. The wire shapes are `scroll_api`'s.

mod auth;
pub mod config;
mod db;
mod http;
mod pages;
mod realtime;
mod records;
mod state;
#[cfg(feature = "test-support")]
pub mod test_support;
mod workspaces;

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::serve::ListenerExt;
use deadpool_postgres::Pool;
use scroll_api::paths;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::auth::google::GoogleVerifier;
use crate::auth::rate_limit::Attempts;
use crate::http::{account_routes, google_routes, info, workspace_routes};
use crate::pages::Compactor;
use crate::state::AppState;

pub use crate::auth::google::{GoogleEndpoints, GoogleSignIn};
pub use crate::auth::rate_limit::{AttemptLimits, Limit};
pub use crate::config::Settings;
pub use crate::db::{ConnectError, DbError, connect, migrate};
pub use scroll_api::accounts::SignUpMode;

/// Account requests are small; this keeps a client from making the server buffer a large body.
const MAX_BODY_BYTES: usize = 16 * 1024;
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// Generous: an app checks a server once when it is added, and on launch.
const INFO_REQUESTS_PER_ADDRESS: Limit = Limit {
    attempts: 120,
    window: Duration::from_secs(60),
};

pub struct Server {
    pub pool: Pool,
    pub settings: Settings,
}

impl Server {
    /// Serves until `shutdown_signal` completes, then closes sync connections and waits briefly for them to finish.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown_signal: impl Future<Output = ()> + Send + 'static,
    ) -> io::Result<()> {
        let (stop_connections, shutdown) = watch::channel(());
        let google = self
            .settings
            .google
            .clone()
            .map(GoogleVerifier::new)
            .transpose()
            .map_err(io::Error::other)?
            .map(Arc::new);
        let state = AppState {
            hub: Arc::default(),
            live_sessions: Arc::default(),
            attempts: Arc::new(Attempts::new(self.settings.sign_in_limits)),
            info_requests: Arc::new(Attempts::per_address_only(INFO_REQUESTS_PER_ADDRESS)),
            compactor: Arc::new(Compactor::new(
                self.pool.clone(),
                self.settings.compaction_threshold,
            )),
            google,
            settings: Arc::new(self.settings),
            pool: self.pool,
            shutdown,
        };
        let stop = stop_connections.clone();
        let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
        // Without this, the last part of a large message can wait for the client's delayed acknowledgement of the
        // part before: a tenth of a second or more.
        let listener = listener.tap_io(|stream| {
            if let Err(error) = stream.set_nodelay(true) {
                tracing::debug!(%error, "could not turn off Nagle's algorithm for a connection");
            }
        });
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal.await;
                stop.send_replace(());
            })
            .await?;
        // axum's graceful shutdown does not track upgraded WebSocket connections; each holds a receiver until it has
        // closed, so the channel closing means they are all done.
        if tokio::time::timeout(CONNECTION_DRAIN_TIMEOUT, stop_connections.closed())
            .await
            .is_err()
        {
            tracing::warn!("sync connections were still closing when the server stopped");
        }
        Ok(())
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route(paths::HEALTH, get(info::health))
        .route(paths::SERVER_INFO, get(info::server_info))
        .route(paths::ACCOUNTS, post(account_routes::create_account))
        .route(paths::SESSIONS, post(account_routes::sign_in))
        .route(paths::CURRENT_SESSION, delete(account_routes::sign_out))
        .route(
            paths::SESSION_EXCHANGE,
            post(account_routes::exchange_sign_in_code),
        )
        .route(paths::GOOGLE_START, get(google_routes::start))
        .route(paths::GOOGLE_CALLBACK, get(google_routes::callback))
        .route(
            paths::WORKSPACES,
            get(workspace_routes::list_workspaces).post(workspace_routes::create_workspace),
        )
        .route(
            paths::WORKSPACE_INVITES_ROUTE,
            post(workspace_routes::create_invite),
        )
        .route(paths::ACCEPT_INVITE, post(workspace_routes::accept_invite))
        .route(paths::SYNC_SOCKET, get(realtime::upgrade))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// A new sign-up invite, which is how the first user gets an account. Only its hash is stored; the code is
/// returned for the caller to show once.
pub async fn create_signup_invite(pool: &Pool) -> Result<String, DbError> {
    let invite = workspaces::invites::NewInvite {
        grant: None,
        created_by: None,
    };
    workspaces::invites::create_invite(pool, &invite).await
}

/// The data behind the server's locks stays consistent even if a holder panicked, so a poisoned lock is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
