//! Paints one block's text: soft-wrapped rich text with line breaks, the local selection and
//! caret, and another peer's selection and caret. Records its layout on the editor for
//! hit-testing and IME geometry.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, Entity, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, SharedString, Style, TextRun, Window,
    WrappedLine, fill, point, relative, size,
};

use super::PageEditor;
use super::block_layout::BlockLayout;
use crate::editor::document::BlockId;

/// Someone else's selection is a light wash of their colour.
const REMOTE_SELECTION_OPACITY: f32 = 0.22;

/// Everything the element needs, computed by the editor at render time.
pub struct BlockPaint {
    pub text: SharedString,
    pub runs: Vec<TextRun>,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub selection: Option<Range<usize>>,
    pub caret: Option<usize>,
    /// Other people's carets and selections in this block.
    pub remote: Vec<RemotePresence>,
    pub placeholder: Option<Placeholder>,
    pub selection_color: Hsla,
    pub caret_color: Hsla,
    pub caret_width: Pixels,
}

/// Another peer's caret, and their selection if it lies in this block.
pub struct RemotePresence {
    pub caret: Option<usize>,
    pub selection: Option<Range<usize>>,
    pub color: Hsla,
    pub label: SharedString,
    pub label_run: TextRun,
    pub label_font_size: Pixels,
    pub label_padding: Pixels,
}

pub struct Placeholder {
    pub text: SharedString,
    pub run: TextRun,
}

pub struct BlockElement {
    editor: Entity<PageEditor>,
    block: BlockId,
    paint: BlockPaint,
}

impl BlockElement {
    pub fn new(editor: Entity<PageEditor>, block: BlockId, paint: BlockPaint) -> Self {
        BlockElement {
            editor,
            block,
            paint,
        }
    }
}

impl IntoElement for BlockElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

type ShapedLines = Rc<RefCell<Vec<WrappedLine>>>;

impl Element for BlockElement {
    type RequestLayoutState = ShapedLines;
    type PrepaintState = Option<BlockLayout>;

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
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let shaped: ShapedLines = Rc::default();
        let mut style = Style::default();
        style.size.width = relative(1.).into();

        let text = self.paint.text.clone();
        let runs = self.paint.runs.clone();
        let font_size = self.paint.font_size;
        let line_height = self.paint.line_height;
        let measured = shaped.clone();
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _| {
                let wrap_width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let lines = match window.text_system().shape_text(
                    text.clone(),
                    font_size,
                    &runs,
                    wrap_width,
                    None,
                ) {
                    Ok(lines) => lines.into_vec(),
                    Err(error) => {
                        log::error!("failed to shape block text: {error:#}");
                        Vec::new()
                    }
                };
                let visual_lines: usize = lines
                    .iter()
                    .map(|line| line.wrap_boundaries().len() + 1)
                    .sum();
                let widest = lines
                    .iter()
                    .map(|line| line.width())
                    .fold(Pixels::ZERO, Pixels::max);
                *measured.borrow_mut() = lines;
                size(
                    wrap_width.unwrap_or(widest),
                    line_height * visual_lines.max(1) as f32,
                )
            });
        (layout_id, shaped)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        shaped: &mut Self::RequestLayoutState,
        _: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let lines = std::mem::take(&mut *shaped.borrow_mut());
        let layout = BlockLayout::new(lines, bounds, self.paint.line_height);
        let block = self.block;
        let recorded = layout.clone();
        self.editor
            .update(cx, |editor, _| editor.record_layout(block, recorded));
        Some(layout)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(layout) = layout.as_ref() else {
            return;
        };
        layout.paint_backgrounds(window, cx);
        self.paint_remote_selection(layout, window);
        self.paint_selection(layout, window);
        layout.paint_text(window, cx);
        self.paint_placeholder(bounds, window, cx);
        self.paint_caret(layout, window);
        self.paint_remote_caret(layout, window, cx);
    }
}

impl BlockElement {
    fn paint_selection(&self, layout: &BlockLayout, window: &mut Window) {
        let Some(range) = self.paint.selection.as_ref() else {
            return;
        };
        for rectangle in layout.selection_bounds(range) {
            window.paint_quad(fill(rectangle, self.paint.selection_color));
        }
    }

    fn paint_remote_selection(&self, layout: &BlockLayout, window: &mut Window) {
        for remote in &self.paint.remote {
            let Some(range) = remote.selection.as_ref() else {
                continue;
            };
            for rectangle in layout.selection_bounds(range) {
                window.paint_quad(fill(
                    rectangle,
                    remote.color.opacity(REMOTE_SELECTION_OPACITY),
                ));
            }
        }
    }

    fn paint_caret(&self, layout: &BlockLayout, window: &mut Window) {
        let Some(offset) = self.paint.caret else {
            return;
        };
        let origin = layout.caret_origin(offset);
        let caret = Bounds::new(origin, size(self.paint.caret_width, layout.line_height()));
        window.paint_quad(fill(caret, self.paint.caret_color));
    }

    fn paint_placeholder(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let Some(placeholder) = self.paint.placeholder.as_ref() else {
            return;
        };
        let run = TextRun {
            len: placeholder.text.len(),
            ..placeholder.run.clone()
        };
        let line = window.text_system().shape_line(
            placeholder.text.clone(),
            self.paint.font_size,
            &[run],
            None,
        );
        if let Err(error) = line.paint(bounds.origin, self.paint.line_height, window, cx) {
            log::error!("failed to paint block placeholder: {error:#}");
        }
    }

    /// Other people's carets: a coloured bar with a small name tag above each.
    fn paint_remote_caret(&self, layout: &BlockLayout, window: &mut Window, cx: &mut App) {
        for remote in &self.paint.remote {
            let Some(offset) = remote.caret else {
                continue;
            };
            let origin = layout.caret_origin(offset);
            let bar = Bounds::new(
                origin,
                size(self.paint.caret_width * 1.5, layout.line_height()),
            );
            window.paint_quad(fill(bar, remote.color));

            let label_run = TextRun {
                len: remote.label.len(),
                ..remote.label_run.clone()
            };
            let label = window.text_system().shape_line(
                remote.label.clone(),
                remote.label_font_size,
                &[label_run],
                None,
            );
            let label_height = remote.label_font_size + remote.label_padding;
            let tag = Bounds::new(
                point(origin.x, origin.y - label_height),
                size(label.width + remote.label_padding * 2.0, label_height),
            );
            window.paint_quad(fill(tag, remote.color));
            let text_origin = point(tag.left() + remote.label_padding, tag.top());
            if let Err(error) = label.paint(text_origin, label_height, window, cx) {
                log::error!("failed to paint collaborator label: {error:#}");
            }
        }
    }
}
