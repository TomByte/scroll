//! Rich text inside a block: inserting and deleting, marks, and cursors.

use std::ops::Range;

use loro::cursor::{Cursor, PosType, Side};
use loro::{ExpandType, LoroText, LoroValue, TextDelta};

use super::events::block_in_path;
use super::{BlockId, PageDoc, PageDocError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mark {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkKind {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link,
}

impl MarkKind {
    pub(super) const ALL: [MarkKind; 5] = [
        MarkKind::Bold,
        MarkKind::Italic,
        MarkKind::Strikethrough,
        MarkKind::InlineCode,
        MarkKind::Link,
    ];

    /// Stored in the document, so these strings must never change.
    pub(super) fn key(self) -> &'static str {
        match self {
            MarkKind::Bold => "bold",
            MarkKind::Italic => "italic",
            MarkKind::Strikethrough => "strikethrough",
            MarkKind::InlineCode => "code",
            MarkKind::Link => "link",
        }
    }

    /// Typing at the end of bold text stays bold; typing at the end of a link or inline code
    /// leaves it, so there is a way to keep writing plain text after one.
    pub(super) fn expand(self) -> ExpandType {
        match self {
            MarkKind::Link | MarkKind::InlineCode => ExpandType::None,
            MarkKind::Bold | MarkKind::Italic | MarkKind::Strikethrough => ExpandType::After,
        }
    }
}

impl Mark {
    pub fn kind(&self) -> MarkKind {
        match self {
            Mark::Bold => MarkKind::Bold,
            Mark::Italic => MarkKind::Italic,
            Mark::Strikethrough => MarkKind::Strikethrough,
            Mark::InlineCode => MarkKind::InlineCode,
            Mark::Link(_) => MarkKind::Link,
        }
    }

    fn value(&self) -> LoroValue {
        match self {
            Mark::Link(url) => LoroValue::from(url.as_str()),
            _ => LoroValue::Bool(true),
        }
    }

    /// Removed marks read back as null attributes; those are not marks.
    fn from_attribute(key: &str, value: &LoroValue) -> Option<Mark> {
        let kind = MarkKind::ALL.into_iter().find(|kind| kind.key() == key)?;
        match (kind, value) {
            (MarkKind::Link, LoroValue::String(url)) => Some(Mark::Link(url.to_string())),
            (MarkKind::Bold, LoroValue::Bool(true)) => Some(Mark::Bold),
            (MarkKind::Italic, LoroValue::Bool(true)) => Some(Mark::Italic),
            (MarkKind::Strikethrough, LoroValue::Bool(true)) => Some(Mark::Strikethrough),
            (MarkKind::InlineCode, LoroValue::Bool(true)) => Some(Mark::InlineCode),
            _ => None,
        }
    }
}

/// A run of text with the same marks. Spans cover the whole text, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    pub range: Range<usize>,
    pub marks: Vec<Mark>,
}

impl PageDoc {
    pub fn insert_text(&self, id: BlockId, offset: usize, text: &str) -> Result<(), PageDocError> {
        let block_text = self.block_text(id)?;
        check_offset(&block_text, id, offset)?;
        block_text.insert_utf8(offset, text)?;
        self.doc.commit();
        Ok(())
    }

    pub fn delete_text(&self, id: BlockId, range: Range<usize>) -> Result<(), PageDocError> {
        let block_text = self.block_text(id)?;
        check_range(&block_text, id, &range)?;
        block_text.delete_utf8(range.start, range.len())?;
        self.doc.commit();
        Ok(())
    }

    pub fn apply_mark(
        &self,
        id: BlockId,
        range: Range<usize>,
        mark: &Mark,
    ) -> Result<(), PageDocError> {
        let block_text = self.block_text(id)?;
        check_range(&block_text, id, &range)?;
        block_text.mark_utf8(range, mark.kind().key(), mark.value())?;
        self.doc.commit();
        Ok(())
    }

    pub fn remove_mark(
        &self,
        id: BlockId,
        range: Range<usize>,
        kind: MarkKind,
    ) -> Result<(), PageDocError> {
        let block_text = self.block_text(id)?;
        // Loro has no UTF-8 variant of unmark.
        let start = to_unicode_offset(&block_text, id, range.start)?;
        let end = to_unicode_offset(&block_text, id, range.end)?;
        block_text.unmark(start..end, kind.key())?;
        self.doc.commit();
        Ok(())
    }

    /// A position in a block's text that stays attached to the same characters as others edit.
    pub fn cursor(&self, id: BlockId, offset: usize, side: Side) -> Result<Cursor, PageDocError> {
        let block_text = self.block_text(id)?;
        let unicode_offset = to_unicode_offset(&block_text, id, offset)?;
        block_text
            .get_cursor(unicode_offset, side)
            .ok_or(PageDocError::InvalidOffset { block: id, offset })
    }

    /// The block a cursor points into and its byte offset there now, wherever the cursor was made.
    pub fn cursor_position(&self, cursor: &Cursor) -> Result<(BlockId, usize), PageDocError> {
        let path = self
            .doc
            .get_path_to_container(&cursor.container)
            .ok_or(PageDocError::CursorNotInPage)?;
        let block = block_in_path(&path).ok_or(PageDocError::CursorNotInPage)?;
        let block_text = self.block_text(block)?;
        let unicode_offset = self.doc.get_cursor_pos(cursor)?.current.pos;
        let offset = block_text
            .convert_pos(unicode_offset, PosType::Unicode, PosType::Bytes)
            .ok_or(PageDocError::InvalidOffset {
                block,
                offset: unicode_offset,
            })?;
        Ok((block, offset))
    }
}

pub(super) fn read_spans(text: &LoroText) -> Vec<TextSpan> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for delta in text.to_delta() {
        let TextDelta::Insert { insert, attributes } = delta else {
            continue;
        };
        let mut marks: Vec<Mark> = attributes
            .iter()
            .flatten()
            .filter_map(|(key, value)| Mark::from_attribute(key, value))
            .collect();
        // Attributes come from a hash map; sorting keeps the order stable across peers.
        marks.sort();
        let end = offset + insert.len();
        spans.push(TextSpan {
            range: offset..end,
            marks,
        });
        offset = end;
    }
    spans
}

pub(super) fn check_offset(
    text: &LoroText,
    block: BlockId,
    offset: usize,
) -> Result<(), PageDocError> {
    to_unicode_offset(text, block, offset).map(|_| ())
}

pub(super) fn check_range(
    text: &LoroText,
    block: BlockId,
    range: &Range<usize>,
) -> Result<(), PageDocError> {
    check_offset(text, block, range.start)?;
    check_offset(text, block, range.end)?;
    if range.start > range.end {
        return Err(PageDocError::InvalidOffset {
            block,
            offset: range.start,
        });
    }
    Ok(())
}

pub(super) fn to_unicode_offset(
    text: &LoroText,
    block: BlockId,
    offset: usize,
) -> Result<usize, PageDocError> {
    // Loro rounds an offset inside a character down instead of failing, and marks such ranges as given.
    if !text.to_string().is_char_boundary(offset) {
        return Err(PageDocError::InvalidOffset { block, offset });
    }
    text.convert_pos(offset, PosType::Bytes, PosType::Unicode)
        .ok_or(PageDocError::InvalidOffset { block, offset })
}
