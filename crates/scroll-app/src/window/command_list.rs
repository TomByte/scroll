//! Every feature's commands in one list, in the order menus and the palette show them.

use std::sync::LazyLock;

use crate::commands::Command;

static ALL: LazyLock<Vec<&'static Command>> = LazyLock::new(|| {
    [
        crate::page::commands::COMMANDS,
        super::commands::NAVIGATION,
        crate::palette::commands::COMMANDS,
        crate::editor::commands::COMMANDS,
        crate::database::commands::COMMANDS,
        crate::accounts::commands::COMMANDS,
        crate::mcp::COMMANDS,
        crate::settings::COMMANDS,
        super::commands::VIEW,
    ]
    .into_iter()
    .flatten()
    .collect()
});

pub fn all() -> &'static [&'static Command] {
    &ALL
}

/// Commands usable right now.
#[cfg(test)]
fn available(
    situation: &crate::commands::Situation,
) -> impl Iterator<Item = &'static Command> + '_ {
    all()
        .iter()
        .copied()
        .filter(move |command| command.is_usable(situation))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::commands::{PageKind, Shortcut, Situation};

    #[test]
    fn every_available_command_can_be_found_in_the_palette_by_its_title() {
        let situations = [
            Situation::default(),
            Situation {
                page: Some(PageKind::Document),
                can_edit: true,
                has_table: false,
                is_workspace_linked: true,
                signed_in_servers: 1,
                known_servers: 1,
                can_go_back: true,
                can_go_forward: true,
            },
            Situation {
                page: Some(PageKind::Database),
                ..Situation::default()
            },
            Situation {
                page: Some(PageKind::Row),
                signed_in_servers: 2,
                known_servers: 2,
                ..Situation::default()
            },
        ];
        for situation in situations {
            for command in available(&situation) {
                let found = crate::commands::matching_commands(
                    all(),
                    command.title,
                    &situation,
                    &Default::default(),
                );
                assert!(
                    found.iter().any(|candidate| candidate.id == command.id),
                    "{} is available but the palette does not offer it for its own title",
                    command.id
                );
            }
        }
    }

    /// Every command must be reachable somewhere (the palette in some situation, the menu bar,
    /// or a key) and no two may share an id or a key in one context.
    #[test]
    fn every_command_is_reachable_and_unique() {
        let pages = [
            None,
            Some(PageKind::Document),
            Some(PageKind::Database),
            Some(PageKind::Row),
        ];
        let situations: Vec<Situation> = pages
            .into_iter()
            .flat_map(|page| {
                [false, true].map(|is_linked| Situation {
                    page,
                    can_edit: true,
                    has_table: page == Some(PageKind::Database),
                    is_workspace_linked: is_linked,
                    signed_in_servers: if is_linked { 1 } else { 2 },
                    known_servers: 2,
                    can_go_back: true,
                    can_go_forward: true,
                })
            })
            .collect();
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        for command in all().iter() {
            assert!(
                ids.insert(command.id),
                "two commands are called {}",
                command.id
            );
            if let Some(shortcut) = command.shortcut {
                assert!(
                    keys.insert((shortcut.keys, shortcut.context)),
                    "{} reuses {}",
                    command.id,
                    shortcut.keys
                );
            }
            let is_in_palette = situations.iter().any(|situation| {
                crate::commands::matching_commands(
                    all(),
                    command.title,
                    situation,
                    &Default::default(),
                )
                .iter()
                .any(|found| found.id == command.id)
            });
            let is_reachable =
                is_in_palette || command.menu.is_some() || command.shortcut.is_some();
            assert!(
                is_reachable,
                "{} cannot be reached from the palette, a menu or a key",
                command.id
            );
        }
    }

    #[test]
    fn shortcut_hints_read_like_the_menu_bar() {
        let shortcut = Shortcut::global("cmd-shift-n").unwrap();
        assert_eq!(shortcut.hint(), "⇧⌘N");
        assert_eq!(Shortcut::global("cmd-[").unwrap().hint(), "⌘[");
    }

    #[test]
    fn every_shortcut_binds() {
        if let Err(error) = crate::commands::key_bindings(all()) {
            panic!("{error}");
        }
    }
}
