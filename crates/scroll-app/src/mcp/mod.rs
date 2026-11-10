//! The MCP server for AI tools, run for the open workspace while its setting is on: started,
//! restarted and stopped as the settings, the open workspace and the person's role change. What
//! it writes reaches the window through `Workspace::mcp_changed`.

use std::path::PathBuf;

use gpui::{App, ClipboardItem, Entity, EntityId, Global, Subscription, Task};
use scroll_mcp::{
    Access, DEFAULT_PORT, MCP_PATH, McpConfig, McpServer, StartError, TokenError,
    load_or_create_token,
};
use scroll_settings::{AnySetting, Category, Setting, Settings};
use scroll_ui::IconName;

use crate::accounts::Accounts;
use crate::app::paths::MCP_TOKEN_FILE;
use crate::commands::available::always;
use crate::commands::{Command, Effect, Run, Surface};
use crate::workspace::{Workspace, WorkspaceEvent};

pub const SERVER_ENABLED: Setting<bool> = Setting {
    key: "integrations.mcp_server",
    category: Category::Integrations,
    label: "MCP server",
    description: "Lets AI tools such as Claude Code read, search and edit the open workspace at \
                  http://127.0.0.1:7465/mcp.",
    default: false,
};

pub const ALLOWS_EDITS: Setting<bool> = Setting {
    key: "integrations.mcp_edits",
    category: Category::Integrations,
    label: "Allow AI tools to edit",
    description: "When off, AI tools can read and search the workspace but not change it.",
    default: true,
};

pub static SETTINGS: &[&dyn AnySetting] = &[&SERVER_ENABLED, &ALLOWS_EDITS];

pub const COPY_CONNECTION: &str = "mcp.copy_connection";

/// Puts the command that connects Claude Code on the clipboard; the window says how that went.
#[derive(Clone, Copy, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub struct CopyConnectionCommand;

pub static COMMANDS: &[Command] = &[Command {
    id: COPY_CONNECTION,
    title: "Copy MCP connection command",
    keywords: "ai claude model context protocol connect token",
    category: crate::commands::Category::Workspace,
    icon: IconName::Link,
    shortcut: None,
    menu: None,
    effect: Effect::Reads,
    is_in_page_menu: false,
    os_action: None,
    is_available: always,
    run: Run::Dispatch {
        to: Surface::Window,
        action: |_| Box::new(CopyConnectionCommand),
    },
}];

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("Turn on the MCP server in Settings first.")]
    TurnedOff,
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error(transparent)]
    Start(#[from] StartError),
}

/// What the server serves: a workspace, with or without edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    workspace: EntityId,
    access: Access,
}

#[derive(Default)]
struct McpState {
    running: Option<Running>,
    /// What the server was last started for, whether or not that worked: a failed start is not
    /// retried (and reported again) until the settings, the workspace or the role change.
    target: Option<Target>,
}

impl Global for McpState {}

struct Running {
    server: McpServer,
    events: Task<()>,
    _role: Subscription,
}

/// Follows the settings from now on. The app calls `serve_current_workspace` itself once a
/// workspace is shown.
pub fn init(cx: &mut App) {
    cx.set_global(McpState::default());
    cx.observe_global::<Settings>(serve_current_workspace)
        .detach();
}

/// Starts, restarts or stops the server so it serves the open workspace as the settings and the
/// person's role allow. Does nothing if that is already so.
pub fn serve_current_workspace(cx: &mut App) {
    let wanted = SERVER_ENABLED
        .get(cx)
        .then(|| Workspace::current(cx))
        .flatten()
        .map(|workspace| {
            let access = access(&workspace, cx);
            (workspace, access)
        });
    let target = wanted.as_ref().map(|(workspace, access)| Target {
        workspace: workspace.entity_id(),
        access: *access,
    });
    if cx.try_global::<McpState>().and_then(|state| state.target) == target {
        return;
    }
    stop(cx);
    cx.default_global::<McpState>().target = target;
    let Some((workspace, access)) = wanted else {
        return;
    };
    match start(&workspace, access, cx) {
        Ok(running) => cx.default_global::<McpState>().running = Some(running),
        Err(error) => {
            log::error!("could not start the MCP server: {error}");
            let notice = format!("Could not start the MCP server: {error}");
            workspace.update(cx, |_, cx| cx.emit(WorkspaceEvent::Notice(notice)));
        }
    }
}

/// Stops the server, waiting for a request it is answering. Called on quit.
pub fn stop(cx: &mut App) {
    let Some(running) = cx.default_global::<McpState>().running.take() else {
        return;
    };
    running.server.shut_down();
    // What the server wrote last still reaches the window; the events end with the server.
    running.events.detach();
}

/// Puts on the clipboard the command that adds this server to Claude Code, token included.
pub fn copy_connection_command(cx: &mut App) -> Result<(), McpError> {
    if !SERVER_ENABLED.get(cx) {
        return Err(McpError::TurnedOff);
    }
    let token = load_or_create_token(&token_path(cx))?;
    let command = format!(
        "claude mcp add --transport http scroll http://127.0.0.1:{DEFAULT_PORT}{MCP_PATH} \
         --header \"Authorization: Bearer {token}\""
    );
    cx.write_to_clipboard(ClipboardItem::new_string(command));
    Ok(())
}

/// Read-only when the person turned edits off or may only view the workspace.
fn access(workspace: &Entity<Workspace>, cx: &App) -> Access {
    if ALLOWS_EDITS.get(cx) && !workspace.read(cx).is_read_only() {
        Access::ReadWrite
    } else {
        Access::ReadOnly
    }
}

fn start(workspace: &Entity<Workspace>, access: Access, cx: &mut App) -> Result<Running, McpError> {
    let token = load_or_create_token(&token_path(cx))?;
    let (server, mut events) = McpServer::start(McpConfig {
        workspace_path: workspace.read(cx).path().clone(),
        port: DEFAULT_PORT,
        token,
        access,
    })?;
    let receiving = workspace.downgrade();
    let events = cx.spawn(async move |cx| {
        while let Some(event) = events.next().await {
            if receiving
                .update(cx, |workspace, cx| workspace.mcp_changed(event, cx))
                .is_err()
            {
                break;
            }
        }
    });
    // A viewer's role can arrive after the workspace opens, or change while it is open.
    let role = cx.subscribe(workspace, |_, event, cx| {
        if matches!(event, WorkspaceEvent::ConnectionChanged) {
            serve_current_workspace(cx);
        }
    });
    log::info!(
        "MCP server listening on http://{}{MCP_PATH}",
        server.address()
    );
    Ok(Running {
        server,
        events,
        _role: role,
    })
}

fn token_path(cx: &App) -> PathBuf {
    Accounts::get(cx).data_directory().join(MCP_TOKEN_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_names_the_address_the_server_listens_on() {
        let address = format!("http://127.0.0.1:{DEFAULT_PORT}{MCP_PATH}");
        assert!(SERVER_ENABLED.description.contains(&address));
    }
}
