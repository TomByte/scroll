//! The rules inputs must meet before anything acts on them.

use scroll_api::browser_sign_in::{CHALLENGE_LENGTH, MIN_REDIRECT_PORT, VERIFIER_LENGTHS};

use super::ApiError;

const MAX_EMAIL_CHARS: usize = 254;
const MIN_PASSWORD_CHARS: usize = 10;
/// Argon2 accepts more, but hashing very long passwords is a cheap way to burn server CPU.
const MAX_PASSWORD_CHARS: usize = 256;
const MAX_INVITE_CODE_CHARS: usize = 128;
const MAX_WORKSPACE_NAME_CHARS: usize = 100;
const MAX_SIGN_IN_CODE_CHARS: usize = 128;

/// Emails are compared case-insensitively, so they are stored trimmed and lowercased.
pub fn normalize_email(email: &str) -> Result<String, ApiError> {
    let email = email.trim().to_lowercase();
    let is_well_formed = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && domain.contains('.') && !domain.contains('@')
    });
    let is_valid = is_well_formed
        && email.chars().count() <= MAX_EMAIL_CHARS
        && !email.contains(char::is_whitespace);
    if !is_valid {
        return Err(invalid("email"));
    }
    Ok(email)
}

/// Sign-in only bounds the size, so a change to the minimum never locks out existing accounts.
pub fn check_password_size(password: String) -> Result<String, ApiError> {
    if password.is_empty() || password.chars().count() > MAX_PASSWORD_CHARS {
        return Err(invalid("password"));
    }
    Ok(password)
}

pub fn check_new_password(password: String) -> Result<String, ApiError> {
    let length = password.chars().count();
    if !(MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&length) {
        return Err(invalid("password"));
    }
    Ok(password)
}

pub fn check_invite_code(invite_code: String) -> Result<String, ApiError> {
    if invite_code.is_empty() || invite_code.len() > MAX_INVITE_CODE_CHARS {
        return Err(invalid("invite_code"));
    }
    Ok(invite_code)
}

pub fn check_workspace_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_WORKSPACE_NAME_CHARS
        || name.contains(char::is_control)
    {
        return Err(invalid("name"));
    }
    Ok(name.to_owned())
}

pub fn check_sign_in_code(code: &str) -> Result<(), ApiError> {
    if code.is_empty() || code.len() > MAX_SIGN_IN_CODE_CHARS {
        return Err(invalid("code"));
    }
    Ok(())
}

pub fn check_verifier(verifier: &str) -> Result<(), ApiError> {
    if !VERIFIER_LENGTHS.contains(&verifier.len()) {
        return Err(invalid("verifier"));
    }
    Ok(())
}

pub fn check_redirect_port(port: u16) -> Result<u16, ApiError> {
    if port < MIN_REDIRECT_PORT {
        return Err(invalid("redirect_port"));
    }
    Ok(port)
}

pub fn check_challenge(challenge: String) -> Result<String, ApiError> {
    let is_base64url = challenge
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if challenge.len() != CHALLENGE_LENGTH || !is_base64url {
        return Err(invalid("challenge"));
    }
    Ok(challenge)
}

fn invalid(field: &'static str) -> ApiError {
    ApiError::InvalidInput { field }
}
