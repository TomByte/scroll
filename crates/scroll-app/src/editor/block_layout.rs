//! Geometry of one painted block: maps between byte offsets and window positions. A block's
//! text can hold line breaks; each hard line is shaped separately and soft-wraps.

use std::ops::Range;

use gpui::{App, Bounds, Pixels, Point, SharedString, TextAlign, Window, WrappedLine, point, px};

use super::visual_lines::VisualLines;

#[derive(Clone)]
struct HardLine {
    line: WrappedLine,
    /// Byte offset of the line's first character in the block's text.
    start: usize,
    first_visual_line: usize,
}

#[derive(Clone)]
pub struct BlockLayout {
    hard_lines: Vec<HardLine>,
    lines: VisualLines,
    text: SharedString,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
}

impl BlockLayout {
    pub fn new(mut shaped: Vec<WrappedLine>, bounds: Bounds<Pixels>, line_height: Pixels) -> Self {
        if shaped.is_empty() {
            shaped.push(WrappedLine::default());
        }
        let mut hard_lines = Vec::with_capacity(shaped.len());
        let mut line_starts = Vec::new();
        let mut start = 0;
        let mut visual_line = 0;
        for line in shaped {
            line_starts.push(start);
            let layout = &line.unwrapped_layout;
            line_starts.extend(line.wrap_boundaries().iter().map(|boundary| {
                start + layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
            }));
            let next_start = start + line.text.len() + 1;
            let wrap_count = line.wrap_boundaries().len();
            hard_lines.push(HardLine {
                line,
                start,
                first_visual_line: visual_line,
            });
            visual_line += wrap_count + 1;
            start = next_start;
        }
        let text: String = hard_lines
            .iter()
            .map(|hard| hard.line.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        BlockLayout {
            lines: VisualLines::new(line_starts, text.len()),
            text: text.into(),
            hard_lines,
            bounds,
            line_height,
        }
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    pub fn line_count(&self) -> usize {
        self.lines.count()
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.lines.line_of(offset)
    }

    /// Horizontal position of `offset` relative to the block's left edge.
    pub fn x_of(&self, offset: usize) -> Pixels {
        self.x_on_line(self.lines.line_of(offset), offset)
    }

    /// Top-left corner of a caret at `offset`, in window coordinates.
    pub fn caret_origin(&self, offset: usize) -> Point<Pixels> {
        let line = self.lines.line_of(offset);
        point(
            self.bounds.left() + self.x_on_line(line, offset),
            self.bounds.top() + self.line_height * line as f32,
        )
    }

    pub fn offset_for_point(&self, position: Point<Pixels>) -> usize {
        let relative_y = (position.y - self.bounds.top()).max(px(0.0));
        let line = ((relative_y / self.line_height).floor() as usize).min(self.line_count() - 1);
        self.offset_at_x(line, position.x - self.bounds.left())
    }

    /// The caret offset on visual `line` closest to `x` (relative to the block's left edge).
    pub fn offset_at_x(&self, line: usize, x: Pixels) -> usize {
        let range = self.lines.range(line);
        let hard = self.hard_line_at(range.start);
        let line_start_x = self.unwrapped_x(hard, range.start);
        let closest = hard.start
            + hard
                .line
                .unwrapped_layout
                .closest_index_for_x(x + line_start_x);
        closest.clamp(range.start, self.lines.caret_end(line, &self.text))
    }

    /// One rectangle per visual line covered by `range`, in window coordinates.
    pub fn selection_bounds(&self, range: &Range<usize>) -> Vec<Bounds<Pixels>> {
        let first_line = self.lines.line_of(range.start);
        let last_line = self.lines.line_of(range.end);
        (first_line..=last_line)
            .filter_map(|line| {
                let line_range = self.lines.range(line);
                let start = range.start.max(line_range.start);
                let end = range.end.min(line_range.end);
                let continues_past_line = range.end > line_range.end;
                if start >= end && !continues_past_line {
                    return None;
                }
                let top = self.bounds.top() + self.line_height * line as f32;
                Some(Bounds::from_corners(
                    point(self.bounds.left() + self.x_on_line(line, start), top),
                    point(
                        self.bounds.left() + self.x_on_line(line, end),
                        top + self.line_height,
                    ),
                ))
            })
            .collect()
    }

    /// Paints run backgrounds (inline code), then nothing else; text goes on top later.
    pub fn paint_backgrounds(&self, window: &mut Window, cx: &mut App) {
        for hard in &self.hard_lines {
            let origin = self.hard_line_origin(hard);
            if let Err(error) = hard.line.paint_background(
                origin,
                self.line_height,
                TextAlign::Left,
                Some(self.bounds),
                window,
                cx,
            ) {
                log::error!("failed to paint block text background: {error:#}");
            }
        }
    }

    pub fn paint_text(&self, window: &mut Window, cx: &mut App) {
        for hard in &self.hard_lines {
            let origin = self.hard_line_origin(hard);
            if let Err(error) = hard.line.paint(
                origin,
                self.line_height,
                TextAlign::Left,
                Some(self.bounds),
                window,
                cx,
            ) {
                log::error!("failed to paint block text: {error:#}");
            }
        }
    }

    fn hard_line_origin(&self, hard: &HardLine) -> Point<Pixels> {
        point(
            self.bounds.left(),
            self.bounds.top() + self.line_height * hard.first_visual_line as f32,
        )
    }

    fn hard_line_at(&self, offset: usize) -> &HardLine {
        let index = self
            .hard_lines
            .iter()
            .rposition(|hard| hard.start <= offset)
            .unwrap_or(0);
        &self.hard_lines[index]
    }

    fn x_on_line(&self, line: usize, offset: usize) -> Pixels {
        let line_start = self.lines.range(line).start;
        let hard = self.hard_line_at(line_start);
        self.unwrapped_x(hard, offset) - self.unwrapped_x(hard, line_start)
    }

    fn unwrapped_x(&self, hard: &HardLine, offset: usize) -> Pixels {
        hard.line
            .unwrapped_layout
            .x_for_index(offset.saturating_sub(hard.start))
    }
}
