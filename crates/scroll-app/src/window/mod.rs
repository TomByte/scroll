//! The main window: sidebar, toolbar, the open page, and the overlays (palette, dialogs and
//! notices) above them. It runs commands, handing each to the part that carries it out.

pub mod command_list;
mod command_routing;
pub mod commands;
pub mod history;
pub mod settings;
#[cfg(test)]
mod tests;
mod toolbar;

use std::collections::HashSet;
use std::path::PathBuf;

use gpui::{
    AnyWindowHandle, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Window, WindowBounds, div, prelude::FluentBuilder,
};
use scroll_core::id::PageId;
use scroll_settings::Settings;
use scroll_ui::theme::{
    follow_system_appearance, follow_system_in_window, force_window_appearance,
};
use scroll_ui::{Appearance, EmptyState, Theme};

use crate::accounts::Accounts;
use crate::commands::CommandUsage;
use crate::commands::{RunCommand, Target};
use crate::overlays::Overlays;
use crate::page::commands::NEW_PAGE;
use crate::page::{PageView, PageViewEvent};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::sync_status::empty_text;
use crate::window::history::History;
use crate::window::settings::{AppSettings, THEME, ThemePreference, WindowFrame};
use crate::workspace::page_tree::labelled_title;
use crate::workspace::settings::WorkspaceSettings;
use crate::workspace::{Workspace, WorkspaceEvent};

pub struct AppWindow {
    window_handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    sidebar: Entity<Sidebar>,
    page: Option<Entity<PageView>>,
    history: History,
    overlays: Overlays,
    settings: AppSettings,
    workspace_settings: WorkspaceSettings,
    data_directory: PathBuf,
    is_sidebar_open: bool,
    palette_subscription: Option<Subscription>,
    /// How often each command was run this session, so frequent ones rank first.
    command_usage: CommandUsage,
    page_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl AppWindow {
    pub fn new(
        workspace: Entity<Workspace>,
        settings: AppSettings,
        data_directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_settings = WorkspaceSettings::load(workspace.read(cx).path());
        let expanded: HashSet<PageId> = workspace_settings.expanded_pages.iter().copied().collect();
        let overlays = Overlays::new(cx);
        let sidebar = cx.new(|cx| {
            Sidebar::new(
                workspace.clone(),
                expanded,
                command_list::all(),
                overlays.clone(),
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&sidebar, window, Self::sidebar_event),
            cx.subscribe_in(&workspace, window, Self::workspace_changed),
            // The toolbar says when the workspace is being published.
            cx.observe_global::<Accounts>(|_, cx| cx.notify()),
            cx.observe_window_bounds(window, |this, window, _| this.remember_window_frame(window)),
            follow_system_appearance(window),
            cx.observe_global_in::<Settings>(window, |this, window, cx| {
                this.apply_theme(window, cx)
            }),
        ];
        let mut app_window = AppWindow {
            window_handle: window.window_handle(),
            workspace,
            sidebar,
            page: None,
            history: History::default(),
            overlays,
            settings,
            workspace_settings,
            data_directory,
            is_sidebar_open: true,
            palette_subscription: None,
            command_usage: CommandUsage::new(),
            page_subscriptions: Vec::new(),
            _subscriptions: subscriptions,
        };
        app_window.apply_theme(window, cx);
        let tree = app_window.workspace.read(cx).tree();
        // The last page is opened by id without reading the tree; if it is gone, the page
        // view says so and the first page opens instead.
        let start_page = app_window
            .workspace_settings
            .last_page
            .or_else(|| tree.first_live_page());
        if let Some(page) = start_page {
            app_window.open_page(page, window, cx);
        }
        app_window
    }

    pub fn window_handle(&self) -> AnyWindowHandle {
        self.window_handle
    }

    pub fn open_page(&mut self, page: PageId, window: &mut Window, cx: &mut Context<Self>) {
        self.history.visit(page);
        self.show_page(page, window, cx);
    }

    fn show_page(&mut self, page: PageId, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .page
            .as_ref()
            .is_some_and(|view| view.read(cx).page() == page)
        {
            return;
        }
        let workspace = self.workspace.clone();
        let my_name = cx.try_global::<Accounts>().and_then(|accounts| {
            let link = accounts.registry().current().server.as_ref()?;
            Some(
                accounts
                    .registry()
                    .account(&link.server_url)?
                    .display_name
                    .clone(),
            )
        });
        let view = cx.new(|cx| PageView::new(page, workspace, my_name, window, cx));
        self.page_subscriptions = vec![
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &PageViewEvent, window, cx| match event {
                    PageViewEvent::OpenPage(page) => this.open_page(*page, window, cx),
                    PageViewEvent::Missing => this.leave_missing_page(window, cx),
                },
            ),
            // Who else is on the page shows in the toolbar.
            cx.observe(&view, |_, _, cx| cx.notify()),
        ];
        self.page = Some(view);
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_current_page(Some(page), cx));
        self.workspace_settings.last_page = Some(page);
        self.update_window_title(window, cx);
        self.save_workspace_settings(cx);
        cx.notify();
    }

