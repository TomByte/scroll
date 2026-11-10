//! Markdown-style shortcuts recognised while typing.

use std::ops::Range;

use super::rich_text::MarkKind;
use crate::editor::document::BlockKind;

/// A block-start shortcut: everything typed before the caret is `prefix_length` bytes of
/// markup that turns the block into `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockShortcut {
    pub kind: BlockKind,
    pub prefix_length: usize,
}

/// Recognises markup typed at the very start of a paragraph. `text_before_caret` is the
/// whole of the block's text before the caret.
pub fn block_shortcut(text_before_caret: &str) -> Option<BlockShortcut> {
    let kind = match text_before_caret {
        "# " => BlockKind::Heading1,
        "## " => BlockKind::Heading2,
        "### " => BlockKind::Heading3,
        "- " | "* " | "+ " => BlockKind::BulletedListItem,
        "1. " | "1) " => BlockKind::NumberedListItem,
        "[] " | "[ ] " => BlockKind::ToDo,
        "> " | "\" " => BlockKind::Quote,
        "```" => BlockKind::Code,
        "---" => BlockKind::Divider,
        _ => return None,
    };
    Some(BlockShortcut {
        kind,
        prefix_length: text_before_caret.len(),
    })
}

/// Inline markup just closed at the caret, e.g. `**bold**`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineShortcut {
    pub kind: MarkKind,
    /// The whole match including delimiters.
    pub range: Range<usize>,
    pub delimiter_length: usize,
}

impl InlineShortcut {
    /// Where the content sits once both delimiters are removed.
    pub fn content_after_removal(&self) -> Range<usize> {
        self.range.start..self.range.end - 2 * self.delimiter_length
    }
}

const DELIMITERS: [(&str, MarkKind); 5] = [
    ("**", MarkKind::Bold),
    ("~~", MarkKind::Strikethrough),
    ("`", MarkKind::Code),
    ("*", MarkKind::Italic),
    ("_", MarkKind::Italic),
];

/// Recognises a delimiter pair that ends exactly at the end of `text_before_caret`. The
/// content must be non-empty and must not start or end with whitespace, so that a lone
/// asterisk in prose is left alone.
pub fn inline_shortcut(text_before_caret: &str) -> Option<InlineShortcut> {
    DELIMITERS.iter().find_map(|&(delimiter, kind)| {
        let before_close = text_before_caret.strip_suffix(delimiter)?;
        let open = before_close.rfind(delimiter)?;
        let content = &before_close[open + delimiter.len()..];
        let is_single_char_delimiter_inside_double = delimiter.len() == 1
            && (before_close.ends_with(delimiter) || before_close[..open].ends_with(delimiter));
        let is_valid = !content.is_empty()
            && !content.starts_with(char::is_whitespace)
            && !content.ends_with(char::is_whitespace)
            && !is_single_char_delimiter_inside_double;
        is_valid.then_some(InlineShortcut {
            kind,
            range: open..text_before_caret.len(),
            delimiter_length: delimiter.len(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_prefixes_map_to_kinds() {
        assert_eq!(
            block_shortcut("# ").map(|s| s.kind),
            Some(BlockKind::Heading1)
        );
        assert_eq!(
            block_shortcut("### ").map(|s| s.kind),
            Some(BlockKind::Heading3)
        );
        assert_eq!(
            block_shortcut("- ").map(|s| s.kind),
            Some(BlockKind::BulletedListItem)
        );
        assert_eq!(
            block_shortcut("1. ").map(|s| s.kind),
            Some(BlockKind::NumberedListItem)
        );
        assert_eq!(block_shortcut("[] ").map(|s| s.kind), Some(BlockKind::ToDo));
        assert_eq!(block_shortcut("> ").map(|s| s.kind), Some(BlockKind::Quote));
        assert_eq!(block_shortcut("```").map(|s| s.kind), Some(BlockKind::Code));
        assert_eq!(block_shortcut("---").map(|s| s.prefix_length), Some(3));
        assert_eq!(block_shortcut("#"), None);
        assert_eq!(block_shortcut("a # "), None);
    }

    #[test]
    fn inline_pairs_close_at_the_caret() {
        let bold = inline_shortcut("say **hi**").unwrap();
        assert_eq!(
            (bold.kind, bold.range.clone(), bold.delimiter_length),
            (MarkKind::Bold, 4..10, 2)
        );
        assert_eq!(bold.content_after_removal(), 4..6);
        assert_eq!(
            inline_shortcut("an *aside*").map(|s| s.kind),
            Some(MarkKind::Italic)
        );
        assert_eq!(
            inline_shortcut("run `ls`").map(|s| s.kind),
            Some(MarkKind::Code)
        );
        assert_eq!(
            inline_shortcut("~~old~~").map(|s| s.kind),
            Some(MarkKind::Strikethrough)
        );
    }

    #[test]
    fn stray_delimiters_are_left_alone() {
        assert_eq!(inline_shortcut("2 * 3 *"), None);
        assert_eq!(inline_shortcut("**"), None);
        assert_eq!(inline_shortcut("a ** b*"), None);
        assert_eq!(inline_shortcut("**half*"), None);
        assert_eq!(inline_shortcut("plain"), None);
    }
}
