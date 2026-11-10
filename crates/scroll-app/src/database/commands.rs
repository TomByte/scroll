//! The database commands: views, rows, sort, filter and properties of the open database.

use gpui::App;
use scroll_core::id::{PropertyId, ViewId};

use scroll_ui::IconName;

use crate::commands::available::in_database;
use crate::commands::{
    Category, Choice, ChoiceValue, Choices, Command, Effect, Run, Shortcut, Surface, Target,
};
use crate::workspace::Workspace;

/// What a database command does; the open database view carries it out.
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub enum DatabaseCommand {
    AddView,
    AddRow,
    Sort,
    Filter,
    Properties,
    EditProperty(PropertyId),
    SwitchView(ViewId),
    RenameView(ViewId),
    DeleteView(ViewId),
}

fn database_properties(target: &Target, cx: &App) -> Vec<Choice> {
    let Some(workspace) = Workspace::current(cx) else {
        return Vec::new();
    };
    let records = workspace.read(cx).records();
    target
        .current
        .and_then(|page| records.database_hosted_by(page))
        .map(|database| {
            database
                .properties
                .iter()
                .map(|property| Choice {
                    label: property.name.clone().into(),
                    emoji: None,
                    value: ChoiceValue::Property(property.id),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn database_views(target: &Target, cx: &App) -> Vec<Choice> {
    let Some(workspace) = Workspace::current(cx) else {
        return Vec::new();
    };
    let records = workspace.read(cx).records();
    let Some(database) = target
        .current
        .and_then(|page| records.database_hosted_by(page))
    else {
        return Vec::new();
    };
    records
        .views_of(database.id)
        .into_iter()
        .map(|view| Choice {
            label: view.name.clone().into(),
            emoji: None,
            value: ChoiceValue::View(view.id),
        })
        .collect()
}

pub static COMMANDS: &[Command] = &[
    Command {
        id: "database.add_view",
        title: "Add a view",
        keywords: "table tab",
        category: Category::Database,
        icon: IconName::Plus,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Dispatch {
            to: Surface::Database,
            action: |_| Box::new(DatabaseCommand::AddView),
        },
    },
    Command {
        id: "database.add_row",
        title: "Add row",
        keywords: "new row record",
        category: Category::Database,
        icon: IconName::Plus,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: |situation| situation.has_table,
        run: Run::Dispatch {
            to: Surface::Database,
            action: |_| Box::new(DatabaseCommand::AddRow),
        },
    },
    Command {
        id: "database.sort",
        title: "Sort rows",
        keywords: "order arrange",
        category: Category::Database,
        icon: IconName::ArrowRight,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Dispatch {
            to: Surface::Database,
            action: |_| Box::new(DatabaseCommand::Sort),
        },
    },
    Command {
        id: "database.filter",
        title: "Filter rows",
        keywords: "where show only",
        category: Category::Database,
        icon: IconName::Search,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Dispatch {
            to: Surface::Database,
            action: |_| Box::new(DatabaseCommand::Filter),
        },
    },
    Command {
        id: "database.properties",
        title: "Show, hide or add properties",
        keywords: "columns fields add property",
        category: Category::Database,
        icon: IconName::List,
        shortcut: None,
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Dispatch {
            to: Surface::Database,
            action: |_| Box::new(DatabaseCommand::Properties),
        },
    },
    Command {
        id: "database.edit_property",
        title: "Edit property",
        keywords: "rename column type delete property hide",
        category: Category::Database,
        icon: IconName::Pencil,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Choose {
            prompt: "Edit which property?",
            choices: Choices::Fixed(database_properties),
            to: Surface::Database,
            pick: |_, value| match value {
                ChoiceValue::Property(property) => {
                    Some(Box::new(DatabaseCommand::EditProperty(property)))
                }
                _ => None,
            },
        },
    },
    Command {
        id: "database.switch_view",
        title: "Switch view",
        keywords: "tab table",
        category: Category::Database,
        icon: IconName::Table,
        shortcut: None,
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Choose {
            prompt: "Show which view?",
            choices: Choices::Fixed(database_views),
            to: Surface::Database,
            pick: |_, value| match value {
                ChoiceValue::View(view) => Some(Box::new(DatabaseCommand::SwitchView(view))),
                _ => None,
            },
        },
    },
    Command {
        id: "database.rename_view",
        title: "Rename view",
        keywords: "tab",
        category: Category::Database,
        icon: IconName::Pencil,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Choose {
            prompt: "Rename which view?",
            choices: Choices::Fixed(database_views),
            to: Surface::Database,
            pick: |_, value| match value {
                ChoiceValue::View(view) => Some(Box::new(DatabaseCommand::RenameView(view))),
                _ => None,
            },
        },
    },
    Command {
        id: "database.delete_view",
        title: "Delete view",
        keywords: "remove tab",
        category: Category::Database,
        icon: IconName::Trash,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Choose {
            prompt: "Delete which view? (the last one stays)",
            choices: Choices::Fixed(database_views),
            to: Surface::Database,
            pick: |_, value| match value {
                ChoiceValue::View(view) => Some(Box::new(DatabaseCommand::DeleteView(view))),
                _ => None,
            },
        },
    },
    Command {
        id: "database.open_row",
        title: "Open row as a page",
        keywords: "expand",
        category: Category::Database,
        icon: IconName::Page,
        shortcut: Shortcut::in_context(scroll_table::OpenRow::KEYS[0], scroll_table::KEY_CONTEXT),
        menu: None,
        effect: Effect::Reads,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Dispatch {
            to: Surface::Focused,
            action: |_| Box::new(scroll_table::OpenRow),
        },
    },
    Command {
        id: "database.delete_rows",
        title: "Delete selected rows",
        keywords: "remove",
        category: Category::Database,
        icon: IconName::Trash,
        shortcut: None,
        menu: None,
        effect: Effect::Edits,
        is_in_page_menu: false,
        os_action: None,
        is_available: in_database,
        run: Run::Dispatch {
            to: Surface::Focused,
            action: |_| Box::new(scroll_table::DeleteRows),
        },
    },
];
