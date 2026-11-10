//! Session tokens, one per account (a person's user id on a server). The released app keeps them in the
//! macOS Keychain; everything else (tests, examples, tools driving the app) keeps them in
//! memory for the life of the process, so no development build ever touches the Keychain.
//! Tokens never go to a file or a log.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use crate::accounts::registry::Account;

const SERVICE: &str = "Scroll session";
/// `errSecItemNotFound`.
const ITEM_NOT_FOUND: i32 = -25300;

/// Where tokens live: chosen once, by the real app's entry point.
enum TokenStore {
    Keychain,
    Memory(Mutex<HashMap<String, String>>),
}

static STORE: OnceLock<TokenStore> = OnceLock::new();

/// Called by the real app's entry point before anything reads a token. Without it tokens stay
/// in memory.
pub fn use_keychain() {
    if STORE.set(TokenStore::Keychain).is_err() {
        log::warn!("the token store was chosen before the app started");
    }
}

fn store() -> &'static TokenStore {
    STORE.get_or_init(|| TokenStore::Memory(Mutex::new(HashMap::new())))
}

#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    #[error("the Keychain refused: {0}")]
    Keychain(#[from] security_framework::base::Error),
    #[error("what the Keychain holds for this account is unreadable")]
    NotText,
}

/// The item's account name: two people may use one server from the same Mac.
fn item_name(account: &Account) -> String {
    format!("{} on {}", account.user_id.0, account.server_url)
}

pub fn save_session_token(account: &Account, token: &str) -> Result<(), KeychainError> {
    match store() {
        TokenStore::Keychain => Ok(set_generic_password(
            SERVICE,
            &item_name(account),
            token.as_bytes(),
        )?),
        TokenStore::Memory(tokens) => {
            let mut tokens = tokens.lock().unwrap_or_else(PoisonError::into_inner);
            tokens.insert(item_name(account), token.to_string());
            Ok(())
        }
    }
}

/// `None` when this account has no session here.
pub fn session_token(account: &Account) -> Result<Option<String>, KeychainError> {
    match store() {
        TokenStore::Keychain => match get_generic_password(SERVICE, &item_name(account)) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| KeychainError::NotText),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(error.into()),
        },
        TokenStore::Memory(tokens) => Ok(tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&item_name(account))
            .cloned()),
    }
}

pub fn delete_session_token(account: &Account) -> Result<(), KeychainError> {
    match store() {
        TokenStore::Keychain => match delete_generic_password(SERVICE, &item_name(account)) {
            Err(error) if error.code() != ITEM_NOT_FOUND => Err(error.into()),
            _ => Ok(()),
        },
        TokenStore::Memory(tokens) => {
            tokens
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&item_name(account));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scroll_core::id::UserId;

    #[test]
    fn without_the_app_entry_point_tokens_stay_in_memory() {
        let account = Account {
            server_url: "http://localhost:1".into(),
            email: "test@example.com".into(),
            display_name: "Test".into(),
            user_id: UserId::generate(),
        };
        save_session_token(&account, "secret").unwrap();
        assert!(matches!(store(), TokenStore::Memory(_)));
        assert_eq!(session_token(&account).unwrap().as_deref(), Some("secret"));
        delete_session_token(&account).unwrap();
        assert_eq!(session_token(&account).unwrap(), None);
    }
}
