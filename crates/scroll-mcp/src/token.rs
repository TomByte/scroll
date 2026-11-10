//! The secret every request must carry: made from the system's random source when the server is
//! first turned on, and kept in a file only this user can read, outside any repository.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// 256 bits, written as hex.
const TOKEN_BYTES: usize = 32;
const OWNER_ONLY: u32 = 0o600;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("the system's random source failed: {0}")]
    Random(getrandom::Error),
    #[error("could not keep the MCP token in {path}: {source}")]
    File {
        path: String,
        source: std::io::Error,
    },
}

/// The token kept at `path`, made and saved first if there is none (or the file holds something
/// that is not one).
pub fn load_or_create_token(path: &Path) -> Result<String, TokenError> {
    let file_error = |source| TokenError::File {
        path: path.display().to_string(),
        source,
    };
    match std::fs::read_to_string(path) {
        Ok(stored) if is_token(stored.trim()) => return Ok(stored.trim().to_string()),
        Ok(_) => log::warn!("replacing the unreadable MCP token in {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(file_error(error)),
    }
    let token = generate_token()?;
    save(path, &token).map_err(file_error)?;
    Ok(token)
}

pub fn generate_token() -> Result<String, TokenError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(TokenError::Random)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Compares every byte whatever the first difference, so the time taken says nothing about how
/// much of a guess was right.
pub(crate) fn tokens_match(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0, |difference, (left, right)| difference | (left ^ right))
            == 0
}

fn is_token(text: &str) -> bool {
    text.len() == TOKEN_BYTES * 2 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Written beside the target and renamed over it, readable by this user alone from the start.
fn save(path: &Path, token: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(OWNER_ONLY)
        .open(&temporary)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn a_token_is_made_once_kept_private_and_read_back() {
        let path = std::env::temp_dir().join(format!(
            "scroll-mcp-token-{}-{}",
            std::process::id(),
            generate_token().unwrap()
        ));

        let token = load_or_create_token(&path).unwrap();
        assert!(is_token(&token));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, OWNER_ONLY);
        assert_eq!(load_or_create_token(&path).unwrap(), token);

        std::fs::write(&path, "not a token").unwrap();
        let replaced = load_or_create_token(&path).unwrap();
        assert!(is_token(&replaced));
        assert_ne!(replaced, token);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn tokens_match_only_in_full() {
        let token = generate_token().unwrap();
        assert!(tokens_match(&token, &token));
        assert!(!tokens_match(&token[..10], &token));
        assert!(!tokens_match(&format!("{token}0"), &token));
        assert!(!tokens_match("", &token));
    }
}
