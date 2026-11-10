//! The commands that open the palette.

use scroll_ui::IconName;

use crate::commands::available::never_in_palette;
use crate::commands::{Category, Command, Effect, MenuName, Run, Shortcut, Surface};

pub const OPEN_PALETTE: &str = "go.palette";
pub const OPEN_COMMANDS: &str = "go.commands";

/// Opens the palette; the window shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub enum OpenPalette {
    /// Pages and commands.
    Everything,
    /// Only commands.
    Commands,
}

impl OpenPalette {
    /// What the palette's field starts with: ">" lists only commands.
    pub fn prefix(self) -> &'static str {
        match self {
            OpenPalette::Everything => "",
            OpenPalette::Commands => ">",
        }
    }
}

pub static COMMANDS: &[Command] = &[
    Command {
        id: OPEN_PALETTE,
        title: "Search or run a command",
        keywords: "palette find open page",
        category: Category::Navigation,
        icon: IconName::Search,
        shortcut: Shortcut::global("cmd-p"),
        menu: Some(MenuName::Go),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: never_in_palette,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(OpenPalette::Everything),
        },
    },
    Command {
        id: OPEN_COMMANDS,
        title: "Run a command",
        keywords: "palette actions",
        category: Category::Navigation,
        icon: IconName::Search,
        shortcut: Shortcut::global("cmd-shift-p"),
        menu: Some(MenuName::Go),
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: never_in_palette,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |_| Box::new(OpenPalette::Commands),
        },
    },
];
