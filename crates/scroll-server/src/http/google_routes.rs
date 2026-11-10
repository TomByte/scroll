//! `/auth/google/start` and `/auth/google/callback`: the browser leg of signing in with Google. The app opens
//! `start` in the system browser with its loopback port and PKCE challenge; Google sends the person back to
//! `callback`, which redirects to the app's loopback address with a one-time code or a refusal, which the app
//! exchanges at `/sessions/exchange`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use reqwest::Url;
use scroll_api::browser_sign_in::{
    BrowserSignInRefusal, GoogleStart, LOOPBACK_CALLBACK_PATH, LoopbackReturn,
};
use serde::Deserialize;

use super::validation::{check_challenge, check_invite_code, check_redirect_port};
use super::{ApiError, ClientIp};
use crate::auth::browser_sign_in::{self, Arrival, PendingSignIn};
use crate::auth::google::{GoogleError, GoogleVerifier};
use crate::auth::identities::{self, IdentitySignIn};
use crate::auth::{AccountError, secrets};
use crate::state::AppState;

pub async fn start(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Query(request): Query<GoogleStart>,
) -> Result<Redirect, ApiError> {
    let google = google_of(&state)?;
    state.attempts.admit_address(ip)?;
    let pending = PendingSignIn {
        challenge: check_challenge(request.challenge)?,
        redirect_port: check_redirect_port(request.redirect_port)?,
        invite_hash: request
            .invite
            .map(check_invite_code)
            .transpose()?
            .map(|code| secrets::hash_secret(&code)),
    };
    let departure = browser_sign_in::depart(&state.pool, &pending).await?;
    let url = google.authorize_url(&departure).map_err(|error| {
        tracing::error!(%error, "could not build the Google sign-in address");
        ApiError::Unavailable
    })?;
    Ok(Redirect::to(url.as_str()))
}

/// Google's answer, as it sends the browser back.
#[derive(Deserialize)]
pub struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    /// Set by Google instead of `code` when the person cancels or Google refuses.
    error: Option<String>,
}

pub async fn callback(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if let Err(refused) = state.attempts.admit_address(ip) {
        return ApiError::from(refused).into_response();
    }
    let Ok(google) = google_of(&state) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let arrival = match query.state.as_deref() {
        Some(sign_in_state) => browser_sign_in::arrive(&state.pool, sign_in_state).await,
        None => Ok(None),
    };
    // Without a known state there is no app to send the person back to.
    let arrival = match arrival {
        Ok(Some(arrival)) => arrival,
        Ok(None) => return expired_page(),
        Err(error) => {
            tracing::error!(%error, "could not look up a Google sign-in");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let port = arrival.pending.redirect_port;
    let returned = match finish(&state, google, (query, arrival)).await {
        Ok(code) => LoopbackReturn::Code(code),
        Err(refusal) => LoopbackReturn::Refused(refusal),
    };
    Redirect::to(loopback_url(port, &returned).as_str()).into_response()
}

/// Turns Google's answer into a one-time code for the app.
async fn finish(
    state: &AppState,
    google: &GoogleVerifier,
    (query, arrival): (CallbackQuery, Arrival),
) -> Result<String, BrowserSignInRefusal> {
    let code = match (query.code, query.error) {
        (Some(code), None) => code,
        _ => return Err(BrowserSignInRefusal::Cancelled),
    };
    let identity = google
        .identity_for_code(&code, &arrival.nonce_hash)
        .await
        .map_err(refusal_for_google)?;
    let sign_in = IdentitySignIn {
        identity: &identity,
        sign_up: state.settings.sign_up,
        invite_hash: arrival.pending.invite_hash,
    };
    let account = identities::sign_in_with_identity(&state.pool, sign_in)
        .await
        .map_err(refusal_for_account)?;
    for token_hash in &account.ended_sessions {
        state.live_sessions.end_by_hash(token_hash);
    }
    browser_sign_in::issue_code(&state.pool, account.user_id, &arrival.pending.challenge)
        .await
        .map_err(|error| {
            tracing::error!(%error, "could not issue a sign-in code");
            BrowserSignInRefusal::Failed
        })
}

fn refusal_for_google(error: GoogleError) -> BrowserSignInRefusal {
    match error {
        GoogleError::EmailNotVerified => BrowserSignInRefusal::EmailNotVerified,
        error => {
            tracing::warn!(%error, "a Google sign-in failed");
            BrowserSignInRefusal::Failed
        }
    }
}

fn refusal_for_account(error: AccountError) -> BrowserSignInRefusal {
    match error {
        AccountError::InviteRequired => BrowserSignInRefusal::InviteRequired,
        AccountError::InvalidInvite => BrowserSignInRefusal::InvalidInvite,
        AccountError::IdentityConflict => BrowserSignInRefusal::AccountConflict,
        error => {
            tracing::error!(%error, "a Google sign-in could not reach an account");
            BrowserSignInRefusal::Failed
        }
    }
}

/// Only ever the loopback address, so a sign-in cannot be steered to another host.
fn loopback_url(port: u16, returned: &LoopbackReturn) -> Url {
    let base = format!("http://127.0.0.1:{port}{LOOPBACK_CALLBACK_PATH}");
    Url::parse_with_params(&base, [returned.query_pair()])
        .expect("a loopback address with a port is a valid URL")
}

fn expired_page() -> Response {
    let page =
        "This sign-in link has expired or was already used. Return to Scroll and sign in again.";
    (StatusCode::BAD_REQUEST, page).into_response()
}

fn google_of(state: &AppState) -> Result<&GoogleVerifier, ApiError> {
    state.google.as_deref().ok_or(ApiError::NotFound)
}
