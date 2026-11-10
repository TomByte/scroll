use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ContentMask, Edges, Element, ElementId, ElementInputHandler,
    Entity, Font, GlobalElementId, InspectorElementId, IntoElement, LayoutId, PaintQuad, Pixels,
    SharedString, Style, TextAlign, TextRun, UnderlineStyle, Window, WrappedLine, fill, font,
    point, px, relative, size,
};

use super::shaped_text::ShapedText;
use super::{LineLayoutCache, TextInput};
use crate::theme::Theme;

/// Paints a `TextInput`'s text, selection and caret. A single-line field scrolls
/// horizontally to keep the caret in view; a wrapping one grows a line at a time instead.
pub(super) struct TextInputElement {
    input: Entity<TextInput>,
}

impl TextInputElement {
    pub fn new(input: Entity<TextInput>) -> Self {
        TextInputElement { input }
    }
}

/// How far, as a fraction of the font size, the text may be painted past the field's left and
/// right edges. Some glyphs' ink starts left of the pen position (a bold "W" or "T" at the
/// start of a title), and a mask at the exact edge shaves it off.
const GLYPH_OVERHANG: f32 = 0.125;

pub(super) struct PrepaintState {
    text: ShapedText,
    /// Where the text may paint: the field, widened by the glyph overhang.
    text_mask: Bounds<Pixels>,
    scroll_offset: Pixels,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let theme = Theme::current(window, cx);
        let input = self.input.read(cx);
        let line_height = input.text_size.unwrap_or(theme.text.ui).line_height;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if !input.is_wrapping {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }

        let text = DisplayText::new(input, &theme);
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _| {
                let wrap_width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
                });
                // Like a single-line field, a wrapping one has no width of its own: it takes
                // what its parent gives it.
                let height = text.shape(wrap_width, window).size(line_height).height;
                size(wrap_width.unwrap_or_default(), height)
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = Theme::current(window, cx);
        let input = self.input.read(cx);
        let text_size = input.text_size.unwrap_or(theme.text.ui);
        let line_height = text_size.line_height;
        let text_mask = text_mask(bounds, text_size.size);
        let wrap_width = input.is_wrapping.then_some(bounds.size.width);
        let text = ShapedText::new(
            DisplayText::new(input, &theme).shape(wrap_width, window),
            line_height,
        );

        let caret_offset = if input.text.is_empty() {
            0
        } else {
            input.display_offset(input.selection.head)
        };
        let caret_position = text.position_for_offset(caret_offset);
        let scroll_offset = if input.is_wrapping {
            px(0.0)
        } else {
            let previous_scroll = input
                .layout
                .as_ref()
                .map_or(px(0.0), |layout| layout.scroll_offset);
            scroll_to_reveal(
                previous_scroll,
                caret_position.x,
                bounds.size.width,
                theme.size.caret_width,
            )
        };
        let origin = point(bounds.left() - scroll_offset, bounds.top());

        let selection_range = input.selection.range();
        let selection = text
            .range_bounds(
                input.display_offset(selection_range.start)
                    ..input.display_offset(selection_range.end),
            )
            .into_iter()
            .map(|line_bounds| fill(line_bounds + origin, theme.colors.text_selection))
            .collect();
        let caret = selection_range.is_empty().then(|| {
            fill(
                text.caret_bounds(caret_offset, theme.size.caret_width) + origin,
                theme.colors.caret,
            )
        });

        PrepaintState {
            text,
            text_mask,
            scroll_offset,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        let origin = point(bounds.left() - prepaint.scroll_offset, bounds.top());
        let mask = ContentMask {
            bounds: prepaint.text_mask,
        };
        window.with_content_mask(Some(mask), |window| {
            for selection in prepaint.selection.drain(..) {
                window.paint_quad(selection);
            }
            let text = &prepaint.text;
            let painted = text.line().paint(
                origin,
                text.line_height(),
                TextAlign::Left,
                None,
                window,
                cx,
            );
            if let Err(error) = painted {
                log::error!("failed to paint text input text: {error:#}");
            }
            if focus_handle.is_focused(window)
                && let Some(caret) = prepaint.caret.take()
            {
                window.paint_quad(caret);
            }
        });

        let text = prepaint.text.clone();
        let scroll_offset = prepaint.scroll_offset;
        self.input.update(cx, |input, _| {
            input.layout = Some(LineLayoutCache {
                text,
                bounds,
                scroll_offset,
            });
        });
    }
}

