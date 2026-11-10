//! Opening a workspace: its file (updated first after an app update), its first read, and
//! the window showing it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, AppContext, Bounds, Entity, Task, TitlebarOptions, WindowBounds, WindowOptions, point, px,
    size,
};
use scroll_core::store::WorkspaceStore;

use super::{AppState, first_run, updating};
use crate::accounts::registry::WorkspaceEntry;
use crate::accounts::{self, Accounts};
use crate::logger;
use crate::title_bar::TRAFFIC_LIGHT_INSET;
use crate::window::AppWindow;
use crate::window::settings::AppSettings;
use crate::workspace::Workspace;
use crate::workspace::records::WorkspaceRecords;
use crate::workspace::storage::{self, Storage, StorageError};

const DEFAULT_WINDOW_SIZE: (f32, f32) = (1200.0, 820.0);
const MINIMUM_WINDOW_SIZE: (f32, f32) = (640.0, 420.0);

/// Opens workspace `index` from the registry in the window (opening the window if needed),
/// closing whatever was open. Its records are read on the storage thread first.
pub(super) fn open_workspace(index: usize, cx: &mut App) {
    let index = Accounts::edit_registry(cx, |registry| {
        registry.current = index.min(registry.workspaces.len().saturating_sub(1));
        registry.current
    });
    let entry = Accounts::get(cx).registry().workspaces[index].clone();
    let path = Accounts::get(cx).path_of(index);
    let opening = if needs_update(&path) {
        open_after_update(path.clone(), cx)
    } else {
        open_now(&path, cx)
    };
    cx.spawn(async move |cx| {
        let (storage, records) = match opening.await {
            Ok(opened) => opened,
            Err(error) => {
                log::error!("could not open {}: {error}", path.display());
                let reported = cx.update(|cx| report_open_failure(&entry.name, &error, cx));
                if let Err(error) = reported {
                    log::error!("the app quit while opening: {error}");
                }
                return;
            }
        };
        log::debug!(
            "timing: initial records received {:?} after launch",
            logger::started().elapsed()
        );
        let opened = cx.update(|cx| {
            let records = if is_brand_new(&records, &entry) {
                first_run::seed(&storage)
            } else {
                records
            };
            let token = entry
                .server
                .as_ref()
                .map(|link| accounts::session_token_for(&link.server_url, cx));
            let workspace = cx.new(|cx| {
                let mut workspace = Workspace::new(path, storage, records);
                if let Some(token) = token {
                    match token {
                        Ok(Some(token)) => workspace.start_sync(token, cx),
                        Ok(None) => workspace.mark_signed_out(cx),
                        Err(error) => {
                            log::error!("could not read the session from the Keychain: {error}");
                            workspace.mark_signed_out(cx);
                        }
                    }
                }
                workspace
            });
            workspace.update(cx, |workspace, cx| workspace.optimize_soon(cx));
            let showing = Instant::now();
            show_workspace(workspace, cx);
            log::debug!(
                "timing: workspace shown in {:?}, {:?} after launch",
                showing.elapsed(),
                logger::started().elapsed()
            );
            if let Some(link) = entry.server.as_ref() {
                accounts::apply_role(&link.server_url, cx);
            }
            // Once the window listens, so a server that cannot start can say so there, and after
            // the role (when already known), so a viewer's server never starts out offering edits.
            crate::mcp::serve_current_workspace(cx);
            updating::close(cx);
        });
        if let Err(error) = opened {
            log::error!("the app quit while loading: {error}");
        }
    })
    .detach();
}

/// Whether a workspace is new and gets the welcome pages: a local one whose first read
/// succeeded and found nothing. A linked one fills from its server instead.
fn is_brand_new(records: &WorkspaceRecords, entry: &WorkspaceEntry) -> bool {
    records.tree.is_empty() && entry.server.is_none()
}

/// Says the workspace could not be opened: in the open window, or in place of it at launch.
fn report_open_failure(name: &str, error: &StorageError, cx: &mut App) {
    updating::close(cx);
    let message = format!("Could not open {name}: {error}");
    match AppState::window(cx) {
        Some(window) => window.update(cx, |window, cx| window.show_toast(message, cx)),
        None => updating::show_startup_failure(message.into(), cx),
    }
}

/// The workspace file, opened, with its first read done.
type OpenedWorkspace = Result<(Arc<Storage>, WorkspaceRecords), StorageError>;

/// Whether the file must first be updated to this version's layout. If that cannot be told,
/// opening it reports the real problem.
fn needs_update(path: &Path) -> bool {
    WorkspaceStore::needs_migration(path).unwrap_or_else(|error| {
        log::error!(
            "could not tell whether {} needs updating: {error}",
            path.display()
        );
        false
    })
}

/// Opens a file already in this version's layout on the main thread (a few milliseconds) and
/// queues its first read at once, so the storage thread reads while the UI is set up.
fn open_now(path: &Path, cx: &mut App) -> Task<OpenedWorkspace> {
    let opening = Instant::now();
    match Storage::open(path) {
        Ok(storage) => {
            log::debug!("timing: workspace file opened in {:?}", opening.elapsed());
            read_initial(storage, cx)
        }
        Err(error) => Task::ready(Err(error)),
    }
}

