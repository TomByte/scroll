//! The JSON bodies of the account and workspace endpoints.

use scroll_core::id::{Role, UserId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Who may create an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignUpMode {
    #[default]
    Invite,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignInMethod {
    Password,
    Google,
    /// Offered by a newer server; the app does not show it.
    #[serde(other)]
    Unknown,
}

/// `GET /server-info`: what a server says about itself, for showing before anyone signs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub protocol_version: u16,
    /// The same as `sign_up == SignUpMode::Invite`.
    pub sign_up_requires_invite: bool,
    /// Absent from servers older than sign-in with Google, which all required an invite.
    #[serde(default)]
    pub sign_up: SignUpMode,
    #[serde(default = "password_only")]
    pub sign_in_methods: Vec<SignInMethod>,
}

fn password_only() -> Vec<SignInMethod> {
    vec![SignInMethod::Password]
}

/// The body of `GET /server-info`: `product` tells a Scroll server apart from any other web server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfoResponse {
    pub product: String,
    #[serde(flatten)]
    pub info: ServerInfo,
}

pub const PRODUCT: &str = "scroll-server";

/// `POST /accounts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAccountRequest {
    pub email: String,
    pub password: String,
    /// Required unless sign-up is open.
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCreated {
    pub user_id: UserId,
}

/// `POST /sessions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

/// `POST /sessions/exchange`: the last step of a browser sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeRequest {
    pub code: String,
    pub verifier: String,
}

/// A new session, from every way of signing in. The token goes in the Keychain; every other call needs it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub user_id: UserId,
    pub session_token: String,
    /// As the server keeps it (lowercase).
    pub email: String,
    /// Shown to others, as on the person's caret: the provider's name for them, or the email's local part.
    pub display_name: String,
}

/// `POST /workspaces`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub id: WorkspaceId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub role: Role,
}

/// `POST /workspaces/{workspace_id}/invites`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInviteRequest {
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteCreated {
    pub invite_code: String,
}

/// `POST /invites/accept`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptInviteRequest {
    pub invite_code: String,
}
