//! Scoring how well typed text matches a title, for ranking palette results.

/// Higher is better; `None` when `query`'s characters do not all appear in `text` in order.
/// A match at the very start counts most, then matches at word starts, then runs of
/// consecutive characters; gaps cost a little.
pub fn fuzzy_score(query: &str, text: &str) -> Option<u32> {
    let query: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score: i64 = 0;
    let mut next = 0;
    let mut previous_match: Option<usize> = None;
    for (index, character) in text.iter().enumerate() {
        if next == query.len() {
            break;
        }
        if *character != query[next] {
            continue;
        }
        let is_word_start = index == 0 || !text[index - 1].is_alphanumeric();
        score += 1;
        if is_word_start {
            score += 8;
        }
        if index == 0 {
            score += 10;
        }
        match previous_match {
            Some(previous) if previous + 1 == index => score += 6,
            Some(previous) => score -= ((index - previous) as i64).min(8),
            None => {}
        }
        previous_match = Some(index);
        next += 1;
    }
    (next == query.len()).then(|| score.max(1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_beats_word_start_beats_scattered() {
        let prefix = fuzzy_score("new", "New page").unwrap();
        let word_start = fuzzy_score("page", "New page").unwrap();
        let scattered = fuzzy_score("npg", "New page").unwrap();
        assert!(prefix > scattered);
        assert!(word_start > scattered);
        assert!(fuzzy_score("np", "New page").unwrap() > fuzzy_score("ep", "New page").unwrap());
        assert_eq!(fuzzy_score("xyz", "New page"), None);
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }
}
