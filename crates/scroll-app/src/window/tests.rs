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
        commands::keep_keys_bound(crate::window::command_list::all(), |_| {}, cx);
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

#[gpui::test]
fn the_settings_page_opens_with_cmd_comma_and_the_window_follows_its_theme(
    cx: &mut TestAppContext,
) {
    use crate::window::settings::{SETTINGS, THEME, ThemePreference};
    use scroll_ui::{Appearance, Theme};

    let (_app, _workspace, cx) = open_app(cx);
    cx.update(|_, cx| scroll_settings::register(SETTINGS, cx));
    let appearance =
        |cx: &mut VisualTestContext| cx.update(|window, cx| Theme::current(window, cx).appearance);

    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    // Past the two categories (Appearance and Shortcuts), the first control is the theme's first
    // option, "System"; the next is "Light".
    cx.simulate_keystrokes("tab tab tab tab");
    cx.simulate_keystrokes("space");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("space").unwrap(),
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| THEME.get(cx)),
        ThemePreference::Fixed(scroll_ui::ThemeId::ScrollLight)
    );
    assert_eq!(appearance(cx), Appearance::Light);

    cx.simulate_keystrokes("escape");
    cx.dispatch_action(commands::RunCommand {
        id: "view.theme_dark",
        page: None,
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| THEME.get(cx)),
        ThemePreference::Fixed(scroll_ui::ThemeId::ScrollDark)
    );
    assert_eq!(appearance(cx), Appearance::Dark);
}

#[gpui::test]
fn what_an_ai_tool_writes_shows_in_the_window(cx: &mut TestAppContext) {
    use scroll_core::operation::Transaction;
    use scroll_core::page_doc::{BlockKind, BlockPlacement};
    use scroll_core::store::WorkspaceStore;
    use scroll_mcp::McpEvent;

    use crate::workspace::{PageSyncEvent, WorkspaceEvent};

    let (app, workspace, cx) = open_app(cx);
    cx.run_until_parked();
    let page = app.read_with(cx, |app, _| app.current_page()).unwrap();
    let delivered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let heard = delivered.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&workspace, move |_, event: &WorkspaceEvent, _| {
            if let WorkspaceEvent::Page(page, PageSyncEvent::Updates(updates)) = event {
                heard.borrow_mut().push((*page, updates.len()));
            }
        })
    });
    let flushed = workspace.read_with(cx, |workspace, cx| workspace.flushed(cx));
    cx.background_executor.block(flushed).unwrap();

    // The MCP server's own connection renames the open page and adds a block to it.
    let path = workspace.read_with(cx, |workspace, _| workspace.path().clone());
    let mut tool_store = WorkspaceStore::open(&path).unwrap();
    let renamed = tool_store
        .apply_local_transaction(&Transaction::set_title(page, "Renamed by an AI"))
        .unwrap();
    let mut doc = tool_store.load_page_doc(page).unwrap();
    doc.insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    let update = doc.take_local_update().unwrap().unwrap();
    tool_store.record_local_page_update(page, &update).unwrap();

    workspace.update(cx, |workspace, cx| {
        workspace.mcp_changed(McpEvent::RecordsChanged(renamed), cx);
        workspace.mcp_changed(
            McpEvent::PageContentChanged {
                page_id: page,
                update,
            },
            cx,
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();

    assert_eq!(current_title(&app, &workspace, cx), "Renamed by an AI");
    assert_eq!(*delivered.borrow(), vec![(page, 1)]);
}

#[gpui::test]
fn the_full_width_command_flips_the_page_width_setting(cx: &mut TestAppContext) {
    use crate::page_layout::{PAGE_WIDTH, PageWidth};
    use crate::window::commands::TOGGLE_PAGE_WIDTH;

    let (_app, _workspace, cx) = open_app(cx);
    let run = |cx: &mut VisualTestContext| {
        cx.dispatch_action(commands::RunCommand {
            id: TOGGLE_PAGE_WIDTH,
            page: None,
        });
        cx.run_until_parked();
        cx.update(|_, cx| PAGE_WIDTH.get(cx))
    };
    assert_eq!(run(cx), PageWidth::Full);
    assert_eq!(run(cx), PageWidth::Constrained);
}

#[gpui::test]
async fn clicking_a_breadcrumb_opens_that_ancestor(cx: &mut TestAppContext) {
    let (app, workspace, cx) = open_app(cx);
    let parent = app.read_with(cx, |app, _| app.current_page()).unwrap();
    let child = workspace
        .update(cx, |workspace, cx| workspace.create_page(Some(parent), cx))
        .await
        .expect("the page is created");
    app.update_in(cx, |app, window, cx| app.open_page(child, window, cx));
    cx.run_until_parked();

    let crumb = cx
        .debug_bounds("crumb-0")
        .expect("the parent's breadcrumb is drawn");
    cx.simulate_click(crumb.center(), gpui::Modifiers::none());

    assert_eq!(app.read_with(cx, |app, _| app.current_page()), Some(parent));
}

#[gpui::test]
fn a_click_on_the_title_edits_it_again_after_typing_in_the_page(cx: &mut TestAppContext) {
    let (app, workspace, cx) = open_app(cx);
    cx.simulate_keystrokes("cmd-n");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("body");
    cx.run_until_parked();
    assert_eq!(current_title(&app, &workspace, cx), "");

    let title = cx.debug_bounds("page-title").expect("the title is drawn");
    cx.simulate_click(title.center(), gpui::Modifiers::none());
    cx.simulate_input("Plans");

    assert_eq!(current_title(&app, &workspace, cx), "Plans");
}

#[gpui::test]
fn the_settings_shortcut_works_with_nothing_focused_and_closes_the_page_too(
    cx: &mut TestAppContext,
) {
    use crate::settings::SettingsPage;

    let (app, _workspace, cx) = open_app(cx);
    let page = |cx: &mut VisualTestContext| {
        app.read_with(cx, |app, cx| app.overlays.showing::<SettingsPage>(cx))
    };
    let window = app.downgrade();
    cx.update(|window_state, cx| {
        crate::app::adopt_window_for_tests(window, cx);
        window_state.blur();
    });

    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    assert!(
        page(cx).is_some(),
        "the shortcut opens the page with nothing focused"
    );
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    assert!(page(cx).is_none(), "pressing it again closes the page");
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    assert!(page(cx).is_some(), "and once more opens it again");
}

/// Opens the settings page on its Shortcuts category and, with the keyboard alone, starts
/// recording the first command listed ("New page").
fn record_the_first_shortcut(cx: &mut VisualTestContext) {
    let press = |key: &str, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes(key);
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
        });
        cx.run_until_parked();
    };
    cx.update(|_, cx| scroll_settings::register(crate::window::settings::SETTINGS, cx));
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    // Appearance, then Shortcuts; then the first row's Change button.
    cx.simulate_keystrokes("tab tab");
    press("space", cx);
    cx.simulate_keystrokes("tab");
    press("space", cx);
}

