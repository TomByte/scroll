//! What the editor puts on the clipboard and reads back. Inside the app, copied blocks travel
//! as JSON beside the text, so kinds, nesting, to-do state and marks survive; other apps get
//! Markdown as the text. Text pasted from elsewhere is read as Markdown.
//!
//! A table travels as its block, then each row with its cells nested under it, and is a
//! Markdown table (whose first row is its header) to other apps.

use serde::{Deserialize, Serialize};

use crate::editor::code_language::CodeLanguage;
use crate::editor::document::BlockKind;
use crate::editor::rich_text::{MarkSpan, Marks, RichText};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopiedBlock {
    pub kind: CopiedKind,
    /// Nesting below the shallowest copied block.
    pub depth: usize,
    pub is_checked: bool,
    /// A code block's language id; absent in what older versions copied.
    #[serde(default)]
    pub language: Option<String>,
    pub text: String,
    pub spans: Vec<CopiedSpan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopiedKind {
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    ToDo,
    Bulleted,
    Numbered,
    Quote,
    Code,
    Divider,
    Callout,
    Table,
    TableRow,
    TableCell,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopiedSpan {
    pub start: usize,
    pub end: usize,
    pub is_bold: bool,
    pub is_italic: bool,
    pub is_strikethrough: bool,
    pub is_code: bool,
    pub link: Option<String>,
}

/// Blocks, or part of one block's text (`is_fragment`), which pastes into the line it lands on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copied {
    pub blocks: Vec<CopiedBlock>,
    pub is_fragment: bool,
}

impl CopiedKind {
    pub fn from_block_kind(kind: BlockKind) -> CopiedKind {
        match kind {
            BlockKind::Paragraph => CopiedKind::Paragraph,
            BlockKind::Heading1 => CopiedKind::Heading1,
            BlockKind::Heading2 => CopiedKind::Heading2,
            BlockKind::Heading3 => CopiedKind::Heading3,
            BlockKind::ToDo => CopiedKind::ToDo,
            BlockKind::BulletedListItem => CopiedKind::Bulleted,
            BlockKind::NumberedListItem => CopiedKind::Numbered,
            BlockKind::Quote => CopiedKind::Quote,
            BlockKind::Code => CopiedKind::Code,
            BlockKind::Divider => CopiedKind::Divider,
            BlockKind::Callout => CopiedKind::Callout,
            BlockKind::Table => CopiedKind::Table,
            BlockKind::TableRow => CopiedKind::TableRow,
            BlockKind::TableCell => CopiedKind::TableCell,
        }
    }

    pub fn block_kind(self) -> BlockKind {
        match self {
            CopiedKind::Paragraph => BlockKind::Paragraph,
            CopiedKind::Heading1 => BlockKind::Heading1,
            CopiedKind::Heading2 => BlockKind::Heading2,
            CopiedKind::Heading3 => BlockKind::Heading3,
            CopiedKind::ToDo => BlockKind::ToDo,
            CopiedKind::Bulleted => BlockKind::BulletedListItem,
            CopiedKind::Numbered => BlockKind::NumberedListItem,
            CopiedKind::Quote => BlockKind::Quote,
            CopiedKind::Code => BlockKind::Code,
            CopiedKind::Divider => BlockKind::Divider,
            CopiedKind::Callout => BlockKind::Callout,
            CopiedKind::Table => BlockKind::Table,
            CopiedKind::TableRow => BlockKind::TableRow,
            CopiedKind::TableCell => BlockKind::TableCell,
        }
    }
}

impl CopiedBlock {
    pub fn new(kind: BlockKind, depth: usize, is_checked: bool, content: &RichText) -> CopiedBlock {
        CopiedBlock {
            kind: CopiedKind::from_block_kind(kind),
            depth,
            is_checked,
            language: None,
            text: content.text.clone(),
            spans: content
                .spans
                .iter()
                .filter(|span| span.marks != Marks::default())
                .map(|span| CopiedSpan {
                    start: span.range.start,
                    end: span.range.end,
                    is_bold: span.marks.bold,
                    is_italic: span.marks.italic,
                    is_strikethrough: span.marks.strikethrough,
                    is_code: span.marks.code,
                    link: span.marks.link.clone(),
                })
                .collect(),
        }
    }

    pub fn with_language(self, language: Option<String>) -> CopiedBlock {
        CopiedBlock { language, ..self }
    }

    /// A block of `kind` with no text, such as a table or one of its rows.
    pub fn empty(kind: CopiedKind, depth: usize) -> CopiedBlock {
        CopiedBlock {
            kind,
            depth,
            is_checked: false,
            language: None,
            text: String::new(),
            spans: Vec::new(),
        }
    }

    /// The part of a block's content in `range`, with marks clipped to it.
    pub fn fragment(content: &RichText, range: std::ops::Range<usize>) -> CopiedBlock {
        let mut clipped = RichText::default();
        for MarkSpan { range: span, marks } in &content.spans {
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            if start < end {
                clipped.push(&content.text[start..end], marks.clone());
            }
        }
        CopiedBlock::new(BlockKind::Paragraph, 0, false, &clipped)
    }
}

/// The blocks as Markdown, for other apps.
pub fn to_markdown(copied: &Copied) -> String {
    if copied.is_fragment {
        return copied
            .blocks
            .iter()
            .map(inline_markdown)
            .collect::<Vec<_>>()
            .join("\n");
    }
    let mut lines = Vec::new();
    let mut number = 0;
    let mut blocks = copied.blocks.iter().peekable();
    while let Some(block) = blocks.next() {
        number = if block.kind == CopiedKind::Numbered {
            number + 1
        } else {
            0
        };
        let indent = "  ".repeat(block.depth);
        if block.kind == CopiedKind::Table {
            let parts: Vec<&CopiedBlock> =
                std::iter::from_fn(|| blocks.next_if(|part| part.depth > block.depth)).collect();
            lines.extend(
                table_markdown(&parts)
                    .into_iter()
                    .map(|line| format!("{indent}{line}")),
            );
            continue;
        }
        let text = inline_markdown(block);
        let line = match block.kind {
            // A table's parts copied without their table are just their text.
            CopiedKind::Paragraph
            | CopiedKind::Table
            | CopiedKind::TableRow
            | CopiedKind::TableCell => text,
            CopiedKind::Heading1 => format!("# {text}"),
            CopiedKind::Heading2 => format!("## {text}"),
            CopiedKind::Heading3 => format!("### {text}"),
            CopiedKind::ToDo => format!("- [{}] {text}", if block.is_checked { "x" } else { " " }),
            CopiedKind::Bulleted => format!("- {text}"),
            CopiedKind::Numbered => format!("{number}. {text}"),
            CopiedKind::Quote | CopiedKind::Callout => format!("> {text}"),
            CopiedKind::Code => format!(
                "```{}\n{}\n```",
                block.language.as_deref().unwrap_or_default(),
                block.text
            ),
            CopiedKind::Divider => "---".to_string(),
        };
        lines.push(format!("{indent}{line}"));
    }
    lines.join("\n")
}

/// A table's rows and cells (`parts`, in document order) as the lines of a Markdown table.
/// Markdown needs a header row, so the first row is it.
fn table_markdown(parts: &[&CopiedBlock]) -> Vec<String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for part in parts {
        match part.kind {
            CopiedKind::TableRow => rows.push(Vec::new()),
            _ => {
                if let Some(row) = rows.last_mut() {
                    row.push(cell_markdown(part));
                }
            }
        }
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let line = |cells: &[String]| {
        let mut cells = cells.to_vec();
        cells.resize(width, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut lines: Vec<String> = rows.iter().map(|row| line(row)).collect();
    if !lines.is_empty() {
        lines.insert(1, line(&vec![TABLE_RULE.to_string(); width]));
    }
    lines
}

const TABLE_RULE: &str = "---";

/// A cell's text on one line, with the characters that would end the cell written so they do not.
fn cell_markdown(cell: &CopiedBlock) -> String {
    inline_markdown(cell).replace('|', "\\|").replace('\n', " ")
}

fn inline_markdown(block: &CopiedBlock) -> String {
    let mut out = String::new();
    let mut offset = 0;
    for span in &block.spans {
        out.push_str(&block.text[offset..span.start]);
        let mut piece = block.text[span.start..span.end].to_string();
        if span.is_code {
            piece = format!("`{piece}`");
        }
        if span.is_bold {
            piece = format!("**{piece}**");
        }
        if span.is_italic {
            piece = format!("*{piece}*");
        }
        if span.is_strikethrough {
            piece = format!("~~{piece}~~");
        }
        if let Some(url) = &span.link {
            piece = format!("[{piece}]({url})");
        }
        out.push_str(&piece);
        offset = span.end;
    }
    out.push_str(&block.text[offset..]);
    out
}

/// Reads Markdown (or plain text, which is Markdown without markup) into blocks.
pub fn from_markdown(text: &str) -> Copied {
    let text = text.replace("\r\n", "\n");
    let mut blocks = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let indent = line
            .chars()
            .take_while(|character| *character == ' ')
            .count();
        let trimmed = line.trim_start();
        if is_table_row(trimmed) && lines.peek().is_some_and(|next| is_table_rule(next)) {
            lines.next();
            let rest = std::iter::from_fn(|| lines.next_if(|next| is_table_row(next.trim_start())));
            let rows = std::iter::once(line).chain(rest);
            blocks.extend(table_from_markdown(rows, indent / 2));
            continue;
        }
        if let Some(info) = trimmed.strip_prefix("```") {
            let code: Vec<&str> = lines
                .by_ref()
                .take_while(|line| !line.trim_start().starts_with("```"))
                .collect();
            blocks.push(CopiedBlock {
                kind: CopiedKind::Code,
                depth: 0,
                is_checked: false,
                language: fence_language(info),
                text: code.join("\n"),
                spans: Vec::new(),
            });
            continue;
        }
        let (kind, is_checked, rest) = line_kind(trimmed);
        let mut block = inline_from_markdown(rest);
        block.kind = kind;
        block.is_checked = is_checked;
        block.depth = indent / 2;
        blocks.push(block);
    }
    Copied {
        is_fragment: blocks.len() == 1 && blocks[0].kind == CopiedKind::Paragraph,
        blocks,
    }
}

fn is_table_row(line: &str) -> bool {
    line.len() > 1 && line.starts_with('|') && line.trim_end().ends_with('|')
}

/// The line under a Markdown table's header: only dashes, with optional alignment colons.
fn is_table_rule(line: &str) -> bool {
    let line = line.trim();
    is_table_row(line)
        && table_cells(line).iter().all(|cell| {
            cell.contains('-')
                && cell
                    .chars()
                    .all(|character| character == '-' || character == ':')
        })
}

/// The text of each cell on a Markdown table line, with `\|` read as a bar inside a cell.
fn table_cells(line: &str) -> Vec<String> {
    const ESCAPED_BAR: char = '\u{0}';
    let line = line.trim().replace("\\|", &ESCAPED_BAR.to_string());
    let inner = line.strip_prefix('|').unwrap_or(&line);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .split('|')
        .map(|cell| cell.trim().replace(ESCAPED_BAR, "|"))
        .collect()
}

/// A table block at `depth` followed by a row block per line and a cell block per cell.
fn table_from_markdown<'a>(rows: impl Iterator<Item = &'a str>, depth: usize) -> Vec<CopiedBlock> {
    let mut blocks = vec![CopiedBlock::empty(CopiedKind::Table, depth)];
    for row in rows {
        blocks.push(CopiedBlock::empty(CopiedKind::TableRow, depth + 1));
        for cell in table_cells(row) {
            let mut block = inline_from_markdown(&cell);
            block.kind = CopiedKind::TableCell;
            block.depth = depth + 2;
            blocks.push(block);
        }
    }
    blocks
}

fn line_kind(line: &str) -> (CopiedKind, bool, &str) {
    let prefixes: [(&str, CopiedKind, bool); 10] = [
        ("### ", CopiedKind::Heading3, false),
        ("## ", CopiedKind::Heading2, false),
        ("# ", CopiedKind::Heading1, false),
        ("- [ ] ", CopiedKind::ToDo, false),
        ("- [x] ", CopiedKind::ToDo, true),
        ("- [X] ", CopiedKind::ToDo, true),
        ("- ", CopiedKind::Bulleted, false),
        ("* ", CopiedKind::Bulleted, false),
        ("> ", CopiedKind::Quote, false),
        ("---", CopiedKind::Divider, false),
    ];
    for (prefix, kind, is_checked) in prefixes {
        if let Some(rest) = line.strip_prefix(prefix) {
            return (kind, is_checked, rest);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = line[digits..].strip_prefix(". ")
    {
        return (CopiedKind::Numbered, false, rest);
    }
    (CopiedKind::Paragraph, false, line)
}

/// Turns `**bold**`, `*italic*`, `~~strike~~`, `` `code` `` and `[text](url)` into marks.
/// Markup that does not close is kept as text.
fn inline_from_markdown(line: &str) -> CopiedBlock {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        if let Some((inner, after, mark)) = delimited(rest) {
            let start = text.len();
            text.push_str(inner);
            let mut span = CopiedSpan {
                start,
                end: text.len(),
                ..CopiedSpan::default()
            };
            match mark {
                InlineMark::Bold => span.is_bold = true,
                InlineMark::Italic => span.is_italic = true,
                InlineMark::Strikethrough => span.is_strikethrough = true,
                InlineMark::Code => span.is_code = true,
                InlineMark::Link(url) => span.link = Some(url.to_string()),
            }
            if span.start < span.end {
                spans.push(span);
            }
            rest = after;
            continue;
        }
        let next = rest.chars().next().map_or(1, char::len_utf8);
        text.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    CopiedBlock {
        text,
        spans,
        ..CopiedBlock::empty(CopiedKind::Paragraph, 0)
    }
}

/// The language a code fence names (```` ```rust ````), if it is one code blocks know.
fn fence_language(info: &str) -> Option<String> {
    let name = info.split_whitespace().next()?.to_lowercase();
    CodeLanguage::from_id(Some(&name)).id().map(str::to_string)
}

enum InlineMark<'a> {
    Bold,
    Italic,
    Strikethrough,
    Code,
    Link(&'a str),
}

/// If `text` starts with a closed piece of inline markup: its content, what follows, the mark.
fn delimited(text: &str) -> Option<(&str, &str, InlineMark<'_>)> {
    for (delimiter, mark) in [("**", 0), ("~~", 1), ("`", 2), ("*", 3), ("_", 3)] {
        // As in Markdown, markup hugs its content: `2 * 3 * 4` is not italic.
        if let Some(after_open) = text.strip_prefix(delimiter)
            && let Some(close) = after_open.find(delimiter)
            && close > 0
            && !after_open[..close].starts_with(char::is_whitespace)
            && !after_open[..close].ends_with(char::is_whitespace)
        {
            let mark = match mark {
                0 => InlineMark::Bold,
                1 => InlineMark::Strikethrough,
                2 => InlineMark::Code,
                _ => InlineMark::Italic,
            };
            return Some((
                &after_open[..close],
                &after_open[close + delimiter.len()..],
                mark,
            ));
        }
    }
    let after_open = text.strip_prefix('[')?;
    let close = after_open.find("](")?;
    let after_label = &after_open[close + 2..];
    let url_end = after_label.find(')')?;
    Some((
        &after_open[..close],
        &after_label[url_end + 1..],
        InlineMark::Link(&after_label[..url_end]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_round_trips_kinds_nesting_and_marks() {
        let markdown = "# Plan\n- [x] Done **well**\n  - nested *item*\n1. first\n2. second\n> quote with [link](https://example.com)\n```\nfn main() {}\n```\n---\nplain ~~gone~~ `code`";
        let copied = from_markdown(markdown);
        let kinds: Vec<CopiedKind> = copied.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(
            kinds,
            [
                CopiedKind::Heading1,
                CopiedKind::ToDo,
                CopiedKind::Bulleted,
                CopiedKind::Numbered,
                CopiedKind::Numbered,
                CopiedKind::Quote,
                CopiedKind::Code,
                CopiedKind::Divider,
                CopiedKind::Paragraph,
            ]
        );
        assert!(copied.blocks[1].is_checked);
        assert_eq!(copied.blocks[1].text, "Done well");
        assert!(copied.blocks[1].spans[0].is_bold);
        assert_eq!(copied.blocks[2].depth, 1);
        assert_eq!(
            copied.blocks[5].spans[0].link.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(copied.blocks[6].text, "fn main() {}");
        assert_eq!(to_markdown(&copied), markdown);
    }

    #[test]
    fn a_table_round_trips_as_a_markdown_table() {
        let markdown =
            "before\n| Name | Role |\n| --- | --- |\n| **Ada** | a \\| b |\n| Grace |  |\nafter";
        let copied = from_markdown(markdown);
        let shape: Vec<(CopiedKind, usize, &str)> = copied
            .blocks
            .iter()
            .map(|block| (block.kind, block.depth, block.text.as_str()))
            .collect();
        use CopiedKind::{Paragraph, Table, TableCell, TableRow};
        assert_eq!(
            shape,
            [
                (Paragraph, 0, "before"),
                (Table, 0, ""),
                (TableRow, 1, ""),
                (TableCell, 2, "Name"),
                (TableCell, 2, "Role"),
                (TableRow, 1, ""),
                (TableCell, 2, "Ada"),
                (TableCell, 2, "a | b"),
                (TableRow, 1, ""),
                (TableCell, 2, "Grace"),
                (TableCell, 2, ""),
                (Paragraph, 0, "after"),
            ]
        );
        assert!(copied.blocks[6].spans[0].is_bold, "marks are read in cells");
        assert_eq!(to_markdown(&copied), markdown);
    }

    #[test]
    fn bars_without_a_rule_under_them_are_not_a_table() {
        let copied = from_markdown("| not | a table |\nplain");
        let kinds: Vec<CopiedKind> = copied.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(kinds, [CopiedKind::Paragraph, CopiedKind::Paragraph]);
        assert!(is_table_rule("|:---|---:|"));
        assert!(!is_table_rule("| a | b |"));
    }

    #[test]
    fn a_ragged_table_is_squared_off_and_stray_parts_are_text() {
        let table = Copied {
            is_fragment: false,
            blocks: vec![
                CopiedBlock::empty(CopiedKind::Table, 0),
                CopiedBlock::empty(CopiedKind::TableRow, 1),
                CopiedBlock {
                    text: "a".into(),
                    ..CopiedBlock::empty(CopiedKind::TableCell, 2)
                },
                CopiedBlock {
                    text: "b".into(),
                    ..CopiedBlock::empty(CopiedKind::TableCell, 2)
                },
                CopiedBlock::empty(CopiedKind::TableRow, 1),
                CopiedBlock {
                    text: "c".into(),
                    ..CopiedBlock::empty(CopiedKind::TableCell, 2)
                },
                CopiedBlock {
                    text: "stray".into(),
                    ..CopiedBlock::empty(CopiedKind::TableCell, 0)
                },
            ],
        };
        assert_eq!(
            to_markdown(&table),
            "| a | b |\n| --- | --- |\n| c |  |\nstray"
        );
    }

    #[test]
    fn code_fences_keep_a_known_language() {
        let markdown = "```mermaid\ngraph TD\n  A --> B\n```\n```Rust\nfn main() {}\n```\n```cobol\nSTOP RUN.\n```";
        let copied = from_markdown(markdown);
        let languages: Vec<Option<&str>> = copied
            .blocks
            .iter()
            .map(|block| block.language.as_deref())
            .collect();
        assert_eq!(languages, [Some("mermaid"), Some("rust"), None]);
        assert_eq!(copied.blocks[0].text, "graph TD\n  A --> B");
        assert!(to_markdown(&copied).starts_with("```mermaid\ngraph TD"));

        let from_an_older_version = r#"{"blocks":[{"kind":"Code","depth":0,"is_checked":false,"text":"x","spans":[]}],"is_fragment":false}"#;
        let copied: Copied = serde_json::from_str(from_an_older_version).unwrap();
        assert_eq!(copied.blocks[0].language, None);
    }

    #[test]
    fn unclosed_markup_stays_text_and_one_line_is_a_fragment() {
        let copied = from_markdown("2 * 3 = 6 and **open");
        assert!(copied.is_fragment);
        assert_eq!(copied.blocks[0].text, "2 * 3 = 6 and **open");
        assert!(copied.blocks[0].spans.is_empty());
    }
}
