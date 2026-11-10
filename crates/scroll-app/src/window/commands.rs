//! The window's own commands: going back and forward, the sidebar, the theme, the page width,
//! closing and quitting.

use scroll_ui::IconName;

use crate::commands::available::always;
use crate::commands::{Category, Command, Effect, MenuName, Run, Shortcut, Surface};
use crate::window::settings::ThemePreference;

pub const GO_BACK: &str = "go.back";
pub const GO_FORWARD: &str = "go.forward";
pub const QUIT: &str = "app.quit";
pub const TOGGLE_PAGE_WIDTH: &str = "view.toggle_page_width";

/// What a window command does; the window carries it out.
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub enum WindowCommand {
    GoBack,
    GoForward,
    ToggleSidebar,
    TogglePageWidth,
    Theme(ThemePreference),
    Close,
    Quit,
}

pub static NAVIGATION: &[Command] = &[
    Command {
        id: GO_BACK,
        title: "Go back",
        keywords: "history previous",
        category: Category::Navigation,
        icon: IconName::ArrowLeft,
        shortcut: Shortcut::global("cmd-["),
        menu: Some(MenuName::Go),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.can_go_back,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::GoBack),
        },
    },
    Command {
        id: GO_FORWARD,
        title: "Go forward",
        keywords: "history next",
        category: Category::Navigation,
        icon: IconName::ArrowRight,
        shortcut: Shortcut::global("cmd-]"),
        menu: Some(MenuName::Go),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.can_go_forward,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::GoForward),
        },
    },
];

pub static VIEW: &[Command] = &[
    Command {
        id: "view.toggle_sidebar",
        title: "Toggle sidebar",
        keywords: "hide show",
        category: Category::Appearance,
        icon: IconName::Sidebar,
        shortcut: Shortcut::global("cmd-\\"),
        menu: Some(MenuName::View),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::ToggleSidebar),
        },
    },
    Command {
        id: TOGGLE_PAGE_WIDTH,
        title: "Toggle full width",
        keywords: "page wide constrained narrow",
        category: Category::Appearance,
        icon: IconName::ArrowLeftRight,
        shortcut: None,
        menu: Some(MenuName::View),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::TogglePageWidth),
        },
    },
    Command {
        id: "view.theme_system",
        title: "Theme: follow system",
        keywords: "appearance auto",
        category: Category::Appearance,
        icon: IconName::Sun,
        shortcut: None,
        menu: Some(MenuName::View),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::Theme(ThemePreference::System)),
        },
    },
    Command {
        id: "view.theme_light",
        title: "Theme: light",
        keywords: "appearance",
        category: Category::Appearance,
        icon: IconName::Sun,
        shortcut: None,
        menu: Some(MenuName::View),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::Theme(ThemePreference::Light)),
        },
    },
    Command {
        id: "view.theme_dark",
        title: "Theme: dark",
        keywords: "appearance night",
        category: Category::Appearance,
        icon: IconName::Moon,
        shortcut: None,
        menu: Some(MenuName::View),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::Theme(ThemePreference::Dark)),
        },
    },
    Command {
        id: "window.close",
        title: "Close window",
        keywords: "",
        category: Category::Window,
        icon: IconName::Close,
        shortcut: Shortcut::global("cmd-w"),
        menu: Some(MenuName::File),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::Close),
        },
    },
    Command {
        id: QUIT,
        title: "Quit Scroll",
        keywords: "exit",
        category: Category::Window,
        icon: IconName::Close,
        shortcut: Shortcut::global("cmd-q"),
        menu: Some(MenuName::App),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(WindowCommand::Quit),
        },
    },
];
