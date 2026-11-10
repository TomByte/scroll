//! A sign-in that goes through the system browser and back to the app (RFC 8252): the state kept while the
//! person is at the provider, and the one-time code the app exchanges for a session.
//!
//! The app makes a random verifier and sends only its SHA-256 (the challenge). The code goes back to the app
//! through the browser, where other software could see it; only the app, which holds the verifier, can
//! exchange it. Every secret here is single use, short-lived and stored hashed.

use deadpool_postgres::Pool;
use scroll_api::browser_sign_in::challenge_of;
use scroll_core::id::UserId;

use super::secrets;
use crate::db::DbError;

/// Long enough to choose an account and consent at the provider.
const STATE_LIFETIME_SECONDS: i32 = 600;
/// The app exchanges the code as soon as the browser hands it over.
const CODE_LIFETIME_SECONDS: i32 = 60;

/// What the app asked for when it sent the person to the provider.
pub struct PendingSignIn {
    /// Base64url SHA-256 of the app's verifier.
    pub challenge: String,
    /// The app's loopback port, where the browser is sent back to.
    pub redirect_port: u16,
    pub invite_hash: Option<Vec<u8>>,
}

/// Secrets for one trip to the provider: `state` comes back with the person, `nonce` inside the ID token.
pub struct Departure {
    pub state: String,
    pub nonce: String,
}

/// A pending sign-in found by its state, which can no longer be used again.
pub struct Arrival {
    pub pending: PendingSignIn,
    pub nonce_hash: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("the sign-in code is unknown, used, expired, or for another verifier")]
    Invalid,
    #[error(transparent)]
    Database(#[from] DbError),
}

impl From<tokio_postgres::Error> for CodeError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(error.into())
    }
}

impl From<deadpool_postgres::PoolError> for CodeError {
    fn from(error: deadpool_postgres::PoolError) -> Self {
        Self::Database(error.into())
    }
}

/// Records the sign-in and returns the secrets to send the person off with. Clears out abandoned ones too.
pub async fn depart(pool: &Pool, pending: &PendingSignIn) -> Result<Departure, DbError> {
    let departure = Departure {
        state: secrets::generate_secret(),
        nonce: secrets::generate_secret(),
    };
    let client = pool.get().await?;
    client
        .execute("DELETE FROM browser_sign_ins WHERE expires_at < now()", &[])
        .await?;
    client
        .execute(
            "INSERT INTO browser_sign_ins (state_hash, nonce_hash, challenge, redirect_port, invite_hash, expires_at)
             VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
            &[
                &secrets::hash_secret(&departure.state),
                &secrets::hash_secret(&departure.nonce),
                &pending.challenge,
                &i32::from(pending.redirect_port),
                &pending.invite_hash,
                &f64::from(STATE_LIFETIME_SECONDS),
            ],
        )
        .await?;
    Ok(departure)
}

/// The sign-in this state belongs to, if it is known and fresh. It is removed either way, so a state works once.
pub async fn arrive(pool: &Pool, state: &str) -> Result<Option<Arrival>, DbError> {
    let client = pool.get().await?;
    let row = client
        .query_opt(
            "DELETE FROM browser_sign_ins WHERE state_hash = $1
             RETURNING nonce_hash, challenge, redirect_port, invite_hash, expires_at > now() AS is_fresh",
            &[&secrets::hash_secret(state)],
        )
        .await?;
    let Some(row) = row.filter(|row| row.get::<_, bool>("is_fresh")) else {
        return Ok(None);
    };
    let port: i32 = row.get("redirect_port");
    let Ok(redirect_port) = u16::try_from(port) else {
        return Ok(None);
    };
    Ok(Some(Arrival {
        pending: PendingSignIn {
            challenge: row.get("challenge"),
            redirect_port,
            invite_hash: row.get("invite_hash"),
        },
        nonce_hash: row.get("nonce_hash"),
    }))
}

/// A one-time code for the app to exchange, bound to its challenge.
pub async fn issue_code(pool: &Pool, user_id: UserId, challenge: &str) -> Result<String, DbError> {
    let code = secrets::generate_secret();
    let client = pool.get().await?;
    client
        .execute("DELETE FROM sign_in_codes WHERE expires_at < now()", &[])
        .await?;
    client
        .execute(
            "INSERT INTO sign_in_codes (code_hash, user_id, challenge, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
            &[
                &secrets::hash_secret(&code),
                &user_id.0,
                &challenge,
                &f64::from(CODE_LIFETIME_SECONDS),
            ],
        )
        .await?;
    Ok(code)
}

/// The user the code was issued for, if it is fresh and `verifier` matches its challenge. The code is used up
/// by the attempt even when the verifier is wrong, so it cannot be guessed at.
pub async fn redeem_code(pool: &Pool, code: &str, verifier: &str) -> Result<UserId, CodeError> {
    let client = pool.get().await?;
    let row = client
        .query_opt(
            "DELETE FROM sign_in_codes WHERE code_hash = $1
             RETURNING user_id, challenge, expires_at > now() AS is_fresh",
            &[&secrets::hash_secret(code)],
        )
        .await?
        .ok_or(CodeError::Invalid)?;
    let challenge: String = row.get("challenge");
    // The challenge is no secret (it travelled in a URL), so comparing it in variable time reveals nothing.
    if !row.get::<_, bool>("is_fresh") || challenge_of(verifier) != challenge {
        return Err(CodeError::Invalid);
    }
    Ok(UserId(row.get("user_id")))
}