    pub(crate) fn current_page(&self) -> Option<PageId> {
        self.history.current()
    }

    fn sidebar_event(
        &mut self,
        _: &Entity<Sidebar>,
        event: &SidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SidebarEvent::OpenPage(page) => self.open_page(*page, window, cx),
            SidebarEvent::RunCommand(id, page) => self.run_command(
                id,
                Target {
                    page: *page,
                    ..Target::default()
                },
                window,
                cx,
            ),
            SidebarEvent::ExpandedChanged => {
                self.workspace_settings.expanded_pages =
                    self.sidebar.read(cx).expanded().iter().copied().collect();
                self.save_workspace_settings(cx);
            }
        }
    }

    /// Shows `message` at the bottom of the window for a few seconds.
    pub fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.overlays.toast(message, cx);
    }

    /// Follows renames into the window title, and leaves a page once it goes to the trash.
    fn workspace_changed(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspaceEvent::Notice(message) => return self.show_toast(message.clone(), cx),
            WorkspaceEvent::Page(..) => return,
            WorkspaceEvent::Loaded => {
                self.update_window_title(window, cx);
                return cx.notify();
            }
            WorkspaceEvent::ConnectionChanged => return cx.notify(),
            WorkspaceEvent::RecordsChanged(_) => {}
        }
        let tree = self.workspace.read(cx).tree();
        // Only a loaded page can be judged; one still loading is not "gone".
        if let Some(current) = self.current_page()
            && tree.page(current).is_some()
            && !tree.is_live(current)
        {
            let mut stale = vec![current];
            stale.extend(
                self.history_pages()
                    .into_iter()
                    .filter(|page| tree.page(*page).is_some() && !tree.is_live(*page)),
            );
            for page in stale {
                self.history.forget(page);
            }
            self.show_next_page(window, cx);
        }
        // A workspace that was empty (still downloading, say) opens its first page once it has one.
        let first_page = self.workspace.read(cx).tree().first_live_page();
        if self.page.is_none()
            && let Some(first) = first_page
        {
            self.open_page(first, window, cx);
        }
        self.update_window_title(window, cx);
        cx.notify();
    }

    /// The open page is gone (an old setting or link): forget it and show the first page.
    fn leave_missing_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(missing) = self.current_page() {
            self.history.forget(missing);
        }
        self.show_next_page(window, cx);
    }

    /// After the open page went away: the page before it in the history, else the first page.
    fn show_next_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self
            .history
            .current()
            .or_else(|| self.workspace.read(cx).tree().first_live_page());
        match next {
            Some(next) => {
                self.history.visit(next);
                self.show_page(next, window, cx);
            }
            None => {
                self.page = None;
                cx.notify();
            }
        }
    }

    fn history_pages(&self) -> Vec<PageId> {
        let mut history = self.history.clone();
        let mut pages = Vec::new();
        while let Some(page) = history.go_back() {
            pages.push(page);
        }
        pages
    }

    fn update_window_title(&self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .current_page()
            .and_then(|page| self.workspace.read(cx).tree().page(page))
            .map_or_else(|| "Scroll".to_string(), labelled_title);
        window.set_window_title(&title);
    }

    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.history.go_back() {
            self.show_page(page, window, cx);
        }
    }

    fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.history.go_forward() {
            self.show_page(page, window, cx);
        }
    }

    /// Renames in the sidebar for a page from its menu, in the title for the open page.
    fn rename_page(&mut self, page: Option<PageId>, window: &mut Window, cx: &mut Context<Self>) {
        match page.filter(|page| Some(*page) != self.current_page()) {
            Some(page) => self
                .sidebar
                .update(cx, |sidebar, cx| sidebar.start_renaming(page, window, cx)),
            None => {
                if let Some(view) = self.page.as_ref() {
                    view.update(cx, |view, cx| view.focus_title(window, cx));
                }
            }
        }
    }

    /// Whether the open page shows a table, for commands that act on one.
    pub(super) fn has_table(&self, cx: &gpui::App) -> bool {
        self.page
            .as_ref()
            .and_then(|page| page.read(cx).database_view())
            .is_some_and(|database| database.read(cx).has_table())
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.is_sidebar_open = !self.is_sidebar_open;
        cx.notify();
    }

    fn apply_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        match THEME.get(cx) {
            ThemePreference::System => follow_system_in_window(window, cx),
            ThemePreference::Light => force_window_appearance(Appearance::Light, window, cx),
            ThemePreference::Dark => force_window_appearance(Appearance::Dark, window, cx),
        }
    }

    fn remember_window_frame(&mut self, window: &mut Window) {
        let bounds = match window.window_bounds() {
            WindowBounds::Windowed(bounds)
            | WindowBounds::Maximized(bounds)
            | WindowBounds::Fullscreen(bounds) => bounds,
        };
        self.settings.window = Some(WindowFrame {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        });
    }

    fn save_workspace_settings(&self, cx: &mut Context<Self>) {
        let settings = self.workspace_settings.clone();
        let workspace_file = self.workspace.read(cx).path().clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = settings.save(&workspace_file) {
                    log::error!("could not save the workspace's settings: {error}");
                }
            })
            .detach();
    }

    /// Called on quit, before storage drains: settings go to disk now, and the open page's
    /// edits are queued for compaction so search and the file are up to date.
    pub fn prepare_to_quit(&self, cx: &mut gpui::App) {
        if let Err(error) = self.workspace_settings.save(self.workspace.read(cx).path()) {
            log::error!("could not save the workspace's settings: {error}");
        }
        if let Err(error) = self.settings.save(&self.data_directory) {
            log::error!("could not save settings: {error}");
        }
        if let Some(page) = self.page.as_ref() {
            page.update(cx, |page, cx| page.compact_now(cx));
        }
    }
}