/// Updates and opens the file off the main thread (seconds on a large file), saying so on
/// screen meanwhile.
fn open_after_update(path: PathBuf, cx: &mut App) -> Task<OpenedWorkspace> {
    // Deferred: at launch this runs before the themes the note is drawn with are installed.
    cx.defer(updating::show);
    let opening = cx
        .background_executor()
        .spawn(async move { Storage::open(&path) });
    cx.spawn(async move |cx| {
        let storage = opening.await?;
        match cx.update(|cx| read_initial(storage, cx)) {
            Ok(reading) => reading.await,
            // The app quit during the update; the storage goes with it.
            Err(_) => Err(StorageError::Stopped),
        }
    })
}

/// Reads what the window first shows.
fn read_initial(storage: Arc<Storage>, cx: &App) -> Task<OpenedWorkspace> {
    let reading = storage.read(cx, |store| storage::load_initial(store));
    cx.background_executor()
        .spawn(async move { Ok((storage, reading.await?)) })
}

/// Puts `workspace` in the window, replacing the one shown before.
fn show_workspace(workspace: Entity<Workspace>, cx: &mut App) {
    let previous_window = AppState::window(cx);
    if let Some(previous) = previous_window.as_ref() {
        previous.update(cx, |window, cx| window.prepare_to_quit(cx));
    }
    if let Some(previous) = Workspace::current(cx) {
        previous.update(cx, |previous, _| previous.shut_down());
    }
    Workspace::set_current(Some(workspace.clone()), cx);
    let Some(window) = previous_window else {
        return open_main_window(cx);
    };
    let handle = window.read(cx).window_handle();
    let data_directory = Accounts::get(cx).data_directory().to_path_buf();
    let replaced = handle.update(cx, |_, window, cx| {
        let settings = AppSettings::load(&data_directory);
        window.replace_root(cx, |window, cx| {
            AppWindow::new(workspace, settings, data_directory, window, cx)
        })
    });
    match replaced {
        Ok(root) => AppState::set_window(root.downgrade(), cx),
        Err(error) => log::error!("could not show the workspace: {error}"),
    }
}

pub(super) fn open_main_window(cx: &mut App) {
    let Some(workspace) = Workspace::current(cx) else {
        return;
    };
    let data_directory = Accounts::get(cx).data_directory().to_path_buf();
    let settings = AppSettings::load(&data_directory);
    let (width, height) = DEFAULT_WINDOW_SIZE;
    let bounds = match settings.window {
        Some(frame) => Bounds::new(
            point(px(frame.x), px(frame.y)),
            size(px(frame.width), px(frame.height)),
        ),
        None => Bounds::centered(None, size(px(width), px(height)), cx),
    };
    let (minimum_width, minimum_height) = MINIMUM_WINDOW_SIZE;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(minimum_width), px(minimum_height))),
        titlebar: Some(TitlebarOptions {
            title: Some("Scroll".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(TRAFFIC_LIGHT_INSET, TRAFFIC_LIGHT_INSET)),
        }),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        cx.new(|cx| AppWindow::new(workspace, settings, data_directory, window, cx))
    });
    match opened {
        Ok(handle) => match handle.update(cx, |_, _, cx| cx.entity().downgrade()) {
            Ok(window) => AppState::set_window(window, cx),
            Err(error) => log::error!("the window closed as it opened: {error}"),
        },
        Err(error) => {
            log::error!("could not open the window: {error:#}");
            cx.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::registry::ServerLink;
    use crate::workspace::page_tree::PageTree;
    use gpui::TestAppContext;
    use scroll_core::id::WorkspaceId;

    fn entry(server: Option<ServerLink>) -> WorkspaceEntry {
        WorkspaceEntry {
            name: "Notes".into(),
            file: "notes.scroll".into(),
            server,
        }
    }

    #[gpui::test]
    async fn a_failed_first_read_is_an_error_not_a_new_workspace(cx: &mut TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "scroll-open-{}.sqlite",
            scroll_core::id::PageId::generate().0
        ));
        let storage = Storage::open(&path).unwrap();
        storage.shut_down();

        let read = cx.update(|cx| read_initial(storage, cx)).await;

        assert!(matches!(read, Err(StorageError::Stopped)));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn only_an_empty_local_workspace_is_brand_new() {
        let empty = WorkspaceRecords::default();
        let mut with_a_page = WorkspaceRecords::default();
        let (_, transaction) = PageTree::default().create_page(None, "Garden").unwrap();
        with_a_page.apply(&transaction);
        let link = ServerLink {
            server_url: "https://scroll.example".into(),
            workspace_id: WorkspaceId::generate(),
        };

        assert!(is_brand_new(&empty, &entry(None)));
        assert!(!is_brand_new(&with_a_page, &entry(None)));
        assert!(!is_brand_new(&empty, &entry(Some(link))));
    }
}
