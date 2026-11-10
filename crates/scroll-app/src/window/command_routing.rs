//! Running commands: from keys, menus, the palette and page menus, to the part of the window that carries each out.

use self::commands::WindowCommand;
use crate::accounts::commands::AccountRequest;
use crate::accounts::{Accounts, OpenWorkspace, flows, request_open_workspace};
use crate::commands::{PageKind, Run, Situation, Surface, Target};
use crate::mcp::CopyConnectionCommand;
use crate::page::commands::{PageCommand, RenamePage};
use crate::page_layout::PAGE_WIDTH;
use crate::palette::commands::OpenPalette;
use crate::palette::{CommandPalette, PaletteEvent, PaletteSetup};
use crate::settings::{OpenSettings, SettingsPage};
use crate::window::settings::THEME;
use gpui::{Context, Window};
use scroll_core::id::PageId;

use super::*;

impl AppWindow {
    /// Runs a command sent as an action, by a key or a menu.
    pub fn run(&mut self, command: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let target = Target {
            page: command.page,
            ..Target::default()
        };
        self.run_command(command.id, target, window, cx)
    }

    /// Runs command `id` (from a key, a menu, the palette or a page's menu) for `target`.
    pub(super) fn run_command(
        &mut self,
        id: &str,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = crate::commands::find(command_list::all(), id) else {
            return log::warn!("no command called {id}");
        };
        if !command.is_allowed(!self.workspace.read(cx).is_read_only()) {
            return self.show_toast("You can view this workspace but not change it.", cx);
        }
        *self.command_usage.entry(command.id).or_default() += 1;
        let target = Target {
            current: self.current_page(),
            ..target
        };
        match &command.run {
            Run::Dispatch { to, action } => self.route(*to, action(&target), window, cx),
            Run::Choose { .. } => self.show_palette("", Some((command, target)), window, cx),
        }
    }

    /// Hands a command's action to the part of the window that carries it out.
    pub(super) fn route(
        &mut self,
        to: Surface,
        action: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match to {
            Surface::Window => self.run_window_action(action.as_ref(), window, cx),
            Surface::Workspace => self.run_page_command(action.as_ref(), window, cx),
            Surface::Page => {
                if let Some(page) = self.page.clone() {
                    page.update(cx, |page, cx| page.run(action.as_ref(), window, cx));
                }
            }
            Surface::Database => {
                let database = self
                    .page
                    .as_ref()
                    .and_then(|page| page.read(cx).database_view());
                if let Some(database) = database {
                    database.update(cx, |database, cx| database.run(action.as_ref(), window, cx));
                }
            }
            Surface::Focused => Self::send_to_focus(action, window, cx),
        }
    }

