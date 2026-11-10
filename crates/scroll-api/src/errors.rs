//! The error bodies of the HTTP API: `{"error": code}`, plus `field` for invalid input.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    RateLimited,
    Unauthenticated,
    Forbidden,
    NotFound,
    Unavailable,
    /// A browser sign-in's one-time code was unknown, used, expired or for another verifier.
    InvalidCode,
    InvalidInvite,
    InviteRequired,
    AccountConflict,
    EmailTaken,
    InvalidCredentials,
    WorkspaceExists,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorCode,
    /// Which input was invalid, with `ErrorCode::InvalidInput`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}
