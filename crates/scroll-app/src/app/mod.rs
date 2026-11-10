//! The app around the window: startup, opening workspaces, the menu bar and quitting.

pub(crate) mod first_run;
mod menus;
mod open;
pub(crate) mod paths;
mod updating;

use std::path::PathBuf;
use std::time::Instant;

use gpui::{AnyWindowHandle, App, Application, Entity, Global, WeakEntity};

use crate::accounts::registry::Registry;
use crate::accounts::{self, Accounts, OpenWorkspace};
use crate::commands::{self, RunCommand};
use crate::window::{AppWindow, command_list, settings};
use crate::workspace::Workspace;
use crate::{editor, logger, mcp};
use paths::data_directory;

/// The windows the app has open. Lives as long as the app.
#[derive(Default)]
struct AppState {
    window: Option<WeakEntity<AppWindow>>,
    /// A small window saying the workspace file is being updated, while there is no other.
    updating_window: Option<AnyWindowHandle>,
}

impl Global for AppState {}

impl AppState {
    fn window(cx: &App) -> Option<Entity<AppWindow>> {
        cx.try_global::<AppState>()?.window.as_ref()?.upgrade()
    }

    fn set_window(window: WeakEntity<AppWindow>, cx: &mut App) {
        cx.default_global::<AppState>().window = Some(window);
    }

    fn set_updating_window(window: AnyWindowHandle, cx: &mut App) {
        cx.default_global::<AppState>().updating_window = Some(window);
    }

    fn take_updating_window(cx: &mut App) -> Option<AnyWindowHandle> {
        cx.default_global::<AppState>().updating_window.take()
    }
}

/// The action that runs command `id` from the command list, for tools that drive the app.
pub fn command_action(id: &str) -> Option<Box<dyn gpui::Action>> {
    let command = commands::find(command_list::all(), id)?;
    Some(Box::new(RunCommand {
        id: command.id,
        page: None,
    }))
}

/// The main window, once it is open; not the window saying the workspace is being updated.
pub fn main_window(cx: &App) -> Option<AnyWindowHandle> {
    AppState::window(cx).map(|window| window.read(cx).window_handle())
}

/// Runs the app until it quits.
pub fn run() {
    accounts::keychain::use_keychain();
    let application = Application::new().with_assets(scroll_ui::Assets);
    application.on_reopen(|cx| {
        if AppState::window(cx).is_none() {
            open::open_main_window(cx);
        }
    });
    application.run(|cx: &mut App| {
        logger::install();
        match data_directory() {
            Ok(directory) => start(directory, cx),
            Err(error) => {
                log::error!("no place to keep the workspace: {error}");
                return cx.quit();
            }
        }
        cx.activate(true);
    });
}

/// Opens the last workspace kept in `data_directory`, and its window, inside an already running
/// GPUI app. Separate from `run` so tools (such as the scale probe example) can drive the real
/// app on a directory of their own.
pub fn start(data_directory: PathBuf, cx: &mut App) {
    // With no window open, quitting from the menu or ⌘Q still works.
    cx.on_action(|command: &RunCommand, cx| {
        if command.id == crate::window::commands::QUIT {
            cx.quit();
        }
    });
    cx.on_action(|request: &OpenWorkspace, cx| open::open_workspace(request.index, cx));
    cx.on_app_quit(|cx| {
        prepare_to_quit(cx);
        async {}
    })
    .detach();
    cx.set_global(AppState::default());
    scroll_settings::init(data_directory.join(paths::PREFERENCES_FILE), cx);
    let loading = Instant::now();
    let registry = match Registry::load(&data_directory) {
        Ok(registry) => registry,
        Err(error) => {
            log::error!("{error}");
            set_up_ui(cx);
            return updating::show_startup_failure(error.to_string().into(), cx);
        }
    };
    log::debug!("timing: registry loaded in {:?}", loading.elapsed());
    let current = registry.current;
    Accounts::install(data_directory, registry, cx);
    open::open_workspace(current, cx);
    // After the workspace's first read is queued, so the storage thread reads while the menus
    // (most of this) are built; the window only opens once that read is back.
    set_up_ui(cx);
    accounts::refresh_all_servers(cx);
    log::debug!(
        "timing: start returned {:?} after launch",
        logger::started().elapsed()
    );
}

/// Themes, settings, key bindings and the menu bar.
fn set_up_ui(cx: &mut App) {
    let initialising = Instant::now();
    scroll_ui::init(cx);
    scroll_table::init(cx);
    editor::init(cx);
    scroll_settings::register(settings::SETTINGS, cx);
    scroll_settings::register(mcp::SETTINGS, cx);
    settings::follow_motion(cx);
    mcp::init(cx);
    commands::bind_keys(command_list::all(), cx);
    menus::set_menus(cx);
    log::debug!("timing: UI set up in {:?}", initialising.elapsed());
}

/// Stops the MCP server, saves settings and drains the storage queue so no edit is lost on quit.
fn prepare_to_quit(cx: &mut App) {
    mcp::stop(cx);
    if let Some(window) = AppState::window(cx) {
        window.update(cx, |window, cx| window.prepare_to_quit(cx));
    }
    if let Some(workspace) = Workspace::current(cx) {
        let storage = workspace.update(cx, |workspace, _| {
            workspace.shut_down();
            workspace.storage().clone()
        });
        storage.shut_down();
    }
}
