//! Soft-wrapped lines of one block, as byte ranges. GPUI reports wraps as glyph positions;
//! this turns them into line ranges and answers the caret questions the editor asks.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualLines {
    /// Byte offset where each visual line starts; the first is always 0.
    starts: Vec<usize>,
    text_length: usize,
}

impl VisualLines {
    /// `line_starts` are the offsets where lines begin (soft wraps and the starts of hard
    /// lines after a line break); zero, repeats and offsets past the text are ignored.
    pub fn new(line_starts: impl IntoIterator<Item = usize>, text_length: usize) -> Self {
        let mut starts = vec![0];
        for offset in line_starts {
            let is_after_previous = starts.last().is_some_and(|&previous| offset > previous);
            if is_after_previous && offset <= text_length {
                starts.push(offset);
            }
        }
        VisualLines {
            starts,
            text_length,
        }
    }

    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// The line a caret at `offset` is drawn on. A caret exactly at a wrap belongs to the
    /// line that starts there, as in every macOS text view.
    pub fn line_of(&self, offset: usize) -> usize {
        self.starts
            .iter()
            .rposition(|&start| start <= offset)
            .unwrap_or(0)
    }

    pub fn range(&self, line: usize) -> Range<usize> {
        let start = self.starts.get(line).copied().unwrap_or(self.text_length);
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text_length);
        start..end
    }

    /// Where a caret goes when placed past the end of `line`. On a wrapped line that is
    /// before the trailing space, so the caret stays on the line that was clicked.
    pub fn caret_end(&self, line: usize, text: &str) -> usize {
        let range = self.range(line);
        let is_last_line = line + 1 >= self.count();
        if is_last_line {
            return range.end;
        }
        let ends_with_whitespace = text[range.clone()]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        if ends_with_whitespace {
            let last_character_length = text[range.clone()]
                .chars()
                .next_back()
                .map_or(0, char::len_utf8);
            return range.end - last_character_length;
        }
        range.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "the quick brown fox";

    fn wrapped() -> VisualLines {
        // "the quick " | "brown fox"
        VisualLines::new([10], TEXT.len())
    }

    #[test]
    fn unwrapped_text_is_one_line() {
        let lines = VisualLines::new([], TEXT.len());
        assert_eq!(lines.count(), 1);
        assert_eq!(lines.range(0), 0..TEXT.len());
        assert_eq!(lines.line_of(TEXT.len()), 0);
    }

    #[test]
    fn caret_at_a_wrap_sits_on_the_following_line() {
        let lines = wrapped();
        assert_eq!(lines.line_of(9), 0);
        assert_eq!(lines.line_of(10), 1);
        assert_eq!(lines.line_of(TEXT.len()), 1);
    }

    #[test]
    fn caret_end_stops_before_a_wrapped_lines_trailing_space() {
        let lines = wrapped();
        assert_eq!(lines.caret_end(0, TEXT), 9);
        assert_eq!(lines.caret_end(1, TEXT), TEXT.len());
    }

    #[test]
    fn spurious_wrap_offsets_are_ignored() {
        let lines = VisualLines::new([0, 4, 4, 2, TEXT.len() + 1], TEXT.len());
        assert_eq!(lines.count(), 2);
        assert_eq!(lines.range(1), 4..TEXT.len());
    }

    #[test]
    fn a_trailing_line_break_starts_an_empty_line() {
        let text = "ab\n";
        let lines = VisualLines::new([3], text.len());
        assert_eq!(lines.count(), 2);
        assert_eq!(lines.line_of(3), 1);
        assert_eq!(
            lines.caret_end(0, text),
            2,
            "the caret stops before the line break"
        );
        assert_eq!(lines.range(1), 3..3);
    }

    #[test]
    fn empty_text_has_one_empty_line() {
        let lines = VisualLines::new([], 0);
        assert_eq!(lines.count(), 1);
        assert_eq!(lines.range(0), 0..0);
        assert_eq!(lines.caret_end(0, ""), 0);
    }
}
