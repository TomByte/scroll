//! The server's account and workspace endpoints, so the app contains no HTTP code.

use std::time::Duration;

use reqwest::Method;
use scroll_api::accounts::{
    AcceptInviteRequest, AccountCreated, CreateAccountRequest, CreateInviteRequest,
    CreateWorkspaceRequest, ExchangeRequest, InviteCreated, Membership, Session, SignInRequest,
};
use scroll_api::browser_sign_in::BrowserSignInRefusal;
use scroll_api::paths;
use scroll_core::id::{Role, UserId, WorkspaceId};
use tokio::task::JoinError;

use crate::server_check::normalize_server_url;
use crate::transport::{Transport, bare, post};

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("the invite code is unknown, already used or expired")]
    InvalidInvite,
    #[error("an account with this email already exists")]
    EmailTaken,
    #[error("the email or password is wrong")]
    InvalidCredentials,
    #[error("the session has ended; sign in again")]
    SignedOut,
    #[error("only an owner of the workspace can do that")]
    Forbidden,
    #[error("a workspace with this id already exists")]
    WorkspaceExists,
    #[error("the {field} is not valid")]
    InvalidInput { field: String },
    #[error("too many attempts; try again in {retry_after:?}")]
    RateLimited { retry_after: Duration },
    #[error("the server could not handle the request (status {status})")]
    Server { status: u16 },
    #[error("this is not a server address")]
    InvalidServerUrl,
    #[error("the extra root certificate is not a PEM certificate: {0}")]
    InvalidCertificate(reqwest::Error),
    #[error("the server's certificate is not trusted, or the secure connection failed: {0}")]
    ServerNotTrusted(reqwest::Error),
    #[error("could not reach the server: {0}")]
    Network(reqwest::Error),
    #[error("the server's response could not be read: {0}")]
    Decoding(#[from] serde_json::Error),
    #[error("the request task stopped: {0}")]
    Task(#[from] JoinError),
    #[error("this server needs an invite code to create an account")]
    InviteRequired,
    #[error("an account with this email is linked to a different Google account")]
    AccountConflict,
    #[error("this server does not offer that")]
    NotOffered,
    #[error("the sign-in code was not accepted; sign in again")]
    InvalidSignInCode,
    #[error("the sign-in in the browser did not complete: {0:?}")]
    BrowserSignInRefused(BrowserSignInRefusal),
    #[error("the sign-in in the browser was not finished in time")]
    BrowserSignInTimedOut,
    #[error("could not listen for the browser's return: {0}")]
    Loopback(std::io::Error),
}

#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub password: String,
    /// Required when the server's `sign_up` is `SignUpMode::Invite`.
    pub invite_code: Option<String>,
}

#[derive(Clone)]
pub struct AccountClient {
    transport: Transport,
}

impl AccountClient {
    /// Call once per server the app knows, with what the person typed: see `normalize_server_url`. The system's
    /// trusted certificates apply, plus `root_certificate` (PEM) if given, for a server whose certificate a
    /// private authority signed.
    pub fn new(server_url: &str, root_certificate: Option<&[u8]>) -> Result<Self, AccountError> {
        let server_url = normalize_server_url(server_url)?;
        Ok(Self {
            transport: Transport::new(server_url, root_certificate)?,
        })
    }

    /// The server this client talks to, normalized. Store this one, not what the person typed.
    pub fn server_url(&self) -> &str {
        self.transport.server_url()
    }

    pub(crate) fn transport(&self) -> &Transport {
        &self.transport
    }

    /// With an invite code unless sign-up is open. Sign in afterwards to get a session.
    pub async fn sign_up(&self, account: NewAccount) -> Result<UserId, AccountError> {
        let request = CreateAccountRequest {
            email: account.email,
            password: account.password,
            invite_code: account.invite_code,
        };
        let created: AccountCreated = self
            .transport
            .send_json(post(paths::ACCOUNTS, None, &request))
            .await?;
        Ok(created.user_id)
    }

    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Session, AccountError> {
        let request = SignInRequest {
            email: email.to_owned(),
            password: password.to_owned(),
        };
        self.transport
            .send_json(post(paths::SESSIONS, None, &request))
            .await
    }

    /// The session's sync connections close too.
    pub async fn sign_out(&self, session_token: &str) -> Result<(), AccountError> {
        let call = bare(Method::DELETE, paths::CURRENT_SESSION, session_token);
        self.transport.send(call).await?;
        Ok(())
    }

    /// The workspaces the user can open.
    pub async fn workspaces(&self, session_token: &str) -> Result<Vec<Membership>, AccountError> {
        let call = bare(Method::GET, paths::WORKSPACES, session_token);
        self.transport.send_json(call).await
    }

    /// A new, empty workspace on the server, which the user owns. To publish a local workspace use
    /// `publish_workspace` instead.
    pub async fn create_workspace(
        &self,
        session_token: &str,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<Membership, AccountError> {
        let request = CreateWorkspaceRequest {
            id: workspace_id,
            name: name.to_owned(),
        };
        self.transport
            .send_json(post(paths::WORKSPACES, Some(session_token), &request))
            .await
    }

    /// Returns the code to give the person invited.
    pub async fn create_invite(
        &self,
        session_token: &str,
        workspace_id: WorkspaceId,
        role: Role,
    ) -> Result<String, AccountError> {
        let path = paths::workspace_invites(workspace_id);
        let request = CreateInviteRequest { role };
        let invite: InviteCreated = self
            .transport
            .send_json(post(&path, Some(session_token), &request))
            .await?;
        Ok(invite.invite_code)
    }

    /// For a signed-in user joining another workspace.
    pub async fn accept_invite(
        &self,
        session_token: &str,
        invite_code: &str,
    ) -> Result<Membership, AccountError> {
        let request = AcceptInviteRequest {
            invite_code: invite_code.to_owned(),
        };
        self.transport
            .send_json(post(paths::ACCEPT_INVITE, Some(session_token), &request))
            .await
    }

    /// The last step of `BrowserSignIn::finish`.
    pub(crate) async fn exchange_sign_in_code(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<Session, AccountError> {
        let request = ExchangeRequest {
            code: code.to_owned(),
            verifier: verifier.to_owned(),
        };
        self.transport
            .send_json(post(paths::SESSION_EXCHANGE, None, &request))
            .await
    }
}