#[gpui::test]
fn a_shortcut_recorded_in_settings_runs_its_command_in_place_of_the_old_one(
    cx: &mut TestAppContext,
) {
    use crate::settings::SettingsPage;

    let (app, workspace, cx) = open_app(cx);
    let preferences =
        std::env::temp_dir().join(format!("scroll-keys-{}.json", PageId::generate().0));
    cx.update(|_, cx| scroll_settings::init(preferences.clone(), cx));
    let is_settings_open = |cx: &mut VisualTestContext| {
        app.read_with(cx, |app, cx| {
            app.overlays.showing::<SettingsPage>(cx).is_some()
        })
    };
    let page_count = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.tree().children(None).len())
    };
    let new_page_keys = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| crate::commands::hint(crate::window::command_list::all(), "page.new", cx))
    };
    let pages_before = page_count(cx);
    record_the_first_shortcut(cx);

    // Keys already in use are refused while recording, and do not run what they are bound to.
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    assert!(is_settings_open(cx), "⌘, did not close the settings");
    cx.simulate_keystrokes("cmd-shift-n");
    cx.run_until_parked();
    assert_eq!(page_count(cx), pages_before, "⇧⌘N did not make a sub-page");
    assert_eq!(new_page_keys(cx), "⌘N");

    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    assert_eq!(new_page_keys(cx), "⌘J");
    assert_eq!(page_count(cx), pages_before, "recording ⌘J did not run it");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        !is_settings_open(cx),
        "with recording over, Escape closes the settings"
    );
    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    assert_eq!(
        page_count(cx),
        pages_before,
        "the old shortcut does nothing"
    );
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    assert_eq!(page_count(cx), pages_before + 1, "the new one makes a page");
    assert_eq!(current_title(&app, &workspace, cx), "");

    cx.update(|_, cx| scroll_settings::init(preferences.clone(), cx));
    assert_eq!(new_page_keys(cx), "⌘J", "the change was saved");
    std::fs::remove_file(preferences).ok();
}

#[gpui::test]
fn escape_while_recording_gives_up_without_closing_the_settings(cx: &mut TestAppContext) {
    use crate::settings::SettingsPage;

    let (app, _workspace, cx) = open_app(cx);
    let is_settings_open = |cx: &mut VisualTestContext| {
        app.read_with(cx, |app, cx| {
            app.overlays.showing::<SettingsPage>(cx).is_some()
        })
    };
    record_the_first_shortcut(cx);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(is_settings_open(cx));
    let keys = cx
        .update(|_, cx| crate::commands::hint(crate::window::command_list::all(), "page.new", cx));
    assert_eq!(keys, "⌘N");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!is_settings_open(cx));
}

