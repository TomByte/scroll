//! The page commands: make, rename, move, tag and trash pages, and the one place that
//! carries out the changes they make to the workspace.

use gpui::{App, Entity, Task};
use scroll_core::id::PageId;
use scroll_core::operation::{RecordId, Transaction};

use scroll_ui::IconName;

use crate::commands::available::{always, has_page};
use crate::commands::{
    Category, Choice, ChoiceValue, Choices, Command, Effect, MenuName, Run, Shortcut, Surface,
    Target,
};
use crate::workspace::Workspace;
use crate::workspace::page_tree::PageDrop;

pub const NEW_PAGE: &str = "page.new";
pub const NEW_SUB_PAGE: &str = "page.new_sub_page";
pub const NEW_DATABASE: &str = "page.new_database";
pub const ADD_TAG: &str = "page.add_tag";

/// A change to the workspace's pages, carried out by `run`.
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub enum PageCommand {
    NewPage {
        parent: Option<PageId>,
    },
    NewDatabase {
        parent: Option<PageId>,
    },
    Move {
        page: Option<PageId>,
        /// `None` is the top level.
        destination: Option<PageId>,
    },
    AddTag {
        page: Option<PageId>,
        name: String,
    },
    RemoveTag {
        page: Option<PageId>,
        name: String,
    },
    Trash {
        page: Option<PageId>,
    },
}

/// Renames a page: in the sidebar for a page from its menu, in the title for the open page.
/// The window decides which.
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub struct RenamePage {
    pub page: Option<PageId>,
}

/// Opens the open page's icon picker.
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub struct ChangeIcon;

/// Carries out `command` on `workspace`. Making a page or database resolves to the new page,
/// for the caller to open; the other commands resolve to `None`.
pub fn run(
    command: &PageCommand,
    workspace: &Entity<Workspace>,
    cx: &mut App,
) -> Task<Option<PageId>> {
    match command.clone() {
        PageCommand::NewPage { parent } => {
            return workspace.update(cx, |workspace, cx| workspace.create_page(parent, cx));
        }
        PageCommand::NewDatabase { parent } => {
            return workspace.update(cx, |workspace, cx| workspace.create_database(parent, cx));
        }
        PageCommand::Move {
            page: Some(page),
            destination,
        } => move_into(page, destination, workspace, cx),
        PageCommand::AddTag {
            page: Some(page),
            name,
        } => workspace.update(cx, |workspace, cx| workspace.add_tag(page, &name, cx)),
        PageCommand::RemoveTag {
            page: Some(page),
            name,
        } => remove_tag(page, &name, workspace, cx),
        PageCommand::Trash { page: Some(page) } => {
            workspace.update(cx, |workspace, cx| workspace.archive(page, cx))
        }
        PageCommand::Move { page: None, .. }
        | PageCommand::AddTag { page: None, .. }
        | PageCommand::RemoveTag { page: None, .. }
        | PageCommand::Trash { page: None } => {}
    }
    Task::ready(None)
}

/// Moves `page` to the end of `destination`'s children, or of the top level.
fn move_into(
    page: PageId,
    destination: Option<PageId>,
    workspace: &Entity<Workspace>,
    cx: &mut App,
) {
    let drop = match destination {
        Some(parent) => PageDrop::Inside(parent),
        None => match workspace.read(cx).tree().children(None).last() {
            Some(last) => PageDrop::After(last.id),
            None => return,
        },
    };
    workspace.update(cx, |workspace, cx| workspace.move_page(page, drop, cx));
}

fn remove_tag(page: PageId, name: &str, workspace: &Entity<Workspace>, cx: &mut App) {
    let tag = workspace
        .read(cx)
        .records()
        .tag_named(name)
        .map(|tag| tag.id);
    if let Some(tag) = tag {
        workspace.update(cx, |workspace, cx| {
            workspace.commit(Transaction::delete(RecordId::PageTag { page, tag }), cx)
        });
    }
}

fn current_page_tags(target: &Target, cx: &App) -> Vec<Choice> {
    let (Some(page), Some(workspace)) = (target.current, Workspace::current(cx)) else {
        return Vec::new();
    };
    workspace
        .read(cx)
        .records()
        .tags_of(page)
        .into_iter()
        .map(|tag| Choice {
            label: format!("#{}", tag.name).into(),
            emoji: None,
            value: ChoiceValue::TagName(tag.name.clone()),
        })
        .collect()
}

