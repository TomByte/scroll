//! The `sessions` table. Only a hash of each token is kept.

use deadpool_postgres::{Pool, Transaction};
use scroll_core::id::UserId;

use super::secrets;
use super::users::{self, Profile};
use crate::db::DbError;

const SESSION_LIFETIME_DAYS: i32 = 30;

pub struct NewSession {
    pub user_id: UserId,
    pub token: String,
    pub email: String,
    /// The provider's name for the person, or the email's local part.
    pub display_name: String,
}

/// The token is returned once, with the account's email and name.
pub async fn create_session(pool: &Pool, user_id: UserId) -> Result<NewSession, DbError> {
    let token = secrets::generate_secret();
    let client = pool.get().await?;
    client
        .execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at)
             VALUES ($1, $2, now() + make_interval(days => $3))",
            &[
                &secrets::hash_secret(&token),
                &user_id.0,
                &SESSION_LIFETIME_DAYS,
            ],
        )
        .await?;
    let Profile {
        email,
        display_name,
    } = users::profile(&client, user_id).await?;
    Ok(NewSession {
        user_id,
        token,
        email,
        display_name,
    })
}

pub async fn end_session(pool: &Pool, token: &str) -> Result<(), DbError> {
    let client = pool.get().await?;
    client
        .execute(
            "DELETE FROM sessions WHERE token_hash = $1",
            &[&secrets::hash_secret(token)],
        )
        .await?;
    Ok(())
}

/// Ends every session of the user and returns their token hashes, so their live connections can be closed.
pub async fn end_all_sessions(
    tx: &Transaction<'_>,
    user_id: UserId,
) -> Result<Vec<Vec<u8>>, tokio_postgres::Error> {
    let ended = tx
        .query(
            "DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash",
            &[&user_id.0],
        )
        .await?;
    Ok(ended.iter().map(|row| row.get("token_hash")).collect())
}

/// `None` if the token is unknown or expired.
pub async fn session_user(pool: &Pool, token: &str) -> Result<Option<UserId>, DbError> {
    let client = pool.get().await?;
    let row = client
        .query_opt(
            "SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > now()",
            &[&secrets::hash_secret(token)],
        )
        .await?;
    Ok(row.map(|row| UserId(row.get("user_id"))))
}
