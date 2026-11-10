//! Accounts and sessions: sign up, sign in with a password, finish a browser sign-in, sign out.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use scroll_api::accounts::{
    AccountCreated, CreateAccountRequest, ExchangeRequest, Session, SignInRequest,
};

use super::validation::{
    check_invite_code, check_new_password, check_password_size, check_sign_in_code, check_verifier,
    normalize_email,
};
use super::{ApiError, ClientIp, SignedIn};
use crate::auth::sessions::{self, NewSession};
use crate::auth::users::{self, Credentials, NewAccount};
use crate::auth::{AccountError, browser_sign_in};
use crate::state::AppState;

pub async fn create_account(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(request): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountCreated>), ApiError> {
    let account = NewAccount {
        email: normalize_email(&request.email)?,
        password: check_new_password(request.password)?,
        invite_code: request.invite_code.map(check_invite_code).transpose()?,
    };
    state.attempts.admit(ip, &account.email)?;
    let user_id = users::create_account(&state.pool, account, state.settings.sign_up).await?;
    Ok((StatusCode::CREATED, Json(AccountCreated { user_id })))
}

pub async fn sign_in(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(request): Json<SignInRequest>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    let credentials = Credentials {
        email: normalize_email(&request.email)?,
        password: check_password_size(request.password)?,
    };
    let email = credentials.email.clone();
    state.attempts.admit(ip, &email)?;
    let user_id = match users::authenticate(&state.pool, credentials).await {
        Err(AccountError::InvalidCredentials) => {
            state.attempts.record_failure(ip, &email);
            return Err(AccountError::InvalidCredentials.into());
        }
        result => result?,
    };
    let session = sessions::create_session(&state.pool, user_id).await?;
    Ok((StatusCode::CREATED, Json(session_body(session))))
}

/// The last step of a browser sign-in (see `auth::browser_sign_in`): the app trades the one-time code the
/// browser brought back, with the verifier only it knows, for a session.
pub async fn exchange_sign_in_code(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(request): Json<ExchangeRequest>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    state.attempts.admit_address(ip)?;
    check_sign_in_code(&request.code)?;
    check_verifier(&request.verifier)?;
    let user_id =
        browser_sign_in::redeem_code(&state.pool, &request.code, &request.verifier).await?;
    let session = sessions::create_session(&state.pool, user_id).await?;
    Ok((StatusCode::CREATED, Json(session_body(session))))
}

pub async fn sign_out(
    State(state): State<AppState>,
    signed_in: SignedIn,
) -> Result<StatusCode, ApiError> {
    sessions::end_session(&state.pool, &signed_in.token).await?;
    state.live_sessions.end(&signed_in.token);
    Ok(StatusCode::NO_CONTENT)
}

fn session_body(session: NewSession) -> Session {
    Session {
        user_id: session.user_id,
        session_token: session.token,
        email: session.email,
        display_name: session.display_name,
    }
}
