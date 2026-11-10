//! The commands offered by the slash menu, and how typing filters them.

use scroll_ui::IconName;

use crate::editor::document::BlockKind;

/// What choosing a slash command does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashAction {
    TurnInto(BlockKind),
    /// Makes a database page inside this page and opens it.
    CreateDatabase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashCommand {
    pub label: &'static str,
    pub hint: &'static str,
    pub action: SlashAction,
    pub icon: IconName,
    /// Extra words that match, so "/h1" or "/check" find what you mean.
    pub aliases: &'static [&'static str],
}

pub const SLASH_COMMANDS: [SlashCommand; 12] = [
    command(
        "Text",
        "Plain paragraph",
        BlockKind::Paragraph,
        IconName::Text,
        &["paragraph", "plain"],
    ),
    command(
        "Heading 1",
        "Large section heading",
        BlockKind::Heading1,
        IconName::Heading,
        &["h1", "title"],
    ),
    command(
        "Heading 2",
        "Medium section heading",
        BlockKind::Heading2,
        IconName::Heading,
        &["h2"],
    ),
    command(
        "Heading 3",
        "Small section heading",
        BlockKind::Heading3,
        IconName::Heading,
        &["h3"],
    ),
    command(
        "To-do list",
        "Track tasks with a checkbox",
        BlockKind::ToDo,
        IconName::CheckSquare,
        &["todo", "task", "check"],
    ),
    command(
        "Bulleted list",
        "A simple list",
        BlockKind::BulletedListItem,
        IconName::List,
        &["bullet", "ul"],
    ),
    command(
        "Numbered list",
        "A list with numbers",
        BlockKind::NumberedListItem,
        IconName::ListNumbered,
        &["number", "ol", "ordered"],
    ),
    command(
        "Quote",
        "Capture a quotation",
        BlockKind::Quote,
        IconName::Quote,
        &["blockquote"],
    ),
    command(
        "Code",
        "A block of code",
        BlockKind::Code,
        IconName::Code,
        &["snippet", "pre"],
    ),
    command(
        "Callout",
        "Make writing stand out",
        BlockKind::Callout,
        IconName::Callout,
        &["note", "info"],
    ),
    SlashCommand {
        label: "Database",
        hint: "A table of pages",
        action: SlashAction::CreateDatabase,
        icon: IconName::Table,
        aliases: &["table", "db"],
    },
    command(
        "Divider",
        "Separate sections",
        BlockKind::Divider,
        IconName::Minus,
        &["line", "hr", "separator"],
    ),
];

const fn command(
    label: &'static str,
    hint: &'static str,
    kind: BlockKind,
    icon: IconName,
    aliases: &'static [&'static str],
) -> SlashCommand {
    SlashCommand {
        label,
        hint,
        action: SlashAction::TurnInto(kind),
        icon,
        aliases,
    }
}

/// Commands whose label or an alias contains every word of `query`, labels starting with
/// the query first.
pub fn matching_commands(query: &str) -> Vec<SlashCommand> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut matches: Vec<SlashCommand> = SLASH_COMMANDS
        .into_iter()
        .filter(|command| {
            let label = command.label.to_lowercase();
            words.iter().all(|word| {
                label.contains(word) || command.aliases.iter().any(|alias| alias.starts_with(word))
            })
        })
        .collect();
    matches.sort_by_key(|command| !command.label.to_lowercase().starts_with(&query));
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    impl SlashCommand {
        fn kind(&self) -> Option<BlockKind> {
            match self.action {
                SlashAction::TurnInto(kind) => Some(kind),
                SlashAction::CreateDatabase => None,
            }
        }
    }

    #[test]
    fn empty_query_lists_everything_in_order() {
        assert_eq!(matching_commands("").len(), SLASH_COMMANDS.len());
        assert_eq!(matching_commands("")[0].kind(), Some(BlockKind::Paragraph));
    }

    #[test]
    fn labels_and_aliases_match() {
        let kinds = |query| {
            matching_commands(query)
                .into_iter()
                .filter_map(|c| c.kind())
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds("h2"), vec![BlockKind::Heading2]);
        assert_eq!(kinds("todo"), vec![BlockKind::ToDo]);
        assert_eq!(
            kinds("list"),
            vec![
                BlockKind::ToDo,
                BlockKind::BulletedListItem,
                BlockKind::NumberedListItem
            ]
        );
        assert!(kinds("quo").contains(&BlockKind::Quote));
        assert!(kinds("zzz").is_empty());
    }

    #[test]
    fn prefix_matches_come_first() {
        assert_eq!(matching_commands("co")[0].kind(), Some(BlockKind::Code));
        assert_eq!(
            matching_commands("data")[0].action,
            SlashAction::CreateDatabase
        );
    }
}
