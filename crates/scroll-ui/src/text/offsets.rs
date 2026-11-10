//! Offset arithmetic for text editing. All offsets are UTF-8 byte offsets into `text` unless
//! the function name says otherwise; the platform input system speaks UTF-16 and CRDTs such as
//! Loro speak Unicode scalar values, so conversions to both live here.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

pub fn utf8_to_utf16(text: &str, utf8_offset: usize) -> usize {
    text[..floor_char_boundary(text, utf8_offset)]
        .chars()
        .map(char::len_utf16)
        .sum()
}

/// Offsets that fall inside a surrogate pair round down to the start of that character.
pub fn utf16_to_utf8(text: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0;
    for (utf8_index, character) in text.char_indices() {
        if utf16_count + character.len_utf16() > utf16_offset {
            return utf8_index;
        }
        utf16_count += character.len_utf16();
    }
    text.len()
}

pub fn utf8_range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end)
}

pub fn utf16_range_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_to_utf8(text, range.start)..utf16_to_utf8(text, range.end)
}

/// Number of Unicode scalar values before `utf8_offset`.
#[cfg(test)]
pub fn utf8_to_char_index(text: &str, utf8_offset: usize) -> usize {
    text[..floor_char_boundary(text, utf8_offset)]
        .chars()
        .count()
}

#[cfg(test)]
pub fn char_index_to_utf8(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(utf8_index, _)| utf8_index)
}

pub fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(index, _)| (index < offset).then_some(index))
        .unwrap_or(0)
}

pub fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(index, _)| (index > offset).then_some(index))
        .unwrap_or(text.len())
}

/// Where Option+Left lands on macOS: the start of the nearest word before `offset`,
/// skipping any whitespace and punctuation in between.
pub fn previous_word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .split_word_bound_indices()
        .rev()
        .find(|(_, segment)| is_word(segment))
        .map_or(0, |(index, _)| index)
}

/// Where Option+Right lands on macOS: the end of the nearest word after `offset`.
pub fn next_word_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .split_word_bound_indices()
        .find(|(_, segment)| is_word(segment))
        .map_or(text.len(), |(index, segment)| {
            offset + index + segment.len()
        })
}

/// The word (or run of whitespace/punctuation) under `offset`, as selected by a double-click.
pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let mut segments = text.split_word_bound_indices().peekable();
    while let Some((start, segment)) = segments.next() {
        let end = start + segment.len();
        let is_last = segments.peek().is_none();
        if offset < end || is_last {
            return start..end;
        }
    }
    offset..offset
}

pub fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &str = "a😀é中b";

    #[test]
    fn utf16_round_trips_through_astral_characters() {
        // a(1) 😀(4 bytes, 2 units) é(2 bytes, 1 unit) 中(3 bytes, 1 unit) b
        assert_eq!(utf8_to_utf16(MIXED, 0), 0);
        assert_eq!(utf8_to_utf16(MIXED, 1), 1);
        assert_eq!(utf8_to_utf16(MIXED, 5), 3);
        assert_eq!(utf8_to_utf16(MIXED, 7), 4);
        assert_eq!(utf8_to_utf16(MIXED, MIXED.len()), 6);
        for utf8_offset in [0, 1, 5, 7, 10, 11] {
            assert_eq!(
                utf16_to_utf8(MIXED, utf8_to_utf16(MIXED, utf8_offset)),
                utf8_offset
            );
        }
    }

    #[test]
    fn utf16_offset_inside_surrogate_pair_rounds_down() {
        assert_eq!(utf16_to_utf8(MIXED, 2), 1);
    }

    #[test]
    fn utf16_offset_past_end_clamps() {
        assert_eq!(utf16_to_utf8(MIXED, 100), MIXED.len());
    }

    #[test]
    fn char_indices_round_trip() {
        assert_eq!(utf8_to_char_index(MIXED, 5), 2);
        assert_eq!(char_index_to_utf8(MIXED, 2), 5);
        assert_eq!(char_index_to_utf8(MIXED, 5), MIXED.len());
        assert_eq!(char_index_to_utf8(MIXED, 50), MIXED.len());
    }

    #[test]
    fn grapheme_boundaries_keep_combined_emoji_together() {
        let family = "x👨‍👩‍👧y";
        let after_x = 1;
        let before_y = family.len() - 1;
        assert_eq!(next_grapheme_boundary(family, after_x), before_y);
        assert_eq!(previous_grapheme_boundary(family, before_y), after_x);
        assert_eq!(previous_grapheme_boundary(family, 0), 0);
        assert_eq!(next_grapheme_boundary(family, family.len()), family.len());
    }

    #[test]
    fn word_movement_skips_punctuation_like_macos() {
        let text = "hello, brave  world";
        assert_eq!(previous_word_start(text, text.len()), 14);
        assert_eq!(previous_word_start(text, 14), 7);
        assert_eq!(previous_word_start(text, 9), 7);
        assert_eq!(previous_word_start(text, 3), 0);
        assert_eq!(next_word_end(text, 0), 5);
        assert_eq!(next_word_end(text, 5), 12);
        assert_eq!(next_word_end(text, 14), text.len());
        assert_eq!(next_word_end(text, text.len()), text.len());
    }

    #[test]
    fn word_range_selects_word_or_gap() {
        let text = "one two";
        assert_eq!(word_range_at(text, 1), 0..3);
        assert_eq!(word_range_at(text, 3), 3..4);
        assert_eq!(word_range_at(text, 7), 4..7);
        assert_eq!(word_range_at("", 0), 0..0);
    }
}
