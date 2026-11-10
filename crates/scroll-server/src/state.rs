//! What every request and connection handler shares.

use std::sync::Arc;

use deadpool_postgres::Pool;
use tokio::sync::watch;

use crate::auth::google::GoogleVerifier;
use crate::auth::live_sessions::LiveSessions;
use crate::auth::rate_limit::Attempts;
use crate::config::Settings;
use crate::pages::Compactor;
use crate::realtime::hub::Hub;

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub hub: Arc<Hub>,
    pub live_sessions: Arc<LiveSessions>,
    pub attempts: Arc<Attempts>,
    /// Requests for `/server-info`, which needs no account; limited per address only.
    pub info_requests: Arc<Attempts>,
    pub compactor: Arc<Compactor>,
    pub google: Option<Arc<GoogleVerifier>>,
    pub settings: Arc<Settings>,
    /// Changes once, when the server starts shutting down.
    pub shutdown: watch::Receiver<()>,
}
