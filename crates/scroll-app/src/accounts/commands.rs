//! The workspace and server commands: open, make, join, publish and share workspaces, and add
//! and sign in to servers.

use gpui::App;

use scroll_ui::IconName;

use crate::accounts::registry::ServerEntry;
use crate::accounts::{Accounts, OpenWorkspace};
use crate::commands::available::{always, is_signed_in};
use crate::commands::{
    Category, Choice, ChoiceValue, Choices, Command, Effect, MenuName, Run, Shortcut, Surface,
    Target,
};

pub const SWITCH_WORKSPACE: &str = "workspace.switch";
pub const NEW_WORKSPACE: &str = "workspace.new";
pub const JOIN_WORKSPACE: &str = "workspace.join";
pub const PUBLISH: &str = "workspace.publish";
pub const INVITE: &str = "workspace.invite";
pub const INVITE_VIEWER: &str = "workspace.invite_viewer";
pub const ADD_SERVER: &str = "server.add";
pub const SIGN_IN: &str = "server.sign_in";
pub const SIGN_OUT: &str = "server.sign_out";
pub const REMOVE_SERVER: &str = "server.remove";

/// Something about workspaces or servers that needs the window, e.g. to show a dialog. A
/// server is named by its address; `None` means the one the open workspace uses, or the only
/// one known.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub enum AccountRequest {
    /// Signing in to a new server is how one is added.
    AddServer,
    SignIn(Option<String>),
    SignOut(Option<String>),
    RemoveServer(String),
    NewWorkspace,
    JoinWorkspace(Option<String>),
    Invite,
    InviteViewer,
    Publish(Option<String>),
}

fn workspace_choices(_: &Target, cx: &App) -> Vec<Choice> {
    let Some(accounts) = cx.try_global::<Accounts>() else {
        return Vec::new();
    };
    accounts
        .registry()
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let place = match entry.server.as_ref() {
                Some(link) => link
                    .server_url
                    .split("://")
                    .nth(1)
                    .unwrap_or(&link.server_url)
                    .to_string(),
                None => "this Mac".to_string(),
            };
            Choice {
                label: format!("{} — {place}", entry.name).into(),
                emoji: None,
                value: ChoiceValue::Workspace(index),
            }
        })
        .collect()
}

fn server_choices(cx: &App, keep: fn(&ServerEntry) -> bool) -> Vec<Choice> {
    let Some(accounts) = cx.try_global::<Accounts>() else {
        return Vec::new();
    };
    accounts
        .registry()
        .servers
        .iter()
        .filter(|server| keep(server))
        .map(|server| Choice {
            label: server.name().to_string().into(),
            emoji: None,
            value: ChoiceValue::Server(server.url.clone()),
        })
        .collect()
}

fn signed_in_servers(_: &Target, cx: &App) -> Vec<Choice> {
    server_choices(cx, |server| server.account.is_some())
}

fn all_servers(_: &Target, cx: &App) -> Vec<Choice> {
    server_choices(cx, |_| true)
}

pub static COMMANDS: &[Command] = &[
    Command {
        id: SWITCH_WORKSPACE,
        title: "Switch workspace",
        keywords: "open change",
        category: Category::Workspace,
        icon: IconName::ArrowRight,
        shortcut: Shortcut::global("cmd-shift-o"),
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Choose {
            prompt: "Open workspace…",
            choices: Choices::Fixed(workspace_choices),
            to: Surface::Window,
            pick: |_, value| match value {
                ChoiceValue::Workspace(index) => Some(Box::new(OpenWorkspace { index })),
                _ => None,
            },
        },
    },
    Command {
        id: NEW_WORKSPACE,
        title: "New workspace",
        keywords: "create local",
        category: Category::Workspace,
        icon: IconName::Plus,
        shortcut: None,
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(AccountRequest::NewWorkspace),
        },
    },
    Command {
        id: JOIN_WORKSPACE,
        title: "Join a workspace with an invite code",
        keywords: "accept invitation",
        category: Category::Workspace,
        icon: IconName::Plus,
        shortcut: None,
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: is_signed_in,
        run: Run::Choose {
            prompt: "Join on which server?",
            choices: Choices::Fixed(signed_in_servers),
            to: Surface::Window,
            pick: |_, value| match value {
                ChoiceValue::Server(server) => {
                    Some(Box::new(AccountRequest::JoinWorkspace(Some(server))))
                }
                _ => None,
            },
        },
    },
    Command {
        id: PUBLISH,
        title: "Publish this workspace to a server",
        keywords: "share upload sync",
        category: Category::Workspace,
        icon: IconName::ArrowRight,
        shortcut: None,
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| !situation.is_workspace_linked && situation.signed_in_servers > 0,
        run: Run::Choose {
            prompt: "Publish to which server?",
            choices: Choices::Fixed(signed_in_servers),
            to: Surface::Window,
            pick: |_, value| match value {
                ChoiceValue::Server(server) => {
                    Some(Box::new(AccountRequest::Publish(Some(server))))
                }
                _ => None,
            },
        },
    },
    Command {
        id: INVITE,
        title: "Invite people to this workspace",
        keywords: "share member",
        category: Category::Workspace,
        icon: IconName::Plus,
        shortcut: None,
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.is_workspace_linked,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(AccountRequest::Invite),
        },
    },
    Command {
        id: INVITE_VIEWER,
        title: "Invite someone to view only",
        keywords: "share read only viewer",
        category: Category::Workspace,
        icon: IconName::Plus,
        shortcut: None,
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.is_workspace_linked,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(AccountRequest::InviteViewer),
        },
    },
    Command {
        id: ADD_SERVER,
        title: "Add a server",
        keywords: "connect sign up account host",
        category: Category::Server,
        icon: IconName::Plus,
        shortcut: None,
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(AccountRequest::AddServer),
        },
    },
    Command {
        id: SIGN_IN,
        title: "Sign in to a server",
        keywords: "log in account",
        category: Category::Server,
        icon: IconName::ArrowRight,
        shortcut: None,
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.known_servers > 0,
        run: Run::Choose {
            prompt: "Sign in to which server?",
            choices: Choices::Fixed(all_servers),
            to: Surface::Window,
            pick: |_, value| match value {
                ChoiceValue::Server(server) => Some(Box::new(AccountRequest::SignIn(Some(server)))),
                _ => None,
            },
        },
    },
    Command {
        id: SIGN_OUT,
        title: "Sign out of a server",
        keywords: "log out account",
        category: Category::Server,
        icon: IconName::ArrowLeft,
        shortcut: None,
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: is_signed_in,
        run: Run::Choose {
            prompt: "Sign out of which server?",
            choices: Choices::Fixed(signed_in_servers),
            to: Surface::Window,
            pick: |_, value| match value {
                ChoiceValue::Server(server) => {
                    Some(Box::new(AccountRequest::SignOut(Some(server))))
                }
                _ => None,
            },
        },
    },
    Command {
        id: REMOVE_SERVER,
        title: "Remove a server",
        keywords: "forget disconnect",
        category: Category::Server,
        icon: IconName::Trash,
        shortcut: None,
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.known_servers > 0,
        run: Run::Choose {
            prompt: "Remove which server?",
            choices: Choices::Fixed(all_servers),
            to: Surface::Window,
            pick: |_, value| match value {
                ChoiceValue::Server(server) => Some(Box::new(AccountRequest::RemoveServer(server))),
                _ => None,
            },
        },
    },
];
