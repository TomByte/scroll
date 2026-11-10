//! The settings page: every registered setting by category, opened with ⌘, from the app menu or
//! the palette. Settings themselves are declared next to what they change (see
//! `scroll_settings`); nothing here needs to know about them.

mod page;

use scroll_ui::IconName;

use crate::commands::available::always;
use crate::commands::{Category, Command, Effect, MenuName, Run, Shortcut, Surface};

pub use page::SettingsPage;

pub const OPEN_SETTINGS: &str = "app.settings";

/// Opens the settings page; the window shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub struct OpenSettings;

pub static COMMANDS: &[Command] = &[Command {
    id: OPEN_SETTINGS,
    title: "Settings…",
    keywords: "preferences options appearance theme",
    category: Category::Window,
    icon: IconName::Ellipsis,
    shortcut: Shortcut::global("cmd-,"),
    menu: Some(MenuName::App),
    effect: Effect::Reads,
    is_in_page_menu: false,
    os_action: None,
    is_available: always,
    run: Run::Dispatch {
        to: Surface::Window,
        action: |_| Box::new(OpenSettings),
    },
}];
