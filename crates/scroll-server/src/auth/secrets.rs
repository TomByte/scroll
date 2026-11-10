//! Session tokens, invite codes and password hashes.

use std::sync::LazyLock;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{Error as HashError, PasswordHasher, PasswordVerifier};
use sha2::{Digest, Sha256};
use tokio::task::JoinError;

const SECRET_BYTES: usize = 32;

/// Verified against when an email is unknown, so a failed sign-in takes as long whether or not the account exists.
static UNKNOWN_ACCOUNT_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"no account has this password")
        .map(|hash: PasswordHash| hash.to_string())
        .expect("Argon2 with default parameters hashes any short password")
});

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password hashing failed: {0}")]
    Hashing(HashError),
    #[error("password hashing task did not finish: {0}")]
    Task(#[from] JoinError),
}

/// A random, URL-safe token for sessions and invites. Only its hash is stored.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).expect("the operating system's random source is available");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Secrets are 256 random bits, so a fast hash is enough; lookups by hash avoid timing-sensitive comparisons.
pub fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

/// Argon2id with a random salt. Runs on the blocking pool because it is deliberately slow.
pub async fn hash_password(password: String) -> Result<String, PasswordError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash: PasswordHash| hash.to_string())
            .map_err(PasswordError::Hashing)
    })
    .await?
}

/// `stored_hash` of `None` means the account does not exist; the work is still done and the answer is `false`.
pub async fn verify_password(
    password: String,
    stored_hash: Option<String>,
) -> Result<bool, PasswordError> {
    tokio::task::spawn_blocking(move || {
        let account_exists = stored_hash.is_some();
        let hash = stored_hash.unwrap_or_else(|| UNKNOWN_ACCOUNT_HASH.clone());
        match Argon2::default().verify_password(password.as_bytes(), hash.as_str()) {
            Ok(()) => Ok(account_exists),
            Err(HashError::PasswordInvalid) => Ok(false),
            Err(error) => Err(PasswordError::Hashing(error)),
        }
    })
    .await?
}
