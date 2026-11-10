//! The emoji the picker offers, read once from a table generated from Unicode's data (see
//! `tools/emoji_table.py`), name and keyword search, and the recently used list.

use std::sync::OnceLock;

use gpui::{App, Global};

const TABLE: &str = include_str!("../../assets/emoji/emoji.tsv");
const RECENT_LIMIT: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Emoji {
    pub emoji: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    /// Unicode's subgroup, such as "face-smiling"; searched as weaker keywords.
    pub subgroup: &'static str,
    /// CLDR's search keywords, such as "happy laugh", separated by spaces.
    pub keywords: &'static str,
}

/// Every emoji, in Unicode's order (which is grouped by category).
pub(crate) fn all_emoji() -> &'static [Emoji] {
    static EMOJI: OnceLock<Vec<Emoji>> = OnceLock::new();
    EMOJI.get_or_init(|| {
        TABLE
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut columns = line.split('\t');
                Some(Emoji {
                    emoji: columns.next()?,
                    name: columns.next()?,
                    group: columns.next()?,
                    subgroup: columns.next()?,
                    keywords: columns.next()?,
                })
            })
            .collect()
    })
}

/// The emoji named exactly `name` (as in `:name:`, with `_` or `-` for spaces).
pub fn emoji_named(name: &str) -> Option<&'static Emoji> {
    let wanted = name.replace(['_', '-'], " ").to_lowercase();
    all_emoji().iter().find(|emoji| emoji.name == wanted)
}

/// Emoji whose name or keywords contain a word starting with each word of `query`, best first:
/// names starting with the query, then names with a word starting with each query word, then
/// CLDR keyword matches, then matches that need Unicode's subgroup.
pub fn search_emoji(query: &str) -> Vec<&'static Emoji> {
    let query = query.trim().to_lowercase().replace(['_', '-'], " ");
    if query.is_empty() {
        return all_emoji().iter().collect();
    }
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut ranked: Vec<(Match, usize, &'static Emoji)> = all_emoji()
        .iter()
        .enumerate()
        .filter_map(|(order, emoji)| Some((rank(emoji, &query, &words)?, order, emoji)))
        .collect();
    ranked.sort_by_key(|(rank, order, _)| (*rank, *order));
    ranked.into_iter().map(|(_, _, emoji)| emoji).collect()
}

/// How well an emoji matches a query; earlier variants are better.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Match {
    NamePrefix,
    NameWords,
    Keywords,
    Subgroup,
}

fn rank(emoji: &Emoji, query: &str, words: &[&str]) -> Option<Match> {
    // Queries are lowercased; some names, such as "flag: United Kingdom", are not.
    let name = emoji.name.to_lowercase();
    let name_words: Vec<&str> = name.split([' ', '-', ':', ',']).collect();
    let keyword_words: Vec<&str> = emoji.keywords.split_whitespace().collect();
    let subgroup_words: Vec<&str> = emoji.subgroup.split('-').collect();
    let is_in_name = |word: &&str| has_word_starting(&name_words, word);
    let is_in_keywords = |word: &&str| is_in_name(word) || has_word_starting(&keyword_words, word);
    let is_in_subgroup =
        |word: &&str| is_in_keywords(word) || has_word_starting(&subgroup_words, word);
    if name.starts_with(query) {
        Some(Match::NamePrefix)
    } else if words.iter().all(is_in_name) {
        Some(Match::NameWords)
    } else if words.iter().all(is_in_keywords) {
        Some(Match::Keywords)
    } else if words.iter().all(is_in_subgroup) {
        Some(Match::Subgroup)
    } else {
        None
    }
}

fn has_word_starting(candidates: &[&str], prefix: &str) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.starts_with(prefix))
}

/// Emoji picked lately, most recent first. Lives for the run of the app.
#[derive(Default)]
struct RecentEmoji(Vec<String>);

impl Global for RecentEmoji {}

pub(crate) fn recent_emoji(cx: &App) -> Vec<String> {
    cx.try_global::<RecentEmoji>()
        .map(|recent| recent.0.clone())
        .unwrap_or_default()
}

pub fn remember_emoji(emoji: &str, cx: &mut App) {
    let mut recent = recent_emoji(cx);
    recent.retain(|existing| existing != emoji);
    recent.insert(0, emoji.to_string());
    recent.truncate(RECENT_LIMIT);
    cx.set_global(RecentEmoji(recent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_loads_every_group_once_per_emoji() {
        let emoji = all_emoji();
        assert!(emoji.len() > 1800, "{}", emoji.len());
        assert_eq!(emoji[0].emoji, "😀");
        assert!(emoji.iter().any(|emoji| emoji.group == "Flags"));
        assert!(emoji.iter().any(|emoji| emoji.emoji == "👨‍👩‍👧‍👦"));
    }

    #[test]
    fn search_ranks_name_prefixes_first_and_matches_keywords() {
        let results = search_emoji("thumbs");
        assert_eq!(results[0].name, "thumbs up");
        let smiling = search_emoji("smiling");
        assert!(smiling.len() > 10);
        assert!(
            search_emoji("face smiling")
                .iter()
                .any(|emoji| emoji.emoji == "😀")
        );
        assert!(search_emoji("zzzqqq").is_empty());
        assert_eq!(
            emoji_named("red_heart").map(|emoji| emoji.emoji),
            Some("❤️")
        );
    }

    fn finds(query: &str, wanted: &str) -> bool {
        search_emoji(query)
            .iter()
            .any(|emoji| emoji.emoji == wanted)
    }

    #[test]
    fn search_matches_cldr_keywords() {
        assert!(finds("happy", "😀"));
        assert!(finds("laugh", "😂"));
        assert!(finds("lol", "😂"));
        assert!(finds("heart", "❤️"));
        assert!(finds("love", "❤️"));
        assert!(finds("flag united kingdom", "🇬🇧"));
    }

    fn rank_of(wanted: &str, query: &str) -> Option<Match> {
        let emoji = all_emoji().iter().find(|emoji| emoji.emoji == wanted)?;
        let words: Vec<&str> = query.split_whitespace().collect();
        rank(emoji, query, &words)
    }

    #[test]
    fn search_ranks_names_above_keywords_above_subgroups() {
        assert_eq!(rank_of("👍", "thumbs"), Some(Match::NamePrefix));
        assert_eq!(rank_of("👍", "up"), Some(Match::NameWords));
        assert_eq!(rank_of("👍", "good"), Some(Match::Keywords));
        assert_eq!(rank_of("👍", "fingers closed"), Some(Match::Subgroup));
        assert_eq!(rank_of("👍", "down"), None);
        let hearts = search_emoji("heart");
        let position = |wanted: &str| hearts.iter().position(|emoji| emoji.emoji == wanted);
        // "heart suit" starts with the query; "smiling face with hearts" only has a word that does.
        assert!(position("♥️") < position("🥰"));
    }

    #[test]
    fn keywords_leave_out_words_already_in_the_name() {
        for emoji in all_emoji() {
            let name = emoji.name.to_lowercase();
            let name_words: Vec<&str> = name.split([' ', '-', ':', ',']).collect();
            let repeated = emoji
                .keywords
                .split_whitespace()
                .find(|keyword| name_words.contains(keyword));
            assert_eq!(repeated, None, "{}", emoji.name);
        }
    }
}
