//! Who someone is: accounts, passwords, sessions, sign-in with Google, and the limits on attempts.
//! Inputs arrive validated; callers do the authorisation.

pub mod browser_sign_in;
pub mod google;
pub mod identities;
pub mod live_sessions;
pub mod rate_limit;
pub mod secrets;
pub mod sessions;
pub mod users;

use deadpool_postgres::PoolError;

use crate::db::DbError;
use secrets::PasswordError;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("the invite code is unknown, already used or expired")]
    InvalidInvite,
    #[error("this server needs an invite code to create an account")]
    InviteRequired,
    #[error("the account with this email is linked to a different Google account")]
    IdentityConflict,
    #[error("an account with this email already exists")]
    EmailTaken,
    #[error("the email or password is wrong")]
    InvalidCredentials,
    #[error("a workspace with this id already exists")]
    WorkspaceExists,
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Password(#[from] PasswordError),
}

impl From<tokio_postgres::Error> for AccountError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(error.into())
    }
}

impl From<PoolError> for AccountError {
    fn from(error: PoolError) -> Self {
        Self::Database(error.into())
    }
}
