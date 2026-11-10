//! Where a field's shaped text sits once it is split into the visual lines it wraps onto.
//!
//! Offsets here are into the drawn text (the placeholder, or the masked dots), and positions
//! are relative to the top left of the first visual line.

use std::ops::Range;

use gpui::{Bounds, Pixels, Point, WrappedLine, point, px, size};

use crate::text::previous_grapheme_boundary;

#[derive(Clone)]
pub(super) struct ShapedText {
    line: WrappedLine,
    line_height: Pixels,
    /// The offset each visual line starts at; the first is always 0.
    line_starts: Vec<usize>,
}

impl ShapedText {
    pub fn new(line: WrappedLine, line_height: Pixels) -> Self {
        let layout = &line.unwrapped_layout;
        let wrap_starts = line.wrap_boundaries.iter().filter_map(|boundary| {
            let run = layout.runs.get(boundary.run_ix)?;
            Some(run.glyphs.get(boundary.glyph_ix)?.index)
        });
        let line_starts = std::iter::once(0).chain(wrap_starts).collect();
        ShapedText {
            line,
            line_height,
            line_starts,
        }
    }

    pub fn line(&self) -> &WrappedLine {
        &self.line
    }

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    /// The top left of the caret placed before `offset`.
    pub fn position_for_offset(&self, offset: usize) -> Point<Pixels> {
        let row = visual_line_index(&self.line_starts, offset);
        point(self.x_within_line(row, offset), self.line_top(row))
    }

    /// The offset closest to `position`, kept on the visual line under it.
    pub fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let row = visual_line_at_y(position.y, self.line_height, self.line_starts.len());
        let start = self.line_starts[row];
        let end = self.line_end(row);
        let x = position.x + self.line.unwrapped_layout.x_for_index(start);
        let offset = self
            .line
            .unwrapped_layout
            .closest_index_for_x(x)
            .clamp(start, end);
        let is_last_line = row + 1 == self.line_starts.len();
        if offset == end && !is_last_line {
            // `end` is drawn at the start of the next line, so step back to stay on this one.
            return previous_grapheme_boundary(&self.line.text, end).max(start);
        }
        offset
    }

    /// One rectangle per visual line that `range` covers; none for an empty range.
    pub fn range_bounds(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        visual_line_spans(&self.line_starts, self.line.len(), range)
            .into_iter()
            .map(|(row, span)| {
                let top = self.line_top(row);
                Bounds::from_corners(
                    point(self.x_within_line(row, span.start), top),
                    point(self.x_within_line(row, span.end), top + self.line_height),
                )
            })
            .collect()
    }

    /// A line-high rectangle of `width` starting at the caret position for `offset`.
    pub fn caret_bounds(&self, offset: usize, width: Pixels) -> Bounds<Pixels> {
        Bounds::new(
            self.position_for_offset(offset),
            size(width, self.line_height),
        )
    }

    fn x_within_line(&self, row: usize, offset: usize) -> Pixels {
        let layout = &self.line.unwrapped_layout;
        layout.x_for_index(offset) - layout.x_for_index(self.line_starts[row])
    }

    fn line_top(&self, row: usize) -> Pixels {
        self.line_height * row as f32
    }

    fn line_end(&self, row: usize) -> usize {
        self.line_starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.line.len())
    }
}

/// The visual line `offset` is drawn on: an offset at a wrap starts the next line.
fn visual_line_index(line_starts: &[usize], offset: usize) -> usize {
    line_starts
        .partition_point(|&start| start <= offset)
        .saturating_sub(1)
}

/// The visual line under height `y`, clamped to the lines there are.
fn visual_line_at_y(y: Pixels, line_height: Pixels, line_count: usize) -> usize {
    if y <= px(0.0) || line_height <= px(0.0) {
        return 0;
    }
    let row = (y / line_height).floor() as usize;
    row.min(line_count.saturating_sub(1))
}

/// The part of `range` on each visual line, skipping lines it does not reach.
fn visual_line_spans(
    line_starts: &[usize],
    text_len: usize,
    range: Range<usize>,
) -> Vec<(usize, Range<usize>)> {
    line_starts
        .iter()
        .enumerate()
        .filter_map(|(row, &start)| {
            let end = line_starts.get(row + 1).copied().unwrap_or(text_len);
            let span = range.start.max(start)..range.end.min(end);
            (!span.is_empty()).then_some((row, span))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE_STARTS: [usize; 3] = [0, 10, 25];

    #[test]
    fn offset_at_a_wrap_belongs_to_the_next_line() {
        assert_eq!(visual_line_index(&LINE_STARTS, 0), 0);
        assert_eq!(visual_line_index(&LINE_STARTS, 9), 0);
        assert_eq!(visual_line_index(&LINE_STARTS, 10), 1);
        assert_eq!(visual_line_index(&LINE_STARTS, 40), 2);
        assert_eq!(visual_line_index(&[0], 5), 0);
    }

    #[test]
    fn height_maps_to_a_line_within_the_text() {
        let line_height = px(20.0);
        assert_eq!(visual_line_at_y(px(-5.0), line_height, 3), 0);
        assert_eq!(visual_line_at_y(px(19.0), line_height, 3), 0);
        assert_eq!(visual_line_at_y(px(20.0), line_height, 3), 1);
        assert_eq!(visual_line_at_y(px(500.0), line_height, 3), 2);
        assert_eq!(visual_line_at_y(px(500.0), line_height, 1), 0);
    }

    #[test]
    fn range_splits_into_one_span_per_line_it_covers() {
        assert_eq!(
            visual_line_spans(&LINE_STARTS, 30, 5..27),
            vec![(0, 5..10), (1, 10..25), (2, 25..27)]
        );
        assert_eq!(
            visual_line_spans(&LINE_STARTS, 30, 12..14),
            vec![(1, 12..14)]
        );
        // Ending exactly at a wrap does not reach onto the next line.
        assert_eq!(visual_line_spans(&LINE_STARTS, 30, 3..10), vec![(0, 3..10)]);
        assert!(visual_line_spans(&LINE_STARTS, 30, 7..7).is_empty());
    }
}
