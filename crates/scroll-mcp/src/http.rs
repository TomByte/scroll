//! The HTTP side: one `POST /mcp` route, open only to local, non-browser callers holding the token.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::post;

use crate::rpc::{self, Reply};
use crate::server::MCP_PATH;
use crate::token::tokens_match;
use crate::tools::Workspace;

const LOCAL_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];

pub(crate) struct ServerState {
    pub workspace: Workspace,
    pub token: String,
}

/// Other methods on the path are answered 405 by the router.
pub(crate) fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route(MCP_PATH, post(receive))
        .with_state(state)
}

/// Why a request was turned away before it was read.
#[derive(Debug)]
enum Refusal {
    /// A web page may not call the server: an `Origin` header means a browser sent it.
    FromBrowser,
    /// A `Host` other than this machine: a page's own domain resolved to 127.0.0.1 (DNS rebinding).
    UnknownHost,
    WithoutToken,
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        match self {
            Refusal::FromBrowser | Refusal::UnknownHost => StatusCode::FORBIDDEN.into_response(),
            Refusal::WithoutToken => (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
            )
                .into_response(),
        }
    }
}

async fn receive(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = check_caller(&headers, &state.token) {
        log::debug!("refused an MCP request: {refusal:?}");
        return refusal.into_response();
    }
    match rpc::handle(&state.workspace, &body) {
        Reply::Accepted => StatusCode::ACCEPTED.into_response(),
        Reply::Answer(message) => Json(message).into_response(),
        Reply::Unreadable(message) => (StatusCode::BAD_REQUEST, Json(message)).into_response(),
    }
}

fn check_caller(headers: &HeaderMap, token: &str) -> Result<(), Refusal> {
    if headers.contains_key(header::ORIGIN) {
        return Err(Refusal::FromBrowser);
    }
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .ok_or(Refusal::UnknownHost)?;
    if !is_local_host(host) {
        return Err(Refusal::UnknownHost);
    }
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Refusal::WithoutToken)?;
    if !tokens_match(given, token) {
        return Err(Refusal::WithoutToken);
    }
    Ok(())
}

/// "127.0.0.1" or "localhost", with or without a port.
fn is_local_host(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => name,
        _ => host,
    };
    LOCAL_HOSTS
        .iter()
        .any(|local| name.eq_ignore_ascii_case(local))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machine_counts_as_a_local_host() {
        for host in [
            "127.0.0.1:7465",
            "localhost:7465",
            "localhost",
            "LOCALHOST:1",
        ] {
            assert!(is_local_host(host), "{host}");
        }
        for host in [
            "evil.example:7465",
            "127.0.0.1.evil.example",
            "localhost.evil.example:7465",
            "",
        ] {
            assert!(!is_local_host(host), "{host}");
        }
    }
}
