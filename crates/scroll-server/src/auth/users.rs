//! The `users` table: accounts with a password, accounts a provider vouches for, and their names.

use deadpool_postgres::{Client, Pool, Transaction};
use scroll_api::accounts::SignUpMode;
use scroll_core::id::UserId;

use super::{AccountError, secrets};
use crate::db::DbError;
use crate::workspaces::invites;

pub struct NewAccount {
    pub email: String,
    pub password: String,
    /// Required when sign-up is by invite; with open sign-up, one can still bring a workspace grant.
    pub invite_code: Option<String>,
}

pub struct Credentials {
    pub email: String,
    pub password: String,
}

/// The email is not verified: whoever chose the password may not own it. See `identities`.
pub async fn create_account(
    pool: &Pool,
    account: NewAccount,
    sign_up: SignUpMode,
) -> Result<UserId, AccountError> {
    let invite_hash = account.invite_code.as_deref().map(secrets::hash_secret);
    if sign_up == SignUpMode::Invite && invite_hash.is_none() {
        return Err(AccountError::InviteRequired);
    }
    let password_hash = secrets::hash_password(account.password).await?;
    let user_id = UserId::generate();
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    let inserted = tx
        .execute(
            "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3) ON CONFLICT (email) DO NOTHING",
            &[&user_id.0, &account.email, &password_hash],
        )
        .await?;
    if inserted == 0 {
        return Err(AccountError::EmailTaken);
    }
    if let Some(invite_hash) = invite_hash {
        invites::apply(&tx, &invite_hash, user_id).await?;
    }
    tx.commit().await?;
    Ok(user_id)
}

/// The account the credentials belong to.
pub async fn authenticate(pool: &Pool, credentials: Credentials) -> Result<UserId, AccountError> {
    let client = pool.get().await?;
    let account = client
        .query_opt(
            "SELECT id, password_hash FROM users WHERE email = $1",
            &[&credentials.email],
        )
        .await?;
    let user_id = account.as_ref().map(|row| UserId(row.get("id")));
    // `None` for an unknown email or an account without a password: the same work, the same answer.
    let stored_hash = account.and_then(|row| row.get("password_hash"));
    let is_password_correct = secrets::verify_password(credentials.password, stored_hash).await?;
    user_id
        .filter(|_| is_password_correct)
        .ok_or(AccountError::InvalidCredentials)
}

/// The account's email and the name shown to others.
pub struct Profile {
    pub email: String,
    /// The provider's name for the person, or the email's local part.
    pub display_name: String,
}

pub async fn profile(client: &Client, user_id: UserId) -> Result<Profile, DbError> {
    let account = client
        .query_one(
            "SELECT email, display_name FROM users WHERE id = $1",
            &[&user_id.0],
        )
        .await?;
    let email: String = account.get("email");
    let display_name = account
        .get::<_, Option<String>>("display_name")
        .unwrap_or_else(|| email_name(&email));
    Ok(Profile {
        email,
        display_name,
    })
}

/// "ada" for "ada@example.com": a name for an account no provider named.
fn email_name(email: &str) -> String {
    email.split('@').next().unwrap_or(email).to_owned()
}

/// The account with this email, locked until the transaction ends.
pub(super) async fn lock_by_email(
    tx: &Transaction<'_>,
    email: &str,
) -> Result<Option<UserId>, tokio_postgres::Error> {
    let row = tx
        .query_opt(
            "SELECT id FROM users WHERE email = $1 FOR UPDATE",
            &[&email],
        )
        .await?;
    Ok(row.map(|row| UserId(row.get("id"))))
}

/// An account with no password, for someone a provider vouches for.
pub(super) async fn create_without_password(
    tx: &Transaction<'_>,
    email: &str,
) -> Result<UserId, tokio_postgres::Error> {
    let user_id = UserId::generate();
    tx.execute(
        "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, NULL)",
        &[&user_id.0, &email],
    )
    .await?;
    Ok(user_id)
}

pub(super) async fn clear_password(
    tx: &Transaction<'_>,
    user_id: UserId,
) -> Result<(), tokio_postgres::Error> {
    tx.execute(
        "UPDATE users SET password_hash = NULL WHERE id = $1",
        &[&user_id.0],
    )
    .await?;
    Ok(())
}

pub(super) async fn set_display_name(
    tx: &Transaction<'_>,
    user_id: UserId,
    name: &str,
) -> Result<(), tokio_postgres::Error> {
    tx.execute(
        "UPDATE users SET display_name = $2 WHERE id = $1",
        &[&user_id.0, &name],
    )
    .await?;
    Ok(())
}
