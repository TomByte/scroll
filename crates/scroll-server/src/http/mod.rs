//! The JSON API: errors as responses, and who and where a request comes from. The routes live by feature in
//! the submodules; the shapes they take and return come from `scroll_api`.

pub mod account_routes;
pub mod google_routes;
pub mod info;
pub mod validation;
pub mod workspace_routes;

use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use scroll_api::errors::{ErrorBody, ErrorCode};
use scroll_core::id::UserId;

use crate::auth::browser_sign_in::CodeError;
use crate::auth::rate_limit::TooManyAttempts;
use crate::auth::{AccountError, sessions};
use crate::db::DbError;
use crate::state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("invalid {field}")]
    InvalidInput { field: &'static str },
    #[error("missing or expired session")]
    Unauthenticated,
    #[error("not allowed")]
    Forbidden,
    #[error("not offered by this server")]
    NotFound,
    #[error("a service the request needs is unavailable")]
    Unavailable,
    #[error("the sign-in code is unknown, used, expired or for another verifier")]
    InvalidSignInCode,
    #[error(transparent)]
    TooManyAttempts(#[from] TooManyAttempts),
    #[error("the client address is unknown")]
    UnknownClientAddress,
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error(transparent)]
    Database(#[from] DbError),
}

impl From<CodeError> for ApiError {
    fn from(error: CodeError) -> Self {
        match error {
            CodeError::Invalid => Self::InvalidSignInCode,
            CodeError::Database(error) => error.into(),
        }
    }
}

impl ApiError {
    fn status_and_code(&self) -> (StatusCode, ErrorCode) {
        match self {
            ApiError::InvalidInput { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::InvalidInput)
            }
            ApiError::TooManyAttempts(_) => (StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimited),
            ApiError::Unauthenticated => (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
            ApiError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            ApiError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable),
            ApiError::InvalidSignInCode => (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCode),
            ApiError::Account(AccountError::InvalidInvite) => {
                (StatusCode::BAD_REQUEST, ErrorCode::InvalidInvite)
            }
            ApiError::Account(AccountError::InviteRequired) => {
                (StatusCode::BAD_REQUEST, ErrorCode::InviteRequired)
            }
            ApiError::Account(AccountError::IdentityConflict) => {
                (StatusCode::CONFLICT, ErrorCode::AccountConflict)
            }
            ApiError::Account(AccountError::EmailTaken) => {
                (StatusCode::CONFLICT, ErrorCode::EmailTaken)
            }
            ApiError::Account(AccountError::InvalidCredentials) => {
                (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials)
            }
            ApiError::Account(AccountError::WorkspaceExists) => {
                (StatusCode::CONFLICT, ErrorCode::WorkspaceExists)
            }
            ApiError::UnknownClientAddress
            | ApiError::Database(_)
            | ApiError::Account(AccountError::Database(_) | AccountError::Password(_)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal)
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = self.status_and_code();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "a request failed");
        }
        let field = match &self {
            ApiError::InvalidInput { field } => Some((*field).to_owned()),
            _ => None,
        };
        let body = Json(ErrorBody { error, field });
        match self {
            ApiError::TooManyAttempts(refused) => {
                let retry_after_seconds = refused.retry_after.as_secs().max(1).to_string();
                (status, [(header::RETRY_AFTER, retry_after_seconds)], body).into_response()
            }
            _ => (status, body).into_response(),
        }
    }
}

/// The user behind the request's `Authorization: Bearer <session token>` header.
pub struct SignedIn {
    pub user_id: UserId,
    pub token: String,
}

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthenticated)?
            .to_owned();
        let user_id = sessions::session_user(&state.pool, &token)
            .await?
            .ok_or(ApiError::Unauthenticated)?;
        Ok(SignedIn { user_id, token })
    }
}

/// The address the request came from: the last entry of the configured proxy header, else the TCP peer.
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let forwarded = state
            .settings
            .client_ip_header
            .as_ref()
            .and_then(|name| last_forwarded_address(&parts.headers, name));
        if let Some(ip) = forwarded {
            return Ok(ClientIp(ip));
        }
        let ConnectInfo(peer) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or(ApiError::UnknownClientAddress)?;
        Ok(ClientIp(peer.ip()))
    }
}

/// Earlier entries are whatever the client sent; only the one the proxy appended can be trusted.
fn last_forwarded_address(headers: &HeaderMap, name: &HeaderName) -> Option<IpAddr> {
    let last_header = headers.get_all(name).iter().next_back()?;
    let last_entry = last_header.to_str().ok()?.rsplit(',').next()?;
    last_entry.trim().parse().ok()
}