    /// Carries out a change to the workspace's pages, and opens the page it makes, if any.
    fn run_page_command(
        &mut self,
        action: &dyn gpui::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = action.as_any().downcast_ref::<PageCommand>() else {
            return log::warn!("{} is not a page command", action.name());
        };
        let created = crate::page::commands::run(command, &self.workspace, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(page) = created.await else {
                return;
            };
            if let Err(error) =
                this.update_in(cx, |this, window, cx| this.open_page(page, window, cx))
            {
                log::debug!("the window closed before a new page opened: {error}");
            }
        })
        .detach();
    }

    pub(super) fn run_window_action(
        &mut self,
        action: &dyn gpui::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = action.name();
        let action = action.as_any();
        if let Some(command) = action.downcast_ref::<WindowCommand>() {
            return self.run_window_command(command, window, cx);
        }
        if let Some(request) = action.downcast_ref::<AccountRequest>() {
            return flows::run(request.clone(), &self.overlays, window, cx);
        }
        if let Some(RenamePage { page }) = action.downcast_ref::<RenamePage>() {
            return self.rename_page(*page, window, cx);
        }
        if let Some(open) = action.downcast_ref::<OpenPalette>() {
            return self.show_palette(open.prefix(), None, window, cx);
        }
        if let Some(open) = action.downcast_ref::<OpenWorkspace>() {
            return request_open_workspace(open.index, cx);
        }
        if action.is::<OpenSettings>() {
            // The shortcut that opens the page also closes it.
            if self.overlays.showing::<SettingsPage>(cx).is_some() {
                self.overlays.dismiss_modal(window, cx);
            } else {
                self.overlays.show_modal(SettingsPage::new, window, cx);
            }
            return;
        }
        if action.is::<CopyConnectionCommand>() {
            return match crate::mcp::copy_connection_command(cx) {
                Ok(()) => self.show_toast("Copied", cx),
                Err(error) => self.show_toast(error.to_string(), cx),
            };
        }
        log::warn!("the window has no handler for {name}");
    }

    pub(super) fn run_window_command(
        &mut self,
        command: &WindowCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            WindowCommand::GoBack => self.go_back(window, cx),
            WindowCommand::GoForward => self.go_forward(window, cx),
            WindowCommand::ToggleSidebar => self.toggle_sidebar(cx),
            WindowCommand::TogglePageWidth => PAGE_WIDTH.set(PAGE_WIDTH.get(cx).toggled(), cx),
            WindowCommand::Theme(preference) => THEME.set(*preference, cx),
            WindowCommand::Close => window.remove_window(),
            WindowCommand::Quit => cx.quit(),
        }
    }

    pub(super) fn show_palette(
        &mut self,
        prefix: &'static str,
        command: Option<(&'static crate::commands::Command, Target)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let recent_pages: Vec<PageId> = std::iter::once(self.current_page())
            .flatten()
            .chain(self.history_pages())
            .fold(Vec::new(), |mut unique, page| {
                if !unique.contains(&page) {
                    unique.push(page);
                }
                unique
            });
        let setup = PaletteSetup {
            current_page: self.current_page(),
            commands: command_list::all(),
            workspace: self.workspace.clone(),
            situation: self.situation(cx),
            usage: self.command_usage.clone(),
            recent_pages,
            prefix,
            command,
        };
        let palette = self.overlays.show_modal(
            move |window, cx| CommandPalette::new(setup, window, cx),
            window,
            cx,
        );
        self.palette_subscription = Some(cx.subscribe_in(
            &palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| this.palette_event(event, window, cx),
        ));
    }

    /// What the commands need to know to decide whether they make sense now.
    pub(super) fn situation(&self, cx: &gpui::App) -> Situation {
        let records = self.workspace.read(cx).records();
        let page = self.current_page().and_then(|page| {
            let stored = records.tree.page(page)?;
            Some(if records.database_hosted_by(page).is_some() {
                PageKind::Database
            } else if stored.row_of.is_some() {
                PageKind::Row
            } else {
                PageKind::Document
            })
        });
        let servers = cx
            .try_global::<Accounts>()
            .map(|accounts| &accounts.registry().servers);
        Situation {
            page,
            can_edit: !self.workspace.read(cx).is_read_only(),
            has_table: self.has_table(cx),
            is_workspace_linked: self.workspace.read(cx).is_linked(),
            signed_in_servers: servers.map_or(0, |servers| {
                servers
                    .iter()
                    .filter(|server| server.account.is_some())
                    .count()
            }),
            known_servers: servers.map_or(0, Vec::len),
            can_go_back: self.history.can_go_back(),
            can_go_forward: self.history.can_go_forward(),
        }
    }

    pub(super) fn palette_event(
        &mut self,
        event: &PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PaletteEvent::OpenPage(page) => self.open_page(*page, window, cx),
            PaletteEvent::Run { id, target } => {
                let (id, target) = (*id, *target);
                // After the palette has closed and handed focus back.
                cx.defer_in(window, move |this, window, cx| {
                    this.run_command(id, target, window, cx)
                });
            }
            PaletteEvent::Chose { id, target, value } => {
                let Some(Run::Choose { pick, to, .. }) =
                    crate::commands::find(command_list::all(), id).map(|command| &command.run)
                else {
                    return;
                };
                let target = Target {
                    current: self.current_page(),
                    ..*target
                };
                let Some(action) = pick(&target, value.clone()) else {
                    return;
                };
                let to = *to;
                // After the palette has closed and handed focus back.
                cx.defer_in(window, move |this, window, cx| {
                    this.route(to, action, window, cx)
                });
            }
        }
    }

    /// Sends `action` to whatever has focus once the current event is done, so a palette that
    /// is closing has handed focus back first.
    pub(super) fn send_to_focus(
        action: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
    }
}