impl Render for AppWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let sidebar_shown =
            scroll_ui::Transition::Sidebar.fraction_on("sidebar", self.is_sidebar_open, window, cx);
        let sidebar_width = crate::sidebar::SIDEBAR_WIDTH * sidebar_shown;
        let page_shown = scroll_ui::Transition::Page.progress(
            "page-content",
            self.page.as_ref().map(Entity::entity_id),
            window,
            cx,
        );
        let toolbar = self.render_toolbar(&theme, sidebar_width, cx);
        let content = match self.page.as_ref() {
            Some(page) => page.clone().into_any_element(),
            None => {
                let no_pages = format!(
                    "No pages. Press {} to create one.",
                    crate::commands::hint(command_list::all(), NEW_PAGE)
                );
                EmptyState::new(empty_text(&self.workspace.read(cx).sync_state(), &no_pages))
                    .into_any_element()
            }
        };
        scroll_ui::tab_navigation(div())
            .relative()
            .flex()
            .size_full()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.font.ui.clone())
            .on_action(cx.listener(|this, command: &RunCommand, window, cx| {
                let target = Target {
                    page: command.page,
                    ..Target::default()
                };
                this.run_command(command.id, target, window, cx)
            }))
            .when(sidebar_shown > 0.0, |root| {
                // The sidebar keeps its width and slides out from under this clip.
                root.child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_end()
                        .h_full()
                        .w(sidebar_width)
                        .overflow_hidden()
                        .child(self.sidebar.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(toolbar)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .when(page_shown < 1.0, |page| page.opacity(page_shown))
                            .child(content),
                    ),
            )
            .children(self.overlays.layers())
    }
}
