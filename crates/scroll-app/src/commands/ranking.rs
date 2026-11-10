//! Ranking commands for the palette: how well each matches what was typed, then how often it
//! has been used.

use std::collections::HashMap;

use super::{Command, Situation, fuzzy_score};

/// How often each command has been run, for ranking.
pub type CommandUsage = HashMap<&'static str, u32>;

/// Available commands whose title or keywords match `query`, best first: how well they match,
/// then how often they have been used.
pub fn matching_commands(
    commands: &[&'static Command],
    query: &str,
    situation: &Situation,
    usage: &CommandUsage,
) -> Vec<&'static Command> {
    let mut ranked: Vec<(u32, &'static Command)> = commands
        .iter()
        .copied()
        .filter(|command| command.is_usable(situation))
        .filter_map(|command| {
            let title_score = fuzzy_score(query, command.title);
            let keyword_score = command
                .keywords
                .split_whitespace()
                .filter_map(|keyword| fuzzy_score(query, keyword))
                .max()
                .map(|score| score / 2);
            let score = title_score.max(keyword_score)?;
            let uses = usage.get(command.id).copied().unwrap_or(0).min(20);
            Some((score + uses, command))
        })
        .collect();
    ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    ranked.into_iter().map(|(_, command)| command).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::command_list::all;

    #[test]
    fn commands_rank_by_match_then_use() {
        let situation = Situation {
            can_edit: true,
            page: Some(crate::commands::PageKind::Document),
            ..Situation::default()
        };
        let found = matching_commands(all(), "new", &situation, &CommandUsage::new());
        assert_eq!(found[0].id, "page.new");
        let mut usage = CommandUsage::new();
        usage.insert("workspace.new", 20);
        let found = matching_commands(all(), "new", &situation, &usage);
        assert_eq!(found[0].id, "workspace.new");
        assert!(
            matching_commands(all(), "bold", &Situation::default(), &CommandUsage::new())
                .is_empty()
        );
    }
}
