//! A block's text as the view sees it: a string plus contiguous runs of inline marks.
//! Offsets are UTF-8 byte offsets. Nothing here knows about Loro.

use std::ops::Range;

use gpui::{
    Font, FontStyle, FontWeight, Hsla, SharedString, StrikethroughStyle, TextRun, UnderlineStyle,
    font,
};

/// Underlines are drawn fainter than the text they mark.
const UNDERLINE_OPACITY: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Bold,
    Italic,
    Strikethrough,
    Code,
    Link,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<String>,
}

impl Marks {
    pub fn has(&self, kind: MarkKind) -> bool {
        match kind {
            MarkKind::Bold => self.bold,
            MarkKind::Italic => self.italic,
            MarkKind::Strikethrough => self.strikethrough,
            MarkKind::Code => self.code,
            MarkKind::Link => self.link.is_some(),
        }
    }

    /// Sets a boolean mark. Links carry a URL and are set through `link` directly.
    pub fn set(&mut self, kind: MarkKind, is_on: bool) {
        match kind {
            MarkKind::Bold => self.bold = is_on,
            MarkKind::Italic => self.italic = is_on,
            MarkKind::Strikethrough => self.strikethrough = is_on,
            MarkKind::Code => self.code = is_on,
            MarkKind::Link => {
                if !is_on {
                    self.link = None;
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkSpan {
    pub range: Range<usize>,
    pub marks: Marks,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RichText {
    pub text: String,
    /// Contiguous, non-empty spans covering `text` exactly; neighbours never share marks.
    pub spans: Vec<MarkSpan>,
}

/// Fonts and colours needed to turn marks into GPUI text runs.
pub struct RunStyle {
    pub font_family: SharedString,
    pub code_font_family: SharedString,
    /// Weight of unmarked text; headings are heavier than paragraphs.
    pub font_weight: FontWeight,
    pub text_color: Hsla,
    pub link_color: Hsla,
    pub code_background: Hsla,
    pub decoration_thickness: gpui::Pixels,
    /// Draws every run struck through, as for a checked to-do.
    pub is_struck_through: bool,
}

impl RichText {
    pub fn push(&mut self, text: &str, marks: Marks) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        if let Some(last) = self.spans.last_mut()
            && last.marks == marks
        {
            last.range.end = end;
            return;
        }
        self.spans.push(MarkSpan {
            range: start..end,
            marks,
        });
    }

    /// This text with `inserted` placed at `offset`, carrying the marks typing there would
    /// get. Used to show IME composition without writing it to the document.
    pub fn with_insertion(&self, offset: usize, inserted: &str) -> RichText {
        let mut result = RichText::default();
        let inserted_marks = self.marks_at(offset);
        for span in &self.spans {
            let before = span.range.start..span.range.end.min(offset);
            if !before.is_empty() {
                result.push(&self.text[before], span.marks.clone());
            }
        }
        result.push(inserted, inserted_marks);
        for span in &self.spans {
            let after = span.range.start.max(offset)..span.range.end;
            if !after.is_empty() {
                result.push(&self.text[after], span.marks.clone());
            }
        }
        result
    }

    /// The same text with no marks, as code blocks show it.
    pub fn without_marks(&self) -> RichText {
        let mut result = RichText::default();
        result.push(&self.text, Marks::default());
        result
    }

    /// The marks text typed at `offset` would naturally continue: those of the character
    /// before it, or of the first character at the start of the block.
    pub fn marks_at(&self, offset: usize) -> Marks {
        self.spans
            .iter()
            .find(|span| span.range.start < offset && offset <= span.range.end)
            .or(self.spans.first())
            .map(|span| span.marks.clone())
            .unwrap_or_default()
    }

    /// Whether every character in `range` carries `kind`. Used to decide whether a toggle
    /// shortcut adds or removes the mark.
    pub fn is_marked_throughout(&self, range: &Range<usize>, kind: MarkKind) -> bool {
        self.spans
            .iter()
            .filter(|span| span.range.start < range.end && range.start < span.range.end)
            .all(|span| span.marks.has(kind))
    }

    pub fn link_at(&self, offset: usize) -> Option<&str> {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .and_then(|span| span.marks.link.as_deref())
    }

    /// Text runs for painting. `underlined` (the IME's marked text) is split out so it can
    /// carry an underline on top of its marks.
    pub fn text_runs(&self, style: &RunStyle, underlined: Option<&Range<usize>>) -> Vec<TextRun> {
        let mut boundaries: Vec<usize> = self
            .spans
            .iter()
            .flat_map(|span| [span.range.start, span.range.end])
            .chain(
                underlined
                    .into_iter()
                    .flat_map(|range| [range.start, range.end]),
            )
            .filter(|&offset| offset <= self.text.len())
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        boundaries
            .windows(2)
            .map(|pair| {
                let range = pair[0]..pair[1];
                let marks = self.marks_covering(range.start);
                let is_underlined = underlined
                    .is_some_and(|marked| marked.start <= range.start && range.end <= marked.end);
                run_for(range.len(), &marks, is_underlined, style)
            })
            .collect()
    }

    fn marks_covering(&self, offset: usize) -> Marks {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .map(|span| span.marks.clone())
            .unwrap_or_default()
    }
}

fn run_for(length: usize, marks: &Marks, is_underlined: bool, style: &RunStyle) -> TextRun {
    let color = if marks.link.is_some() {
        style.link_color
    } else {
        style.text_color
    };
    let underline = (is_underlined || marks.link.is_some()).then(|| UnderlineStyle {
        thickness: style.decoration_thickness,
        color: Some(color.opacity(UNDERLINE_OPACITY)),
        wavy: false,
    });
    TextRun {
        len: length,
        font: font_for(marks, style),
        color,
        background_color: marks.code.then_some(style.code_background),
        underline,
        strikethrough: (marks.strikethrough || style.is_struck_through).then_some(
            StrikethroughStyle {
                thickness: style.decoration_thickness,
                color: Some(color),
            },
        ),
    }
}

fn font_for(marks: &Marks, style: &RunStyle) -> Font {
    let family = if marks.code {
        style.code_font_family.clone()
    } else {
        style.font_family.clone()
    };
    let mut font = font(family);
    font.weight = style.font_weight;
    if marks.bold {
        font.weight = FontWeight((style.font_weight.0 + 200.0).min(FontWeight::BOLD.0));
    }
    if marks.italic {
        font.style = FontStyle::Italic;
    }
    font
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn bold() -> Marks {
        Marks {
            bold: true,
            ..Marks::default()
        }
    }

    fn sample() -> RichText {
        let mut rich_text = RichText::default();
        rich_text.push("plain ", Marks::default());
        rich_text.push("bold", bold());
        rich_text.push(" more", Marks::default());
        rich_text
    }

    fn style() -> RunStyle {
        RunStyle {
            font_family: "Sans".into(),
            code_font_family: "Mono".into(),
            font_weight: FontWeight::NORMAL,
            text_color: gpui::black(),
            link_color: gpui::blue(),
            code_background: gpui::white(),
            decoration_thickness: px(1.0),
            is_struck_through: false,
        }
    }

    #[test]
    fn push_merges_equal_neighbours_and_skips_empty_text() {
        let mut rich_text = RichText::default();
        rich_text.push("a", bold());
        rich_text.push("", Marks::default());
        rich_text.push("b", bold());
        assert_eq!(rich_text.spans.len(), 1);
        assert_eq!(rich_text.spans[0].range, 0..2);
    }

    #[test]
    fn marks_at_follow_the_preceding_character() {
        let rich_text = sample();
        assert_eq!(rich_text.marks_at(6), Marks::default());
        assert_eq!(rich_text.marks_at(7), bold());
        assert_eq!(rich_text.marks_at(10), bold());
        assert_eq!(rich_text.marks_at(11), Marks::default());
        assert_eq!(rich_text.marks_at(0), Marks::default());
        assert_eq!(RichText::default().marks_at(0), Marks::default());
    }

    #[test]
    fn toggling_checks_the_whole_range() {
        let rich_text = sample();
        assert!(rich_text.is_marked_throughout(&(6..10), MarkKind::Bold));
        assert!(!rich_text.is_marked_throughout(&(5..10), MarkKind::Bold));
        assert!(!rich_text.is_marked_throughout(&(6..11), MarkKind::Bold));
    }

    #[test]
    fn runs_cover_the_text_and_split_at_the_marked_range() {
        let rich_text = sample();
        let runs = rich_text.text_runs(&style(), Some(&(2..8)));
        let lengths: Vec<usize> = runs.iter().map(|run| run.len).collect();
        assert_eq!(lengths, vec![2, 4, 2, 2, 5]);
        assert!(runs[1].underline.is_some());
        assert!(runs[2].underline.is_some());
        assert!(runs[3].underline.is_none());
        assert_eq!(runs[2].font.weight, FontWeight(600.0));
        assert_eq!(lengths.iter().sum::<usize>(), rich_text.text.len());
    }

    #[test]
    fn insertion_takes_the_marks_of_the_text_before_it() {
        let spliced = sample().with_insertion(8, "XY");
        assert_eq!(spliced.text, "plain boXYld more");
        assert!(spliced.is_marked_throughout(&(6..12), MarkKind::Bold));
        assert!(!spliced.is_marked_throughout(&(12..13), MarkKind::Bold));
        assert_eq!(RichText::default().with_insertion(0, "a").text, "a");
        assert_eq!(
            sample().with_insertion(sample().text.len(), "!").text,
            "plain bold more!"
        );
    }

    #[test]
    fn code_uses_the_code_font_and_background() {
        let mut rich_text = RichText::default();
        rich_text.push(
            "x",
            Marks {
                code: true,
                ..Marks::default()
            },
        );
        let runs = rich_text.text_runs(&style(), None);
        assert_eq!(runs[0].font.family, SharedString::from("Mono"));
        assert_eq!(runs[0].background_color, Some(gpui::white()));
    }
}