/// Where a field's text may paint: its bounds, widened left and right by the glyph overhang.
fn text_mask(bounds: Bounds<Pixels>, font_size: Pixels) -> Bounds<Pixels> {
    let overhang = font_size * GLYPH_OVERHANG;
    bounds.extend(Edges {
        left: overhang,
        right: overhang,
        ..Edges::default()
    })
}

/// The text a field draws, styled and ready to shape at any width.
struct DisplayText {
    text: SharedString,
    font_size: Pixels,
    runs: Vec<TextRun>,
}

impl DisplayText {
    /// The field's text (masked if need be), or its placeholder while it is empty.
    fn new(input: &TextInput, theme: &Theme) -> Self {
        let (text, color) = if input.text.is_empty() {
            (input.placeholder.clone(), theme.colors.text_placeholder)
        } else {
            (SharedString::from(input.display_text()), theme.colors.text)
        };
        let base = TextRun {
            len: text.len(),
            font: Font {
                weight: input.font_weight,
                ..font(theme.font.ui.clone())
            },
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_range = input
            .marked_range
            .clone()
            .map(|range| input.display_offset(range.start)..input.display_offset(range.end));
        DisplayText {
            text,
            font_size: input.text_size.unwrap_or(theme.text.ui).size,
            runs: text_runs(base, marked_range, theme),
        }
    }

    /// Lays the text out on one line, wrapping it at `wrap_width` if given.
    fn shape(&self, wrap_width: Option<Pixels>, window: &Window) -> WrappedLine {
        let shaped = window.text_system().shape_text(
            self.text.clone(),
            self.font_size,
            &self.runs,
            wrap_width,
            None,
        );
        match shaped {
            // The text has no newlines, so it shapes to exactly one line.
            Ok(lines) => lines.into_iter().next().unwrap_or_default(),
            Err(error) => {
                log::error!("failed to shape text input text: {error:#}");
                WrappedLine::default()
            }
        }
    }
}

fn text_runs(base: TextRun, marked_range: Option<Range<usize>>, theme: &Theme) -> Vec<TextRun> {
    let length = base.len;
    let color = base.color;
    let Some(marked) = marked_range else {
        return vec![base];
    };
    let marked_underline = UnderlineStyle {
        color: Some(color),
        thickness: theme.size.border_width,
        wavy: false,
    };
    [
        TextRun {
            len: marked.start,
            ..base.clone()
        },
        TextRun {
            len: marked.end - marked.start,
            underline: Some(marked_underline),
            ..base.clone()
        },
        TextRun {
            len: length - marked.end,
            ..base
        },
    ]
    .into_iter()
    .filter(|run| run.len > 0)
    .collect()
}

/// The smallest change to `scroll_offset` that keeps the caret inside `visible_width`.
fn scroll_to_reveal(
    scroll_offset: Pixels,
    caret_x: Pixels,
    visible_width: Pixels,
    caret_width: Pixels,
) -> Pixels {
    if caret_x < scroll_offset {
        return caret_x;
    }
    let rightmost_visible = scroll_offset + visible_width - caret_width;
    if caret_x > rightmost_visible {
        return caret_x - visible_width + caret_width;
    }
    scroll_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_reveals_caret_past_either_edge() {
        let width = px(100.0);
        let caret = px(2.0);
        assert_eq!(scroll_to_reveal(px(0.0), px(50.0), width, caret), px(0.0));
        assert_eq!(scroll_to_reveal(px(0.0), px(150.0), width, caret), px(52.0));
        assert_eq!(scroll_to_reveal(px(60.0), px(10.0), width, caret), px(10.0));
    }

    #[test]
    fn text_may_paint_a_little_past_the_left_and_right_edges_only() {
        let field = Bounds::new(point(px(40.0), px(10.0)), size(px(200.0), px(40.0)));
        let mask = text_mask(field, px(32.0));
        assert_eq!(mask.left(), px(36.0), "room for a bold first letter's ink");
        assert_eq!(mask.right(), px(244.0));
        assert_eq!((mask.top(), mask.bottom()), (field.top(), field.bottom()));
    }
}