pub static COMMANDS: &[Command] = &[
    Command {
        id: NEW_PAGE,
        title: "New page",
        keywords: "create add document",
        category: Category::Page,
        icon: IconName::Plus,
        shortcut: Shortcut::global("cmd-n"),
        menu: Some(MenuName::File),
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Workspace,
            action: |_| Box::new(PageCommand::NewPage { parent: None }),
        },
    },
    Command {
        id: NEW_SUB_PAGE,
        title: "New sub-page",
        keywords: "create child nested inside",
        category: Category::Page,
        icon: IconName::Plus,
        shortcut: Shortcut::global("cmd-shift-n"),
        menu: Some(MenuName::File),
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Dispatch {
            to: Surface::Workspace,
            action: |target| {
                Box::new(PageCommand::NewPage {
                    parent: target.page_or_current(),
                })
            },
        },
    },
    Command {
        id: NEW_DATABASE,
        title: "New database",
        keywords: "table create",
        category: Category::Database,
        icon: IconName::Table,
        shortcut: Shortcut::global("cmd-alt-n"),
        menu: Some(MenuName::File),
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: always,
        run: Run::Dispatch {
            to: Surface::Workspace,
            action: |_| Box::new(PageCommand::NewDatabase { parent: None }),
        },
    },
    Command {
        id: "page.new_database_inside",
        title: "New database inside this page",
        keywords: "table create child",
        category: Category::Database,
        icon: IconName::Table,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Dispatch {
            to: Surface::Workspace,
            action: |target| {
                Box::new(PageCommand::NewDatabase {
                    parent: target.page_or_current(),
                })
            },
        },
    },
    Command {
        id: "page.rename",
        title: "Rename page",
        keywords: "title",
        category: Category::Page,
        icon: IconName::Pencil,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Dispatch {
            to: Surface::Window,
            action: |target| Box::new(RenamePage { page: target.page }),
        },
    },
    Command {
        id: "page.change_icon",
        title: "Change page icon",
        keywords: "emoji picture",
        category: Category::Page,
        icon: IconName::Pencil,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: has_page,
        run: Run::Dispatch {
            to: Surface::Page,
            action: |_| Box::new(ChangeIcon),
        },
    },
    Command {
        id: "page.move",
        title: "Move page to",
        keywords: "parent nest reorganise",
        category: Category::Page,
        icon: IconName::ArrowRight,
        shortcut: Shortcut::global("cmd-shift-m"),
        menu: Some(MenuName::File),
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Choose {
            prompt: "Move to…",
            choices: Choices::Pages,
            to: Surface::Workspace,
            pick: |target, value| match value {
                ChoiceValue::Page(destination) => Some(Box::new(PageCommand::Move {
                    page: target.page_or_current(),
                    destination,
                })),
                _ => None,
            },
        },
    },
    Command {
        id: ADD_TAG,
        title: "Add tag",
        keywords: "label categorise #",
        category: Category::Page,
        icon: IconName::Hash,
        shortcut: Shortcut::global("cmd-shift-t"),
        menu: Some(MenuName::File),
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Choose {
            prompt: "Tag with…",
            choices: Choices::Tags,
            to: Surface::Workspace,
            pick: |target, value| match value {
                ChoiceValue::TagName(name) => Some(Box::new(PageCommand::AddTag {
                    page: target.page_or_current(),
                    name,
                })),
                _ => None,
            },
        },
    },
    Command {
        id: "page.remove_tag",
        title: "Remove tag",
        keywords: "untag label",
        category: Category::Page,
        icon: IconName::Hash,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Choose {
            prompt: "Remove which tag?",
            choices: Choices::Fixed(current_page_tags),
            to: Surface::Workspace,
            pick: |target, value| match value {
                ChoiceValue::TagName(name) => Some(Box::new(PageCommand::RemoveTag {
                    page: target.page_or_current(),
                    name,
                })),
                _ => None,
            },
        },
    },
    Command {
        id: "page.trash",
        title: "Move page to trash",
        keywords: "delete remove archive",
        category: Category::Page,
        icon: IconName::Trash,
        shortcut: Shortcut::global("cmd-shift-backspace"),
        menu: Some(MenuName::File),
        effect: Effect::Edits,
        is_in_page_menu: true,
        os_action: None,
        is_available: has_page,
        run: Run::Dispatch {
            to: Surface::Workspace,
            action: |target| {
                Box::new(PageCommand::Trash {
                    page: target.page_or_current(),
                })
            },
        },
    },
];
