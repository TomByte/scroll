//! The app window on GPUI's headless platform, over a real workspace file in a temporary
//! directory: keystrokes go through the real key bindings and actions.

use std::path::PathBuf;

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use scroll_core::id::PageId;

use crate::app::first_run;
use crate::commands;
use crate::editor;
use crate::window::AppWindow;
use crate::window::settings::AppSettings;
use crate::workspace::Workspace;
use crate::workspace::storage::Storage;

fn open_app(
    cx: &mut TestAppContext,
) -> (Entity<AppWindow>, Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        scroll_ui::init(cx);
        editor::init(cx);
        commands::bind_keys(crate::window::command_list::all(), cx);
    });
    let directory: PathBuf =
        std::env::temp_dir().join(format!("scroll-shell-{}", PageId::generate().0));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("Workspace.scroll");
    let storage = Storage::open(&path).unwrap();
    let records = first_run::seed(&storage);
    let workspace = cx.new(|_| Workspace::new(path, storage, records));
    let app_workspace = workspace.clone();
    let (app, cx) = cx.add_window_view(|window, cx| {
        AppWindow::new(app_workspace, AppSettings::default(), directory, window, cx)
    });
    (app, workspace, cx)
}

fn current_title(
    app: &Entity<AppWindow>,
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> String {
    let page = app.read_with(cx, |app, _| app.current_page()).unwrap();
    workspace.read_with(cx, |workspace, _| {
        workspace.tree().page(page).unwrap().title.clone()
    })
}

#[gpui::test]
fn the_first_page_opens_and_new_pages_join_the_history(cx: &mut TestAppContext) {
    let (app, workspace, cx) = open_app(cx);
    assert_eq!(current_title(&app, &workspace, cx), "Getting started");

    cx.simulate_keystrokes("cmd-n");
    assert_eq!(
        current_title(&app, &workspace, cx),
        "",
        "a new page starts untitled"
    );
    cx.simulate_input("Ideas");
    assert_eq!(
        current_title(&app, &workspace, cx),
        "Ideas",
        "typing in the title renames the page"
    );

    cx.simulate_keystrokes("cmd-[");
    assert_eq!(current_title(&app, &workspace, cx), "Getting started");
    cx.simulate_keystrokes("cmd-]");
    assert_eq!(current_title(&app, &workspace, cx), "Ideas");
}

#[gpui::test]
fn the_palette_opens_pages_and_runs_commands(cx: &mut TestAppContext) {
    let (app, workspace, cx) = open_app(cx);
    cx.simulate_keystrokes("cmd-p");
    cx.simulate_input("road");
    // Page search waits for a pause in typing.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(50));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(current_title(&app, &workspace, cx), "Roadmap");

    let roadmap = app.read_with(cx, |app, _| app.current_page()).unwrap();
    cx.simulate_keystrokes("cmd-p");
    cx.simulate_input("trash");
    cx.simulate_keystrokes("enter");
    let is_live = workspace.read_with(cx, |workspace, _| workspace.tree().is_live(roadmap));
    assert!(!is_live, "the command moved the page to the trash");
    assert_eq!(
        current_title(&app, &workspace, cx),
        "Getting started",
        "the window left the trashed page"
    );
}

#[gpui::test]
fn a_database_is_open_on_the_server_only_while_it_is_on_screen(cx: &mut TestAppContext) {
    let (app, workspace, cx) = open_app(cx);
    cx.simulate_keystrokes("cmd-alt-n");
    cx.run_until_parked();
    let host = app.read_with(cx, |app, _| app.current_page()).unwrap();
    let database = workspace
        .read_with(cx, |workspace, _| {
            workspace
                .records()
                .database_hosted_by(host)
                .map(|database| database.id)
        })
        .expect("the new page hosts a database");
    let is_open = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.is_database_open(database))
    };
    assert!(is_open(cx), "its table is on screen");

    cx.simulate_keystrokes("cmd-[");
    cx.run_until_parked();
    assert!(!is_open(cx), "leaving the page closes it");
}

#[gpui::test]
fn a_read_only_refusal_makes_everything_read_only_across_a_reconnect(cx: &mut TestAppContext) {
    use crate::editor::PageAccess;
    use scroll_sync::{ConnectionStatus, EditRefusal, SyncEvent};
    use scroll_table::TableAccess;

    let (app, workspace, cx) = open_app(cx);
    cx.simulate_keystrokes("cmd-alt-n");
    let has_table = |cx: &mut VisualTestContext| {
        app.read_with(cx, |app, cx| {
            app.page
                .as_ref()
                .and_then(|page| page.read(cx).database_view())
                .is_some_and(|database| database.read(cx).has_table())
        })
    };
    // The table opens once the new database reaches the file, on the storage thread.
    for _ in 0..100 {
        cx.run_until_parked();
        if has_table(cx) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(has_table(cx), "the new database shows its table");
    // Refused edits to another page: the role it reveals applies everywhere.
    let other_page = workspace
        .read_with(cx, |workspace, _| workspace.tree().first_live_page())
        .unwrap();
    let page_count = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.tree().children(None).len())
    };
    let is_read_only_everywhere = |cx: &mut VisualTestContext| {
        let pages_before = page_count(cx);
        // The sidebar and window refuse an edit through the same rule.
        cx.simulate_keystrokes("cmd-n");
        cx.run_until_parked();
        let is_tree_unchanged = page_count(cx) == pages_before;
        app.read_with(cx, |app, cx| {
            let page = app.page.as_ref().unwrap().read(cx);
            let table = page.database_view().unwrap().read(cx).table().unwrap();
            workspace.read(cx).is_read_only()
                && is_tree_unchanged
                && page.access() == PageAccess::ReadOnly
                && table.read(cx).access() == TableAccess::ReadOnly
        })
    };

    workspace.update(cx, |workspace, cx| {
        workspace.handle_sync_event(
            SyncEvent::PageEditsRefused {
                page_id: other_page,
                reason: EditRefusal::ReadOnly,
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(is_read_only_everywhere(cx));

    workspace.update(cx, |workspace, cx| {
        workspace.handle_sync_event(SyncEvent::Status(ConnectionStatus::Offline), cx);
        workspace.handle_sync_event(SyncEvent::Status(ConnectionStatus::Online), cx);
    });
    cx.run_until_parked();
    assert!(
        is_read_only_everywhere(cx),
        "a reconnect keeps it read-only"
    );
}
