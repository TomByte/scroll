//! The server's settings and their defaults, and reading them from the environment: every variable is read once,
//! here. A variable set to an empty string counts as unset: Compose passes unset optional settings through that
//! way.

use std::env;

use std::time::Duration;

use axum::http::HeaderName;
use scroll_api::accounts::SignUpMode;

use crate::auth::google::{GoogleEndpoints, GoogleSignIn};
use crate::auth::rate_limit::AttemptLimits;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
pub const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_SERVER_NAME: &str = "Scroll";
const DEFAULT_SIGN_UP: SignUpMode = SignUpMode::Invite;
const DEFAULT_COMPACTION_THRESHOLD: u64 = 1000;

pub struct Settings {
    /// The header in which the reverse proxy passes the client's address, such as `x-forwarded-for`. The last
    /// address in it is used, because that is the one the proxy itself added. `None` uses the TCP peer address,
    /// for running without a proxy.
    pub client_ip_header: Option<HeaderName>,
    pub sign_in_limits: AttemptLimits,
    /// A page is compacted once a catch-up from zero would replay this many updates.
    pub compaction_threshold: u64,
    /// How often each sync connection checks that its session and memberships still hold.
    pub revalidate_interval: Duration,
    /// The longest a connection goes without receiving row changes of databases it does not have open (see
    /// `feed`).
    pub deferred_delivery: Duration,
    /// Shown by apps when someone adds this server.
    pub server_name: String,
    pub sign_up: SignUpMode,
    /// `None` leaves sign-in with Google off.
    pub google: Option<GoogleSignIn>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            client_ip_header: None,
            sign_in_limits: AttemptLimits::default(),
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
            revalidate_interval: Duration::from_secs(60),
            deferred_delivery: Duration::from_secs(1),
            server_name: DEFAULT_SERVER_NAME.to_owned(),
            sign_up: DEFAULT_SIGN_UP,
            google: None,
        }
    }
}

pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub settings: Settings,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("the DATABASE_URL environment variable is not set")]
    MissingDatabaseUrl,
    #[error("CLIENT_IP_HEADER is not a valid header name")]
    InvalidClientIpHeader,
    #[error("PAGE_COMPACTION_THRESHOLD is not a positive whole number")]
    InvalidCompactionThreshold,
    #[error("SIGN_UP must be `invite` or `open`")]
    InvalidSignUp,
    #[error(
        "Google sign-in needs GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET and PUBLIC_URL all set (or none); missing: {0}"
    )]
    IncompleteGoogleSettings(String),
    #[error("PUBLIC_URL must be the server's https:// address (http:// only for localhost)")]
    InvalidPublicUrl,
}

pub fn from_env() -> Result<Config, ConfigError> {
    let mut settings = Settings::default();
    if let Some(header) = setting("CLIENT_IP_HEADER") {
        let header =
            HeaderName::try_from(header).map_err(|_| ConfigError::InvalidClientIpHeader)?;
        settings.client_ip_header = Some(header);
    }
    if let Some(name) = setting("SERVER_NAME") {
        settings.server_name = name;
    }
    if let Some(threshold) = setting("PAGE_COMPACTION_THRESHOLD") {
        settings.compaction_threshold = threshold
            .parse()
            .ok()
            .filter(|threshold| *threshold > 0)
            .ok_or(ConfigError::InvalidCompactionThreshold)?;
    }
    if let Some(sign_up) = setting("SIGN_UP") {
        settings.sign_up = match sign_up.as_str() {
            "invite" => SignUpMode::Invite,
            "open" => SignUpMode::Open,
            _ => return Err(ConfigError::InvalidSignUp),
        };
    }
    settings.google = google()?;
    Ok(Config {
        database_url: setting("DATABASE_URL").ok_or(ConfigError::MissingDatabaseUrl)?,
        bind_addr: setting("BIND_ADDR").unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned()),
        settings,
    })
}

/// `RUST_LOG`, read before anything else so that everything after can log.
pub fn log_filter() -> String {
    setting("RUST_LOG").unwrap_or_else(|| DEFAULT_LOG_FILTER.to_owned())
}

/// Only `DATABASE_URL` is needed to create an invite.
pub fn database_url() -> Result<String, ConfigError> {
    setting("DATABASE_URL").ok_or(ConfigError::MissingDatabaseUrl)
}

fn setting(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// All three settings, or none (Google sign-in off). A partial set is a mistake worth stopping for.
fn google() -> Result<Option<GoogleSignIn>, ConfigError> {
    let names = ["GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET", "PUBLIC_URL"];
    let values = names.map(setting);
    let missing: Vec<&str> = names
        .iter()
        .zip(&values)
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| *name)
        .collect();
    if missing.len() == names.len() {
        return Ok(None);
    }
    let [Some(client_id), Some(client_secret), Some(public_url)] = values else {
        return Err(ConfigError::IncompleteGoogleSettings(missing.join(", ")));
    };
    Ok(Some(GoogleSignIn {
        client_id,
        client_secret,
        public_url: check_public_url(public_url)?,
        endpoints: GoogleEndpoints::default(),
    }))
}

/// Google only sends people back to https addresses, except for local testing.
fn check_public_url(url: String) -> Result<String, ConfigError> {
    let is_local = ["http://localhost", "http://127.0.0.1"]
        .iter()
        .any(|prefix| url.starts_with(prefix));
    if !url.starts_with("https://") && !is_local {
        return Err(ConfigError::InvalidPublicUrl);
    }
    Ok(url.trim_end_matches('/').to_owned())
}