mod page_switching {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Entity, TestAppContext, VisualTestContext};
    use scroll_core::id::PageId;
    use scroll_sync::{ConnectionStatus, SyncEvent};

    use super::open_app;
    use crate::editor::document::{BlockKind, PageDocument, Placement};
    use crate::window::AppWindow;
    use crate::window::page_switch::PAGE_WAIT;
    use crate::workspace::Workspace;
    use crate::workspace::storage;

    fn on_screen(app: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Option<PageId> {
        app.read_with(cx, |app, cx| app.page_on_screen(cx))
    }

    fn current(app: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Option<PageId> {
        app.read_with(cx, |app, _| app.current_page())
    }

    fn page_titled(
        title: &str,
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> PageId {
        workspace.read_with(cx, |workspace, _| {
            let tree = workspace.tree();
            let mut pending: Vec<PageId> = tree.children(None).iter().map(|page| page.id).collect();
            while let Some(page) = pending.pop() {
                if tree.page(page).is_some_and(|stored| stored.title == title) {
                    return page;
                }
                pending.extend(tree.children(Some(page)).iter().map(|child| child.id));
            }
            panic!("no page titled {title}")
        })
    }

    /// The page on screen now and every page put on screen from now on, in order.
    fn record_pages_on_screen(
        app: &Entity<AppWindow>,
        cx: &mut VisualTestContext,
    ) -> (Rc<RefCell<Vec<PageId>>>, gpui::Subscription) {
        let shown = Rc::new(RefCell::new(
            on_screen(app, cx).into_iter().collect::<Vec<_>>(),
        ));
        let recorded = shown.clone();
        let subscription = cx.update(|_, cx| {
            cx.observe(app, move |app, cx| {
                let page = app.read(cx).page_on_screen(cx);
                let mut recorded = recorded.borrow_mut();
                if page.is_some() && recorded.last().copied() != page {
                    recorded.extend(page);
                }
            })
        });
        (shown, subscription)
    }

    /// A new page under `parent` whose content has not arrived: in a workspace that is online
    /// it waits for it from the server, so it is not ready until that comes or times out.
    async fn page_still_loading(
        parent: PageId,
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> PageId {
        workspace.update(cx, |workspace, cx| {
            workspace.handle_sync_event(SyncEvent::Status(ConnectionStatus::Online), cx)
        });
        workspace
            .update(cx, |workspace, cx| workspace.create_page(Some(parent), cx))
            .await
            .expect("the page is created")
    }

    #[gpui::test]
    fn the_page_on_screen_stays_until_the_next_one_is_ready(cx: &mut TestAppContext) {
        let (app, workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let first = on_screen(&app, cx).expect("the first page is on screen");
        let roadmap = page_titled("Roadmap", &workspace, cx);

        app.update_in(cx, |app, window, cx| app.open_page(roadmap, window, cx));
        assert_eq!(
            current(&app, cx),
            Some(roadmap),
            "it is current from the click"
        );
        assert_eq!(on_screen(&app, cx), Some(first), "the page before stays up");

        cx.run_until_parked();
        assert_eq!(on_screen(&app, cx), Some(roadmap), "on screen once ready");
    }

    #[gpui::test]
    async fn a_page_still_loading_goes_on_screen_after_the_wait(cx: &mut TestAppContext) {
        let (app, workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let first = on_screen(&app, cx).unwrap();
        let slow = page_still_loading(first, &workspace, cx).await;

        app.update_in(cx, |app, window, cx| app.open_page(slow, window, cx));
        cx.run_until_parked();
        assert_eq!(on_screen(&app, cx), Some(first));
        assert!(
            cx.debug_bounds("crumb-1").is_some(),
            "the breadcrumbs already show the page being opened"
        );

        cx.executor().advance_clock(PAGE_WAIT);
        cx.run_until_parked();
        assert_eq!(on_screen(&app, cx), Some(slow), "shown, still loading");
    }

    #[gpui::test]
    async fn going_back_before_the_next_page_is_ready_stays_on_the_page(cx: &mut TestAppContext) {
        let (app, workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let first = on_screen(&app, cx).unwrap();
        let slow = page_still_loading(first, &workspace, cx).await;
        let (shown, _subscription) = record_pages_on_screen(&app, cx);

        app.update_in(cx, |app, window, cx| app.open_page(slow, window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-[");
        cx.executor().advance_clock(PAGE_WAIT * 2);
        cx.run_until_parked();

        assert_eq!(current(&app, cx), Some(first));
        assert_eq!(*shown.borrow(), [first], "the abandoned page never showed");
        cx.simulate_keystrokes("cmd-]");
        assert_eq!(current(&app, cx), Some(slow), "and can be gone forward to");
    }

    /// The user's report: "it's like it's showing a blank page quickly for a second before
    /// showing the populated page". Every frame drawn during a switch shows a page with its
    /// content, never an empty or still-loading page area, and a fade never starts from nothing.
    #[gpui::test]
    fn no_frame_shows_an_empty_page_area_during_a_switch(cx: &mut TestAppContext) {
        let (app, workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let first = on_screen(&app, cx).unwrap();
        let roadmap = page_titled("Roadmap", &workspace, cx);
        app.update(cx, |app, _| app.page_area_frames.clear());

        app.update_in(cx, |app, window, cx| app.open_page(roadmap, window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-[");
        cx.run_until_parked();

        let frames = app.read_with(cx, |app, _| app.page_area_frames.clone());
        assert!(frames.len() >= 2, "frames were drawn: {frames:?}");
        for frame in &frames {
            assert!(frame.page.is_some() && !frame.is_loading, "{frame:?}");
            assert!(
                frame.opacity >= 0.5,
                "at least half there from the start: {frame:?}"
            );
        }
        let pages: Vec<PageId> = frames.iter().filter_map(|frame| frame.page).collect();
        assert!(pages.contains(&roadmap), "the new page was drawn");
        assert_eq!(
            pages.last(),
            Some(&first),
            "and the old one again after going back"
        );
    }

    #[gpui::test]
    fn rapid_switching_ends_on_the_last_page_and_lets_the_others_go(cx: &mut TestAppContext) {
        let (app, workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let first = on_screen(&app, cx).unwrap();
        let (shown, _subscription) = record_pages_on_screen(&app, cx);
        let [projects, roadmap, reading] =
            ["Projects", "Roadmap", "Reading list"].map(|title| page_titled(title, &workspace, cx));

        let mut abandoned = Vec::new();
        for page in [projects, roadmap, reading] {
            app.update_in(cx, |app, window, cx| {
                app.open_page(page, window, cx);
                abandoned.extend(app.current_view().map(Entity::downgrade));
            });
        }
        cx.run_until_parked();

        assert_eq!(on_screen(&app, cx), Some(reading));
        assert_eq!(
            *shown.borrow(),
            [first, reading],
            "no page in between was shown"
        );
        assert!(
            abandoned[..2].iter().all(|view| view.upgrade().is_none()),
            "the pages abandoned on the way were let go"
        );
        cx.simulate_keystrokes("cmd-[");
        cx.run_until_parked();
        assert_eq!(
            on_screen(&app, cx),
            Some(roadmap),
            "the history kept every visit"
        );
    }

    #[gpui::test]
    fn opening_a_page_that_no_longer_exists_stays_on_the_page(cx: &mut TestAppContext) {
        let (app, _workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let first = on_screen(&app, cx).unwrap();

        app.update_in(cx, |app, window, cx| {
            app.open_page(PageId::generate(), window, cx)
        });
        cx.run_until_parked();

        assert_eq!(current(&app, cx), Some(first));
        assert_eq!(on_screen(&app, cx), Some(first));
        assert!(
            !app.read_with(cx, |app, _| app.history.can_go_back()),
            "it was forgotten"
        );
    }

    #[gpui::test]
    async fn a_page_with_a_diagram_goes_on_screen_with_it_drawn(cx: &mut TestAppContext) {
        let (app, workspace, cx) = open_app(cx);
        cx.run_until_parked();
        let page = workspace
            .update(cx, |workspace, cx| workspace.create_page(None, cx))
            .await
            .unwrap();
        let mut content = PageDocument::empty();
        let block = content
            .insert_block(Placement::LastChildOf(None), BlockKind::Code)
            .unwrap();
        content
            .insert_text(block, 0, "flowchart LR\n    A --> B")
            .unwrap();
        content.set_block_language(block, Some("mermaid")).unwrap();
        content.commit();
        let updates = content.take_outgoing_updates();
        workspace.read_with(cx, |workspace, _| {
            workspace.write("write a diagram page", move |store| {
                storage::record_updates(store, page, &updates)
            })
        });
        let ready_when_shown = Rc::new(RefCell::new(Vec::new()));
        let recorded = ready_when_shown.clone();
        let _subscription = cx.update(|_, cx| {
            cx.observe(&app, move |app, cx| {
                let app = app.read(cx);
                if let Some(view) = app
                    .page
                    .as_ref()
                    .filter(|_| app.page_on_screen(cx) == Some(page))
                {
                    recorded
                        .borrow_mut()
                        .push(view.read(cx).is_first_screen_ready(cx));
                }
            })
        });

        app.update_in(cx, |app, window, cx| app.open_page(page, window, cx));
        cx.run_until_parked();

        assert_eq!(on_screen(&app, cx), Some(page));
        assert_eq!(
            ready_when_shown.borrow().first(),
            Some(&true),
            "drawn from the first frame"
        );
    }
}
