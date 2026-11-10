//! What each account request shows: the sign-in and invite dialogs, the prompts for a new
//! workspace or an invite code, and the notices while publishing.

use gpui::{App, Task, Window};
use scroll_core::id::Role;

use crate::accounts::commands::AccountRequest;
use crate::accounts::invite_dialog::InviteDialog;
use crate::accounts::sign_in_dialog::SignInDialog;
use crate::accounts::{self, Accounts};
use crate::overlays::Overlays;
use crate::overlays::prompt_dialog::PromptText;

/// Shown when a request needs a server and there is no obvious one.
const CHOOSE_SERVER: &str = "Choose a server from the workspace menu at the top of the sidebar.";

const NEW_WORKSPACE: PromptText = PromptText {
    title: "New workspace",
    explanation: "A workspace on this Mac. You can publish it to a server later.",
    placeholder: "Workspace name",
    action: "Create",
};

const JOIN_WORKSPACE: PromptText = PromptText {
    title: "Join a workspace",
    explanation: "Paste the invite code someone sent you.",
    placeholder: "Invite code",
    action: "Join",
};

/// Carries out `request`, showing its dialogs and notices on `overlays`.
pub fn run(request: AccountRequest, overlays: &Overlays, window: &mut Window, cx: &mut App) {
    match with_default_server(request, cx) {
        AccountRequest::AddServer => show_sign_in(None, overlays, window, cx),
        AccountRequest::SignIn(server) => show_sign_in(server, overlays, window, cx),
        AccountRequest::SignOut(Some(server)) => accounts::sign_out(&server, cx),
        AccountRequest::RemoveServer(server) => accounts::remove_server(&server, cx),
        AccountRequest::NewWorkspace => overlays.prompt(
            NEW_WORKSPACE,
            |name, cx| {
                accounts::create_local_workspace(&name, cx);
                Task::ready(Ok(()))
            },
            window,
            cx,
        ),
        AccountRequest::JoinWorkspace(Some(server)) => overlays.prompt(
            JOIN_WORKSPACE,
            move |code, cx| {
                let joining = accounts::accept_invite(&server, code, cx);
                cx.spawn(async move |_| joining.await.map_err(|error| error.to_string()))
            },
            window,
            cx,
        ),
        AccountRequest::Invite => {
            overlays.show_modal(InviteDialog::new, window, cx);
        }
        AccountRequest::InviteViewer => {
            overlays.show_modal(
                |window, cx| InviteDialog::new(window, cx).with_role(Role::Viewer),
                window,
                cx,
            );
        }
        AccountRequest::Publish(Some(server)) => publish(server, overlays, cx),
        AccountRequest::SignOut(None)
        | AccountRequest::JoinWorkspace(None)
        | AccountRequest::Publish(None) => overlays.toast(CHOOSE_SERVER, cx),
    }
}

/// The request with its server filled in where it named none and there is an obvious one.
fn with_default_server(request: AccountRequest, cx: &App) -> AccountRequest {
    let or_default = |server: Option<String>| server.or_else(|| accounts::default_server(cx));
    match request {
        AccountRequest::SignIn(server) => AccountRequest::SignIn(or_default(server)),
        AccountRequest::SignOut(server) => AccountRequest::SignOut(or_default(server)),
        AccountRequest::JoinWorkspace(server) => AccountRequest::JoinWorkspace(or_default(server)),
        AccountRequest::Publish(server) => AccountRequest::Publish(or_default(server)),
        AccountRequest::AddServer
        | AccountRequest::RemoveServer(_)
        | AccountRequest::NewWorkspace
        | AccountRequest::Invite
        | AccountRequest::InviteViewer => request,
    }
}

/// The sign-in form, for `server` or, with `None`, for a server not yet known.
fn show_sign_in(server: Option<String>, overlays: &Overlays, window: &mut Window, cx: &mut App) {
    let email = server
        .as_deref()
        .and_then(|server| Accounts::get(cx).registry().account(server))
        .map(|account| account.email.clone())
        .unwrap_or_default();
    let server = server.unwrap_or_default();
    overlays.show_modal(
        move |window, cx| SignInDialog::new(server, email, window, cx),
        window,
        cx,
    );
}

/// Publishes the open workspace to `server`, which can take minutes for a large one; the
/// toolbar says so meanwhile.
fn publish(server: String, overlays: &Overlays, cx: &mut App) {
    Accounts::start_publishing(cx);
    let publishing = accounts::publish_current(&server, cx);
    let overlays = overlays.clone();
    cx.spawn(async move |cx| {
        let message = match publishing.await {
            Ok(()) => "Published. Invite people from the workspace menu.".to_string(),
            Err(error) => format!("Could not publish: {error}"),
        };
        let shown = cx.update(|cx| {
            Accounts::finish_publishing(cx);
            overlays.toast(message, cx);
        });
        if let Err(error) = shown {
            log::debug!("the app quit before publishing finished: {error}");
        }
    })
    .detach();
}
