//! What a server says about itself, and whether it is healthy.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use scroll_api::accounts::{PRODUCT, ServerInfo, ServerInfoResponse, SignInMethod, SignUpMode};

use super::{ApiError, ClientIp};
use crate::state::AppState;

pub async fn server_info(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<ServerInfoResponse>, ApiError> {
    state.info_requests.admit_address(ip)?;
    let google = state.google.is_some().then_some(SignInMethod::Google);
    let info = ServerInfo {
        name: state.settings.server_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: scroll_api::protocol::PROTOCOL_VERSION,
        sign_up_requires_invite: state.settings.sign_up == SignUpMode::Invite,
        sign_up: state.settings.sign_up,
        sign_in_methods: std::iter::once(SignInMethod::Password)
            .chain(google)
            .collect(),
    };
    Ok(Json(ServerInfoResponse {
        product: PRODUCT.to_owned(),
        info,
    }))
}

/// For the proxy or orchestrator in front: 200 when the server can reach its database, else 503.
pub async fn health(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match crate::db::check(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(error) => {
            tracing::error!(%error, "health check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
        }
    }
}
