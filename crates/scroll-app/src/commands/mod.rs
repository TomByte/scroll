//! What a command is, and the menu bar, key bindings and palette entries built from a list of
//! them. Each feature declares its own commands (`page::commands`, `database::commands`, …);
//! the window puts the lists together and runs them.
//!
//! A command sends an action to the surface that handles it (the window, the workspace's
//! pages, the open page, its database, or whatever has focus), first asking in the palette for something to
//! act on if it needs one (`Run::Choose`), such as a destination page.

pub mod available;
mod fuzzy;
mod ranking;

use std::rc::Rc;

use gpui::{
    Action, App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, SharedString,
};
use scroll_core::id::PageId;
use scroll_core::page_doc::BlockKind;
use scroll_ui::IconName;

pub use fuzzy::fuzzy_score;
pub use ranking::{CommandUsage, matching_commands};

/// Runs the command `id` from a key binding or a menu, for the page `page` (`None` means the
/// open page).
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub struct RunCommand {
    pub id: &'static str,
    pub page: Option<PageId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Page,
    Navigation,
    Block,
    Format,
    Database,
    Workspace,
    Server,
    Appearance,
    Window,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Page => "Page",
            Category::Navigation => "Go",
            Category::Block => "Block",
            Category::Format => "Format",
            Category::Database => "Database",
            Category::Workspace => "Workspace",
            Category::Server => "Server",
            Category::Appearance => "Appearance",
            Category::Window => "Window",
        }
    }
}

/// What the command acts on, from the palette or a context menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    /// The page a context menu was opened on; `None` means the open page.
    pub page: Option<PageId>,
    /// The page open in the window.
    pub current: Option<PageId>,
}

impl Target {
    pub fn page_or_current(&self) -> Option<PageId> {
        self.page.or(self.current)
    }
}

/// One answer to a command's question in the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub label: SharedString,
    pub emoji: Option<SharedString>,
    pub value: ChoiceValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChoiceValue {
    /// `None` is the top level of the workspace.
    Page(Option<PageId>),
    BlockKind(BlockKind),
    Workspace(usize),
    Server(String),
    /// An existing tag's name, or a new one to create.
    TagName(String),
    Property(scroll_core::id::PropertyId),
    View(scroll_core::id::ViewId),
}

/// Where a command's answers come from.
pub enum Choices {
    /// A short list, filtered in the palette as the person types.
    Fixed(fn(&Target, &App) -> Vec<Choice>),
    /// Pages found by the search index as the person types; never a scan of every page.
    Pages,
    /// The workspace's tags matching what is typed, plus "Create tag" for a new name.
    Tags,
}

/// The part of the window that handles a command's action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Window,
    /// The workspace's pages, for changes such as making, moving and tagging them.
    Workspace,
    /// The open page.
    Page,
    /// The open page's database, when it shows one.
    Database,
    /// Whatever has focus, such as the editor for formatting.
    Focused,
}

type MakeAction = fn(&Target) -> Box<dyn Action>;
type PickAction = fn(&Target, ChoiceValue) -> Option<Box<dyn Action>>;

pub enum Run {
    Dispatch {
        to: Surface,
        action: MakeAction,
    },
    /// Asks in the palette first, then sends what `pick` makes of the answer.
    Choose {
        prompt: &'static str,
        choices: Choices,
        to: Surface,
        pick: PickAction,
    },
}

/// The key that runs a command, in a key context (`None` is everywhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub keys: &'static str,
    pub context: Option<&'static str>,
}

impl Shortcut {
    pub const fn global(keys: &'static str) -> Option<Shortcut> {
        Some(Shortcut {
            keys,
            context: None,
        })
    }

    pub const fn in_context(keys: &'static str, context: &'static str) -> Option<Shortcut> {
        Some(Shortcut {
            keys,
            context: Some(context),
        })
    }

    /// "⇧⌘N" for "cmd-shift-n": modifiers in the order macOS menus show them, then the key.
    pub fn hint(&self) -> String {
        let parts: Vec<&str> = self.keys.split('-').collect();
        let (modifiers, key) = parts.split_at(parts.len().saturating_sub(1));
        let mut hint: String = [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("cmd", "⌘")]
            .iter()
            .filter(|(name, _)| modifiers.contains(name))
            .map(|(_, symbol)| *symbol)
            .collect();
        hint.push_str(&match key.first().copied().unwrap_or_default() {
            "enter" => "↵".to_string(),
            "backspace" => "⌫".to_string(),
            "up" => "↑".to_string(),
            "down" => "↓".to_string(),
            "escape" => "esc".to_string(),
            "tab" => "⇥".to_string(),
            "space" => "Space".to_string(),
            key => key.to_uppercase(),
        });
        hint
    }
}

/// Where in the menu bar a command appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuName {
    App,
    File,
    Edit,
    Format,
    View,
    Go,
}

