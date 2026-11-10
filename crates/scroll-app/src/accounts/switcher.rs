//! The menu at the top of the sidebar: workspaces on this Mac, then each server with its state
//! and its workspaces (opening one not yet here downloads it), and the commands to make, join,
//! publish and share workspaces and to manage servers and accounts.

use gpui::{App, Context, Window};
use scroll_sync::Membership;
use scroll_ui::{IconName, Menu, MenuItem};

use crate::accounts::commands::{
    ADD_SERVER, AccountRequest, COMMANDS, INVITE, JOIN_WORKSPACE, NEW_WORKSPACE, PUBLISH,
    REMOVE_SERVER, SIGN_IN, SIGN_OUT,
};
use crate::accounts::registry::{Registry, ServerEntry};
use crate::accounts::{Accounts, flows, request_open_workspace};
use crate::commands::{ChoiceValue, Run, Target};
use crate::overlays::Overlays;

/// One server's section, gathered before the menu is built.
struct ServerSection {
    server: ServerEntry,
    /// Workspaces with a copy on this Mac, by registry index.
    local_copies: Vec<(usize, String)>,
    /// Workspaces there with no copy here yet.
    elsewhere: Vec<Membership>,
    problem: Option<String>,
}

pub fn switcher_menu(overlays: Overlays, cx: &mut Context<Menu>) -> Menu {
    let registry = Accounts::get(cx).registry().clone();
    let sections = server_sections(&registry, cx);
    let item = |id: &str, server: Option<&str>| command_item(id, server, &overlays);
    let is_current_local = registry.current().server.is_none();

    let mut menu = Menu::new(cx).header("On this Mac");
    for (index, entry) in registry.workspaces.iter().enumerate() {
        let is_known_server = entry
            .server
            .as_ref()
            .is_some_and(|link| registry.server(&link.server_url).is_some());
        if !is_known_server {
            menu = menu.item(workspace_item(index, &entry.name, &registry));
        }
    }
    menu = menu.items(item(NEW_WORKSPACE, None));

    for section in sections {
        let url = section.server.url.as_str();
        menu = menu.separator().header(server_heading(&section));
        for (index, name) in &section.local_copies {
            menu = menu.item(workspace_item(*index, name, &registry));
        }
        for membership in &section.elsewhere {
            menu = menu.item(download_item(url, membership));
        }
        menu = match section.server.account.as_ref() {
            Some(_) => menu
                .items(item(JOIN_WORKSPACE, Some(url)))
                .items(is_current_local.then(|| item(PUBLISH, Some(url))).flatten())
                .items(item(SIGN_OUT, Some(url))),
            None => menu.items(item(SIGN_IN, Some(url))),
        };
        menu = menu.items(item(REMOVE_SERVER, Some(url)));
    }

    menu.separator()
        .items((!is_current_local).then(|| item(INVITE, None)).flatten())
        .items(item(ADD_SERVER, None))
}

fn server_sections(registry: &Registry, cx: &App) -> Vec<ServerSection> {
    let accounts = Accounts::get(cx);
    registry
        .servers
        .iter()
        .map(|server| {
            let session = accounts.server(&server.url);
            let memberships = session
                .map(|session| session.workspaces.clone())
                .unwrap_or_default();
            ServerSection {
                server: server.clone(),
                local_copies: copies_on(registry, &server.url),
                elsewhere: memberships
                    .into_iter()
                    .filter(|membership| {
                        registry
                            .index_of_server_workspace(&server.url, membership.workspace_id)
                            .is_none()
                    })
                    .collect(),
                problem: session.and_then(|session| session.problem.clone()),
            }
        })
        .collect()
}

/// The account command `id` as a menu item, for `server` if it asks for one.
fn command_item(id: &str, server: Option<&str>, overlays: &Overlays) -> Option<MenuItem> {
    let Some(command) = COMMANDS.iter().find(|command| command.id == id) else {
        log::error!("the workspace menu names a command that does not exist: {id}");
        return None;
    };
    let target = Target::default();
    let action = match (&command.run, server) {
        (Run::Dispatch { action, .. }, _) => Some(action(&target)),
        (Run::Choose { pick, .. }, Some(server)) => {
            pick(&target, ChoiceValue::Server(server.to_string()))
        }
        (Run::Choose { .. }, None) => None,
    };
    let request = action?.as_any().downcast_ref::<AccountRequest>()?.clone();
    let overlays = overlays.clone();
    Some(
        MenuItem::new(command.title, move |window: &mut Window, cx: &mut App| {
            flows::run(request.clone(), &overlays, window, cx)
        })
        .icon(command.icon),
    )
}

fn download_item(server_url: &str, membership: &Membership) -> MenuItem {
    let url = server_url.to_string();
    let membership = membership.clone();
    let label = format!("{} (download)", membership.name);
    MenuItem::new(label, move |_, cx: &mut App| {
        crate::accounts::open_server_workspace(&url, &membership, cx)
    })
    .icon(IconName::ArrowRight)
}

fn workspace_item(index: usize, name: &str, registry: &Registry) -> MenuItem {
    let item = MenuItem::new(name.to_string(), move |_, cx: &mut App| {
        request_open_workspace(index, cx)
    });
    if index == registry.current {
        item.icon(IconName::Check)
    } else {
        item
    }
}

fn copies_on(registry: &Registry, server_url: &str) -> Vec<(usize, String)> {
    registry
        .workspaces
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .server
                .as_ref()
                .is_some_and(|link| link.server_url == server_url)
        })
        .map(|(index, entry)| (index, entry.name.clone()))
        .collect()
}

/// "scroll.example.com — Ada Lovelace (ada@example.com)", or why it is not usable.
fn server_heading(section: &ServerSection) -> String {
    let name = section.server.name();
    match (&section.server.account, &section.problem) {
        (None, _) => format!("{name} — signed out"),
        (Some(_), Some(problem)) => format!("{name} — {problem}"),
        (Some(account), None) => format!("{name} — {}", account.label()),
    }
}

/// The open workspace's name, for the switcher button.
pub fn current_workspace_name(cx: &App) -> String {
    cx.try_global::<Accounts>()
        .map(|accounts| accounts.registry().current().name.clone())
        .unwrap_or_else(|| "Workspace".into())
}
