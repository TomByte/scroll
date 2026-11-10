//! Finding out whether an address is a Scroll server this app can talk to, before anyone signs in.

use std::net::IpAddr;
use std::time::Duration;

use scroll_api::accounts::{PRODUCT, ServerInfo, ServerInfoResponse};
use scroll_api::paths;
use scroll_api::protocol::PROTOCOL_VERSION;
use tokio::task::JoinError;

use crate::accounts::{AccountClient, AccountError};
use crate::transport::{is_rate_limited, is_tls_failure};

/// Why a URL cannot be used as a Scroll server.
#[derive(Debug, thiserror::Error)]
pub enum ServerCheckError {
    #[error("the server could not be reached: {0}")]
    Unreachable(reqwest::Error),
    #[error("the server's certificate is not trusted, or the secure connection failed: {0}")]
    ServerNotTrusted(reqwest::Error),
    #[error("something answered at this address, but it is not a Scroll server")]
    NotAScrollServer,
    #[error(
        "the server speaks protocol version {server}, this app version {app}; one of them needs updating"
    )]
    IncompatibleProtocol { server: u16, app: u16 },
    #[error("the server is busy; try again in {retry_after:?}")]
    RateLimited { retry_after: Duration },
    #[error("the request task stopped: {0}")]
    Task(#[from] JoinError),
}

impl AccountClient {
    /// Call when someone adds a server, before showing the sign-in form, to check the address.
    pub async fn server_info(&self) -> Result<ServerInfo, ServerCheckError> {
        let response = match self.transport().fetch(paths::SERVER_INFO).await? {
            Ok(response) => response,
            Err(error) if is_tls_failure(&error) => {
                return Err(ServerCheckError::ServerNotTrusted(error));
            }
            Err(error) => return Err(ServerCheckError::Unreachable(error)),
        };
        if is_rate_limited(&response) {
            return Err(ServerCheckError::RateLimited {
                retry_after: response.retry_after,
            });
        }
        let answer = serde_json::from_slice::<ServerInfoResponse>(&response.body)
            .ok()
            .filter(|answer| response.status.is_success() && answer.product == PRODUCT)
            .ok_or(ServerCheckError::NotAScrollServer)?;
        if answer.info.protocol_version != PROTOCOL_VERSION {
            return Err(ServerCheckError::IncompatibleProtocol {
                server: answer.info.protocol_version,
                app: PROTOCOL_VERSION,
            });
        }
        Ok(answer.info)
    }
}

/// Turns what a person typed into the server address to use: `scroll.example.com`, `scroll.example.com/` and
/// `https://scroll.example.com` all become `https://scroll.example.com`. A bare host gets `https://`, except a
/// loopback one (`localhost`, `127.0.0.1`, `[::1]`), which gets `http://` for running a server locally; plain
/// `http://` elsewhere is accepted only when typed explicitly. A path is kept, minus trailing slashes, for a
/// server behind a proxy under a sub-path. Credentials, queries and fragments are refused.
pub fn normalize_server_url(input: &str) -> Result<String, AccountError> {
    let input = input.trim();
    let has_scheme = input.contains("://");
    let with_scheme = if has_scheme {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut url = reqwest::Url::parse(&with_scheme).map_err(|_| AccountError::InvalidServerUrl)?;
    let is_usable = matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    if !is_usable {
        return Err(AccountError::InvalidServerUrl);
    }
    if !has_scheme && is_loopback(&url) {
        url.set_scheme("http")
            .map_err(|()| AccountError::InvalidServerUrl)?;
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_become_server_urls() {
        let normalized = |input| normalize_server_url(input).ok();
        let cases = [
            ("scroll.example.com", "https://scroll.example.com"),
            ("  Scroll.Example.com/ ", "https://scroll.example.com"),
            (
                "https://scroll.example.com///",
                "https://scroll.example.com",
            ),
            (
                "scroll.example.com:8443/team/",
                "https://scroll.example.com:8443/team",
            ),
            ("http://scroll.lan", "http://scroll.lan"),
            ("localhost:8080", "http://localhost:8080"),
            ("127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("[::1]:8080", "http://[::1]:8080"),
            ("https://localhost", "https://localhost"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalized(input).as_deref(), Some(expected), "{input}");
        }
        for refused in [
            "",
            "ftp://scroll.example.com",
            "https://user:secret@scroll.example.com",
            "https://scroll.example.com/?x=1",
            "https://scroll.example.com/#top",
            "https://",
        ] {
            assert_eq!(normalized(refused), None, "{refused}");
        }
    }
}
