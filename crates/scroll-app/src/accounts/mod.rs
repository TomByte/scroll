//! Servers and their accounts, and moving workspaces between this Mac and a server. The app
//! knows any number of servers, each with its own account; local workspaces need none.
//! Requests run on each account client's own runtime; results come back to the app's thread.

pub mod commands;
pub mod flows;
pub mod invite_dialog;
pub mod keychain;
pub mod registry;
pub mod sign_in_dialog;
mod state;
pub mod switcher;

pub use state::{Accounts, OpenWorkspace, request_open_workspace};

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AsyncApp, Task};
use scroll_core::id::Role;
use scroll_core::store::WorkspaceStore;
use scroll_sync::{
    AccountClient, AccountError, BrowserSignInRefusal, Membership, NewAccount, PublishError,
    ServerCheckError, ServerInfo, Session, publish_workspace,
};

use crate::accounts::registry::{Account, Registry, ServerLink};
use crate::workspace::Workspace;

/// How long the person has to finish signing in with Google in the browser.
const BROWSER_SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// What the person typed for an email sign-in, and whether they are creating the account.
#[derive(Clone, Debug)]
pub struct Credentials {
    pub server_url: String,
    pub email: String,
    pub password: String,
    pub intent: PasswordIntent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasswordIntent {
    SignIn,
    /// The code is required on invite-only servers and absent on open ones.
    CreateAccount {
        invite_code: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("{}", describe_account_error(.0))]
    Account(#[from] AccountError),
    #[error("Could not keep you signed in ({0}).")]
    Keychain(#[from] keychain::KeychainError),
    #[error("{0}")]
    Publish(String),
    #[error("Sign in to this server first.")]
    NotSignedIn,
    #[error("This workspace is only on this Mac. Publish it to a server first.")]
    NotOnServer,
}

/// A sentence a person can act on, for every account error.
pub fn describe_account_error(error: &AccountError) -> String {
    match error {
        AccountError::InvalidInvite => "That invite code is unknown, already used or expired.".into(),
        AccountError::EmailTaken => "An account with this email already exists. Sign in instead.".into(),
        AccountError::InvalidCredentials => "The email or password is wrong.".into(),
        AccountError::SignedOut => "You have been signed out. Sign in again.".into(),
        AccountError::Forbidden => "Only an owner of the workspace can do that.".into(),
        AccountError::WorkspaceExists => "That workspace already exists on the server.".into(),
        AccountError::InvalidInput { field } => format!("Check the {field}."),
        AccountError::RateLimited { retry_after } => {
            let seconds = retry_after.as_secs().max(1);
            let unit = if seconds == 1 { "second" } else { "seconds" };
            format!("Too many attempts. Try again in {seconds} {unit}.")
        }
        AccountError::Server { status } => format!("The server had a problem (status {status}). Try again shortly."),
        AccountError::InvalidServerUrl => "The server address must start with http:// or https://.".into(),
        AccountError::InvalidCertificate(_) => "The extra certificate for this server is not valid.".into(),
        AccountError::ServerNotTrusted(_) => "The server's certificate is not trusted, so the connection is not secure.".into(),
        AccountError::Network(_) => "Could not reach the server. Check the address and your connection.".into(),
        AccountError::Decoding(_) => {
            "That address answered, but not as a Scroll server this app can talk to. Check the address, or update the app or server.".into()
        }
        AccountError::Task(_) => "Something went wrong on this device. Try again.".into(),
        AccountError::InviteRequired => "This server needs an invite code to create an account.".into(),
        AccountError::AccountConflict => {
            "An account with this email is linked to a different Google account.".into()
        }
        AccountError::NotOffered => "This server does not offer that sign-in.".into(),
        AccountError::InvalidSignInCode => "Sign-in expired. Try again.".into(),
        AccountError::BrowserSignInRefused(refusal) => describe_refusal(*refusal).into(),
        AccountError::BrowserSignInTimedOut => "Sign-in in the browser timed out. Try again.".into(),
        AccountError::Loopback(_) => "Could not receive the browser's answer. Try again.".into(),
    }
}

fn describe_refusal(refusal: BrowserSignInRefusal) -> &'static str {
    match refusal {
        BrowserSignInRefusal::Cancelled => "Sign-in was cancelled.",
        BrowserSignInRefusal::InviteRequired => "No account yet; ask for an invite code.",
        BrowserSignInRefusal::InvalidInvite => "That invite code is unknown, used or expired.",
        BrowserSignInRefusal::EmailNotVerified => "Google has not verified this account's email.",
        BrowserSignInRefusal::AccountConflict => {
            "An account with this email is linked to a different Google account."
        }
        BrowserSignInRefusal::Failed => "Sign-in failed. Try again.",
    }
}

/// Checks that `address` is a Scroll server this app can talk to, before any credentials are
/// asked for. Resolves to the normalised address and what the server says about itself, or a
/// sentence saying what is wrong.
pub fn check_server(address: &str, cx: &mut App) -> Task<Result<(String, ServerInfo), String>> {
    let server_url = match scroll_sync::normalize_server_url(address) {
        Ok(url) => url,
        Err(error) => return Task::ready(Err(describe_account_error(&error))),
    };
    let client = match client_for(&server_url, cx) {
        Ok(client) => client,
        Err(error) => return Task::ready(Err(error.to_string())),
    };
    cx.spawn(async move |_| match client.server_info().await {
        Ok(info) => Ok((server_url, info)),
        Err(error) => Err(describe_server_check_error(&error)),
    })
}

/// A sentence a person can act on, for every reason an address is not usable.
pub fn describe_server_check_error(error: &ServerCheckError) -> String {
    match error {
        ServerCheckError::Unreachable(_) => {
            "Could not reach that address. Check it, and that the server is running.".into()
        }
        ServerCheckError::ServerNotTrusted(_) => {
            "The server's certificate is not trusted, so the connection would not be secure.".into()
        }
        ServerCheckError::NotAScrollServer => {
            "Something answered at that address, but it is not a Scroll server.".into()
        }
        ServerCheckError::IncompatibleProtocol { server, app } => format!(
            "This server needs a different version of Scroll (it speaks protocol {server}, this app {app}). Update whichever is older."
        ),
        ServerCheckError::RateLimited { retry_after } => {
            let seconds = retry_after.as_secs().max(1);
            format!("The server is busy. Try again in {seconds} s.")
        }
        ServerCheckError::Task(_) => "Something went wrong on this device. Try again.".into(),
    }
}

/// Signs in with email and password (creating the account first when asked), then keeps the
/// session as every sign-in does.
pub fn sign_in(credentials: Credentials, cx: &mut App) -> Task<Result<(), SessionError>> {
    let (server_url, client) = match server_client(&credentials.server_url, cx) {
        Ok(found) => found,
        Err(error) => return Task::ready(Err(error)),
    };
    cx.spawn(async move |cx: &mut AsyncApp| {
        if let PasswordIntent::CreateAccount { invite_code } = credentials.intent.clone() {
            client
                .sign_up(NewAccount {
                    email: credentials.email.clone(),
                    password: credentials.password.clone(),
                    invite_code,
                })
                .await?;
        }
        let session = client
            .sign_in(&credentials.email, &credentials.password)
            .await?;
        remember_session(&server_url, session, cx)
    })
}

/// Signs in with Google in the system browser. Dropping the task cancels: the app stops
/// listening for the browser's answer. When it completes, the app comes to the front.
pub fn sign_in_with_google(
    server_url: &str,
    invite_code: Option<String>,
    cx: &mut App,
) -> Task<Result<(), SessionError>> {
    let (server_url, client) = match server_client(server_url, cx) {
        Ok(found) => found,
        Err(error) => return Task::ready(Err(error)),
    };
    cx.spawn(async move |cx: &mut AsyncApp| {
        let browser = client.start_google_sign_in(invite_code.as_deref()).await?;
        let url = browser.url().to_string();
        cx.update(|cx| cx.open_url(&url))
            .map_err(|_| SessionError::NotSignedIn)?;
        let finished = browser.finish(BROWSER_SIGN_IN_TIMEOUT).await;
        let activated = cx.update(|cx| cx.activate(true));
        if activated.is_err() {
            log::debug!("the app quit during a browser sign-in");
        }
        remember_session(&server_url, finished?, cx)
    })
}

fn server_client(
    address: &str,
    cx: &mut App,
) -> Result<(String, Arc<AccountClient>), SessionError> {
    let server_url = scroll_sync::normalize_server_url(address)?;
    let client = client_for(&server_url, cx)?;
    Ok((server_url, client))
}

/// Keeps a new session (Keychain in the app, memory elsewhere), remembers the account, and
/// refreshes what depends on being signed in.
fn remember_session(
    server_url: &str,
    session: Session,
    cx: &mut AsyncApp,
) -> Result<(), SessionError> {
    let account = Account {
        server_url: server_url.to_string(),
        email: session.email,
        display_name: session.display_name,
        user_id: session.user_id,
    };
    keychain::save_session_token(&account, &session.session_token)?;
    let server_url = server_url.to_string();
    let remembered = cx.update(|cx| {
        Accounts::edit_registry(cx, |registry| {
            registry.set_account(&server_url, Some(account))
        });
        restart_sync_if_signed_out(&server_url, cx);
        refresh_server(&server_url, cx);
    });
    remembered.map_err(|_| SessionError::NotSignedIn)
}

/// Ends the session on `server_url` (best effort) and forgets it on this Mac. The server stays
/// listed so signing in again is one step.
pub fn sign_out(server_url: &str, cx: &mut App) {
    let account = Accounts::edit_registry(cx, |registry| {
        let account = registry.account(server_url).cloned();
        registry.set_account(server_url, None);
        account
    });
    let client = Accounts::get(cx)
        .server(server_url)
        .map(|session| session.client.clone());
    Accounts::update_session(cx, server_url, |session| session.workspaces.clear());
    let Some(account) = account else {
        return;
    };
    let token = keychain::session_token(&account).ok().flatten();
    if let Err(error) = keychain::delete_session_token(&account) {
        log::error!("could not remove the session from the Keychain: {error}");
    }
    mark_signed_out(server_url, cx);
    if let (Some(client), Some(token)) = (client, token) {
        cx.spawn(async move |_| {
            if let Err(error) = client.sign_out(&token).await {
                log::warn!(
                    "the server did not confirm signing out: {}",
                    describe_account_error(&error)
                );
            }
        })
        .detach();
    }
}

/// Signs out of `server_url` and forgets it. Copies of its workspaces stay on this Mac.
pub fn remove_server(server_url: &str, cx: &mut App) {
    sign_out(server_url, cx);
    Accounts::edit_registry(cx, |registry| registry.remove_server(server_url));
    Accounts::forget_session(cx, server_url);
    cx.refresh_windows();
}

/// Fetches the workspace list of every server signed in to.
pub fn refresh_all_servers(cx: &mut App) {
    let urls: Vec<String> = Accounts::get(cx)
        .registry()
        .servers
        .iter()
        .filter(|server| server.account.is_some())
        .map(|server| server.url.clone())
        .collect();
    for url in urls {
        refresh_server(&url, cx);
    }
}

/// Fetches the signed-in person's workspaces on `server_url`.
pub fn refresh_server(server_url: &str, cx: &mut App) {
    let Ok((client, token)) = session(server_url, cx) else {
        return;
    };
    let server_url = server_url.to_string();
    cx.spawn(async move |cx| {
        let fetched = client.workspaces(&token).await;
        let stored = cx.update(|cx| {
            if let Err(AccountError::SignedOut) = &fetched {
                mark_signed_out(&server_url, cx);
            }
            Accounts::update_session(cx, &server_url, |session| match fetched {
                Ok(memberships) => {
                    session.workspaces = memberships;
                    session.problem = None;
                }
                Err(error) => session.problem = Some(describe_account_error(&error)),
            });
            apply_role(&server_url, cx);
            cx.refresh_windows();
        });
        if stored.is_err() {
            log::debug!("the app quit before the workspace list arrived");
        }
    })
    .detach();
}

/// Opens a server workspace on this Mac, creating its linked file (and so downloading it) the
/// first time.
pub fn open_server_workspace(server_url: &str, membership: &Membership, cx: &mut App) {
    if let Some(index) = Accounts::get(cx)
        .registry()
        .index_of_server_workspace(server_url, membership.workspace_id)
    {
        return request_open_workspace(index, cx);
    }
    let link = ServerLink {
        server_url: server_url.to_string(),
        workspace_id: membership.workspace_id,
    };
    let file = Registry::new_workspace_file();
    let path = Accounts::get(cx).data_directory().join(&file);
    if let Err(error) = scroll_sync::create_linked_workspace_file(&path, server_url, membership) {
        return log::error!("could not create the workspace file: {error}");
    }
    let index = Accounts::edit_registry(cx, |registry| {
        registry.add_workspace(&membership.name, file, Some(link))
    });
    request_open_workspace(index, cx);
}

pub fn create_local_workspace(name: &str, cx: &mut App) {
    let file = Registry::new_workspace_file();
    let path = Accounts::get(cx).data_directory().join(&file);
    if let Err(error) =
        WorkspaceStore::open(&path).and_then(|mut store| store.set_workspace_name(name))
    {
        return log::error!("could not create the workspace file: {error}");
    }
    let index = Accounts::edit_registry(cx, |registry| registry.add_workspace(name, file, None));
    request_open_workspace(index, cx);
}

/// Publishes the open local workspace to `server_url`, then syncs it; the engine uploads its
/// pages and records in the background, reporting progress.
pub fn publish_current(server_url: &str, cx: &mut App) -> Task<Result<(), SessionError>> {
    let (client, token) = match session(server_url, cx) {
        Ok(session) => session,
        Err(error) => return Task::ready(Err(error)),
    };
    let accounts = Accounts::get(cx);
    let index = accounts.registry().current;
    let path = accounts.path_of(index);
    let name = accounts.registry().workspaces[index].name.clone();
    let server_url = server_url.to_string();
    let publishing = cx.background_executor().spawn({
        let token = token.clone();
        async move {
            let publish_error =
                |error: scroll_core::store::StoreError| SessionError::Publish(error.to_string());
            let mut store = WorkspaceStore::open(&path).map_err(publish_error)?;
            if store.workspace_name().unwrap_or_default().is_empty() {
                store.set_workspace_name(&name).map_err(publish_error)?;
            }
            publish_workspace(&client, &token, &mut store)
                .await
                .map_err(|error| match error {
                    PublishError::Account(error) => SessionError::Account(error),
                    other => SessionError::Publish(other.to_string()),
                })
        }
    });
    cx.spawn(async move |cx| {
        let membership = publishing.await?;
        let linked = cx.update(|cx| {
            Accounts::edit_registry(cx, |registry| {
                registry.workspaces[index].server = Some(ServerLink {
                    server_url: server_url.clone(),
                    workspace_id: membership.workspace_id,
                });
            });
            if let Some(workspace) = Workspace::current(cx) {
                workspace.update(cx, |workspace, cx| workspace.start_sync(token, cx));
            }
            refresh_server(&server_url, cx);
        });
        linked.map_err(|_| SessionError::NotSignedIn)
    })
}

/// A new invite code for the open server workspace.
pub fn create_invite(role: Role, cx: &mut App) -> Task<Result<String, SessionError>> {
    let Some(link) = Accounts::get(cx).registry().current().server.clone() else {
        return Task::ready(Err(SessionError::NotOnServer));
    };
    let (client, token) = match session(&link.server_url, cx) {
        Ok(session) => session,
        Err(error) => return Task::ready(Err(error)),
    };
    cx.spawn(async move |_| {
        Ok(client
            .create_invite(&token, link.workspace_id, role)
            .await?)
    })
}

/// Joins a workspace on `server_url` with an invite code and opens it.
pub fn accept_invite(
    server_url: &str,
    code: String,
    cx: &mut App,
) -> Task<Result<(), SessionError>> {
    let (client, token) = match session(server_url, cx) {
        Ok(session) => session,
        Err(error) => return Task::ready(Err(error)),
    };
    let server_url = server_url.to_string();
    cx.spawn(async move |cx| {
        let membership = client.accept_invite(&token, code.trim()).await?;
        let opened = cx.update(|cx| {
            open_server_workspace(&server_url, &membership, cx);
            refresh_server(&server_url, cx);
        });
        opened.map_err(|_| SessionError::NotSignedIn)
    })
}

/// The server a command without one should use: the open workspace's, else the only one
/// signed in to.
pub fn default_server(cx: &App) -> Option<String> {
    let registry = Accounts::get(cx).registry();
    if let Some(link) = registry.current().server.as_ref() {
        return Some(link.server_url.clone());
    }
    let mut signed_in = registry
        .servers
        .iter()
        .filter(|server| server.account.is_some());
    match (signed_in.next(), signed_in.next()) {
        (Some(only), None) => Some(only.url.clone()),
        _ => None,
    }
}

/// The account client for `server_url`, made the first time it is needed.
fn client_for(server_url: &str, cx: &mut App) -> Result<Arc<AccountClient>, SessionError> {
    Accounts::session_with(cx, server_url, || {
        Ok(Arc::new(AccountClient::new(server_url, None)?))
    })
}

fn session(server_url: &str, cx: &mut App) -> Result<(Arc<AccountClient>, String), SessionError> {
    let token = session_token_for(server_url, cx)?.ok_or(SessionError::NotSignedIn)?;
    Ok((client_for(server_url, cx)?, token))
}

/// The session token for `server_url`; `None` when not signed in there.
pub fn session_token_for(
    server_url: &str,
    cx: &App,
) -> Result<Option<String>, keychain::KeychainError> {
    match Accounts::get(cx).registry().account(server_url) {
        Some(account) => keychain::session_token(account),
        None => Ok(None),
    }
}

/// After signing in again, the open workspace starts syncing if it lives on this server and
/// had lost its session.
fn restart_sync_if_signed_out(server_url: &str, cx: &mut App) {
    let Some(workspace) = open_workspace_on(server_url, cx) else {
        return;
    };
    let Ok(Some(token)) = session_token_for(server_url, cx) else {
        return;
    };
    workspace.update(cx, |workspace, cx| {
        if workspace.connection() == Some(scroll_sync::ConnectionStatus::SignedOut) {
            workspace.start_sync(token, cx);
        }
    });
}

fn mark_signed_out(server_url: &str, cx: &mut App) {
    if let Some(workspace) = open_workspace_on(server_url, cx) {
        workspace.update(cx, |workspace, cx| {
            workspace.shut_down();
            workspace.mark_signed_out(cx);
        });
    }
}

/// Gives the open workspace the person's role there, as the server last listed it.
pub fn apply_role(server_url: &str, cx: &mut App) {
    let Some(workspace) = open_workspace_on(server_url, cx) else {
        return;
    };
    let accounts = Accounts::get(cx);
    let Some(link) = accounts.registry().current().server.as_ref() else {
        return;
    };
    let role = accounts.server(server_url).and_then(|session| {
        session
            .workspaces
            .iter()
            .find(|membership| membership.workspace_id == link.workspace_id)
            .map(|membership| membership.role)
    });
    workspace.update(cx, |workspace, cx| workspace.set_role(role, cx));
}

fn open_workspace_on(server_url: &str, cx: &App) -> Option<gpui::Entity<Workspace>> {
    let link = Accounts::get(cx).registry().current().server.as_ref()?;
    (link.server_url == server_url)
        .then(|| Workspace::current(cx))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_say_how_long_to_wait() {
        let error = AccountError::RateLimited {
            retry_after: Duration::from_secs(30),
        };
        assert_eq!(
            describe_account_error(&error),
            "Too many attempts. Try again in 30 seconds."
        );
        let error = AccountError::RateLimited {
            retry_after: Duration::from_millis(200),
        };
        assert_eq!(
            describe_account_error(&error),
            "Too many attempts. Try again in 1 second."
        );
    }

    #[test]
    fn every_account_error_reads_as_a_sentence() {
        let errors = [
            AccountError::InvalidInvite,
            AccountError::EmailTaken,
            AccountError::InvalidCredentials,
            AccountError::SignedOut,
            AccountError::Forbidden,
            AccountError::WorkspaceExists,
            AccountError::InvalidInput {
                field: "email".into(),
            },
            AccountError::Server { status: 503 },
            AccountError::InvalidServerUrl,
        ];
        for error in errors {
            let text = describe_account_error(&error);
            assert!(text.ends_with('.'), "{text:?}");
            assert!(
                text.chars().next().is_some_and(char::is_uppercase),
                "{text:?}"
            );
        }
    }
}