/// What the window knows when deciding which commands make sense right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Situation {
    pub page: Option<PageKind>,
    /// False for a viewer of a server workspace.
    pub can_edit: bool,
    /// A database's table is on screen.
    pub has_table: bool,
    pub is_workspace_linked: bool,
    pub signed_in_servers: usize,
    pub known_servers: usize,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKind {
    Document,
    Database,
    Row,
}

/// Whether a command changes the workspace; viewers only get the ones that read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Reads,
    Edits,
}

pub struct Command {
    /// Stable, for usage counts and tests: "page.new".
    pub id: &'static str,
    pub title: &'static str,
    /// Extra words the palette matches, such as synonyms.
    pub keywords: &'static str,
    pub category: Category,
    pub icon: IconName,
    pub shortcut: Option<Shortcut>,
    pub menu: Option<MenuName>,
    pub effect: Effect,
    /// Offered in the sidebar's page menu, acting on that page.
    pub is_in_page_menu: bool,
    /// The standard Edit menu item macOS routes to text fields by itself, if this is one.
    pub os_action: Option<gpui::OsAction>,
    pub is_available: fn(&Situation) -> bool,
    pub run: Run,
}

impl Command {
    /// Whether this person may run it: anyone may run what only reads the workspace, and
    /// only someone who can edit may run what changes it.
    pub fn is_allowed(&self, can_edit: bool) -> bool {
        self.effect == Effect::Reads || can_edit
    }

    /// Whether it makes sense now, including whether this person may run it.
    pub fn is_usable(&self, situation: &Situation) -> bool {
        self.is_allowed(situation.can_edit) && (self.is_available)(situation)
    }
}

pub fn find(commands: &[&'static Command], id: &str) -> Option<&'static Command> {
    commands.iter().copied().find(|command| command.id == id)
}

/// How command `id`'s shortcut is shown, e.g. "⌘N"; empty if it has none.
pub fn hint(commands: &[&'static Command], id: &str) -> String {
    find(commands, id)
        .and_then(|command| command.shortcut)
        .map(|shortcut| shortcut.hint())
        .unwrap_or_default()
}

#[derive(Debug, thiserror::Error)]
#[error("the shortcut for {command} cannot be bound: {reason}")]
pub struct ShortcutError {
    command: &'static str,
    reason: String,
}

/// Every command's key binding. Commands handled by whatever has focus bind their own action,
/// so the component handling it works without the window around it (as in tests); the rest
/// bind `RunCommand`.
pub fn key_bindings(commands: &[&'static Command]) -> Result<Vec<KeyBinding>, ShortcutError> {
    commands
        .iter()
        .filter_map(|command| Some((*command, command.shortcut?)))
        .map(|(command, shortcut)| {
            let error = |reason: String| ShortcutError {
                command: command.id,
                reason,
            };
            let action: Box<dyn Action> = match &command.run {
                Run::Dispatch {
                    to: Surface::Focused,
                    action,
                } => action(&Target::default()),
                Run::Dispatch { .. } | Run::Choose { .. } => Box::new(RunCommand {
                    id: command.id,
                    page: None,
                }),
            };
            let context = shortcut
                .context
                .map(|context| {
                    KeyBindingContextPredicate::parse(context)
                        .map(Rc::new)
                        .map_err(|parse| error(parse.to_string()))
                })
                .transpose()?;
            KeyBinding::load(
                shortcut.keys,
                action,
                context,
                false,
                None,
                &DummyKeyboardMapper,
            )
            .map_err(|load| error(load.to_string()))
        })
        .collect()
}

/// Binds every command's shortcut. A shortcut that cannot be bound is a mistake in the list:
/// it stops a debug build, and a release build goes on without it.
pub fn bind_keys(commands: &[&'static Command], cx: &mut App) {
    match key_bindings(commands) {
        Ok(bindings) => cx.bind_keys(bindings),
        Err(error) if cfg!(debug_assertions) => panic!("{error}"),
        Err(error) => log::error!("{error}"),
    }
}

pub fn menu_items(commands: &[&'static Command], menu: MenuName) -> Vec<gpui::MenuItem> {
    commands
        .iter()
        .filter(|command| command.menu == Some(menu))
        .map(|command| match command.run {
            Run::Dispatch {
                to: Surface::Focused,
                action,
            } => gpui::MenuItem::Action {
                name: command.title.into(),
                action: action(&Target::default()),
                os_action: command.os_action,
            },
            _ => gpui::MenuItem::action(
                menu_title(command),
                RunCommand {
                    id: command.id,
                    page: None,
                },
            ),
        })
        .collect()
}

/// Menu items that ask a question end in an ellipsis, as macOS expects.
fn menu_title(command: &Command) -> String {
    match command.run {
        Run::Choose { .. } if !command.title.ends_with('…') => format!("{}…", command.title),
        _ => command.title.to_string(),
    }
}
