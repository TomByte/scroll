//! Mermaid diagram blocks: code blocks in the Mermaid language, shown as their diagram. While
//! the caret is in one (and the editor has focus) its source shows above a live preview; the
//! button at the top right switches between the two, as does moving the caret in or out.
//!
//! Diagrams are drawn in the background (`mermaid::draw`), one drawing per block at a time,
//! and kept until the block's source or the theme changes. The last good drawing stays up
//! while a newer one is drawn, and when the source has an error.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, ImageSource, InteractiveElement,
    IntoElement, MouseButton, ParentElement, SharedString, Styled, Window, div, img,
    prelude::FluentBuilder,
};
use scroll_ui::{Button, ButtonSize, ButtonVariant, IconName, Theme};

use super::code_language::{CodeLanguage, code_frame};
use super::mermaid::{self, DiagramError, DiagramStyle, DrawnDiagram, EXAMPLE_DIAGRAM};
use super::{PageEditor, Selection};
use crate::editor::document::{BlockId, BlockKind};

/// What a diagram is drawn from; a change to either draws it again.
#[derive(Clone, PartialEq)]
struct DiagramRequest {
    source: String,
    style: DiagramStyle,
}

#[derive(Default)]
struct BlockDiagram {
    /// The newest source and style asked for.
    wanted: Option<DiagramRequest>,
    is_drawing: bool,
    /// The last drawing that succeeded.
    drawn: Option<DrawnDiagram>,
    /// Why the last drawing failed; cleared by the next one that succeeds.
    error: Option<SharedString>,
}

/// What to show for a diagram block now.
pub(super) struct DiagramView {
    pub drawn: Option<DrawnDiagram>,
    pub error: Option<SharedString>,
    pub is_drawing: bool,
}

/// Each diagram block's drawing.
#[derive(Default)]
pub(super) struct Diagrams {
    // Asked for while rendering, which only reads the editor.
    blocks: RefCell<HashMap<BlockId, BlockDiagram>>,
}

impl Diagrams {
    /// Notes that `block` should show `source` in `style`, and returns what to start drawing
    /// now: nothing if that is already wanted, or while an older drawing of the block is under
    /// way (`finish` hands on the newest when it ends).
    fn request(&self, block: BlockId, source: &str, style: DiagramStyle) -> Option<DiagramRequest> {
        let mut blocks = self.blocks.borrow_mut();
        let diagram = blocks.entry(block).or_default();
        let is_wanted = diagram
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.source == source && wanted.style == style);
        if is_wanted {
            return None;
        }
        let request = DiagramRequest {
            source: source.to_string(),
            style,
        };
        diagram.wanted = Some(request.clone());
        if diagram.is_drawing {
            return None;
        }
        diagram.is_drawing = true;
        Some(request)
    }

    /// Keeps the outcome of drawing `drawn` and returns what to draw next, if the block
    /// asked for something newer meanwhile. A replaced image is freed from every window.
    fn finish(
        &self,
        block: BlockId,
        drawn: &DiagramRequest,
        outcome: Result<DrawnDiagram, DiagramError>,
        cx: &mut App,
    ) -> Option<DiagramRequest> {
        let mut blocks = self.blocks.borrow_mut();
        let diagram = blocks.get_mut(&block)?;
        match outcome {
            Ok(new) => {
                if let Some(old) = diagram.drawn.replace(new) {
                    cx.drop_image(old.image, None);
                }
                diagram.error = None;
            }
            Err(error) => diagram.error = Some(error.to_string().into()),
        }
        let next = diagram.wanted.clone().filter(|wanted| wanted != drawn);
        diagram.is_drawing = next.is_some();
        next
    }

    pub(super) fn view(&self, block: BlockId) -> DiagramView {
        let blocks = self.blocks.borrow();
        let diagram = blocks.get(&block);
        DiagramView {
            drawn: diagram.and_then(|diagram| diagram.drawn.clone()),
            error: diagram.and_then(|diagram| diagram.error.clone()),
            is_drawing: diagram.is_some_and(|diagram| diagram.is_drawing),
        }
    }

    /// Frees every drawn image, for when the editor goes away.
    pub(super) fn release_images(&self, cx: &mut App) {
        for (_, diagram) in self.blocks.borrow_mut().drain() {
            if let Some(drawn) = diagram.drawn {
                cx.drop_image(drawn.image, None);
            }
        }
    }
}

impl PageEditor {
    pub(super) fn is_diagram(&self, block: BlockId) -> bool {
        self.kind_of(block) == BlockKind::Code && self.code_language(block) == CodeLanguage::Mermaid
    }

    /// Whether the block's source shows: the page can be edited and the focused editor's
    /// caret is in the block.
    pub(super) fn is_editing_diagram(&self, block: BlockId, window: &Window) -> bool {
        !self.is_read_only()
            && self.selection.block == block
            && self.block_selection.is_none()
            && self.focus_handle.is_focused(window)
    }

    /// The diagram, with its source (`text`) above it while it is being edited.
    pub(super) fn render_diagram_block(
        &self,
        block: BlockId,
        text: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(window, cx);
        self.request_diagram(block, DiagramStyle::from_theme(&theme), cx);
        let view = self.diagrams.view(block);
        let preview = diagram_preview(block, &view, &theme);
        if self.is_editing_diagram(block, window) {
            let controls = div()
                .flex()
                .items_center()
                .gap(theme.spacing.xs)
                .child(self.language_label(block, window, cx))
                .child(self.diagram_button(block, DiagramButton::Done, cx));
            return div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(theme.spacing.sm)
                .child(code_frame(text, controls, &theme))
                .child(preview)
                .into_any_element();
        }
        preview
            .on_mouse_down(
                MouseButton::Left,
                // A click selects the diagram as a block, to move, copy or delete it; only its
                // Edit button shows the source.
                cx.listener(move |editor, _, window, cx| {
                    cx.stop_propagation();
                    window.focus(&editor.focus_handle);
                    editor.select_block_range(block, block, cx);
                }),
            )
            .when(!self.is_read_only(), |preview| {
                preview.child(
                    div()
                        .absolute()
                        .top(theme.spacing.xs)
                        .right(theme.spacing.xs)
                        .child(self.diagram_button(block, DiagramButton::Edit, cx)),
                )
            })
            .into_any_element()
    }

    fn request_diagram(&self, block: BlockId, style: DiagramStyle, cx: &mut Context<Self>) {
        let source = self
            .content(block)
            .map_or("", |content| content.text.as_str());
        if let Some(request) = self.diagrams.request(block, source, style) {
            start_drawing(block, request, cx);
        }
    }

    fn finish_drawing(
        &mut self,
        block: BlockId,
        drawn: DiagramRequest,
        outcome: Result<DrawnDiagram, DiagramError>,
        cx: &mut Context<Self>,
    ) {
        if let Some(next) = self.diagrams.finish(block, &drawn, outcome, cx) {
            start_drawing(block, next, cx);
        }
        // The list keeps the heights of rows off screen; this one's may have changed.
        if let Some(index) = self.block_index(block) {
            let row = self.header_rows() + index;
            self.list_state.splice(row..row + 1, 1);
        }
        cx.notify();
    }

    fn diagram_button(
        &self,
        block: BlockId,
        button: DiagramButton,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (label, icon) = match button {
            DiagramButton::Edit => ("Edit", IconName::Pencil),
            DiagramButton::Done => ("Done", IconName::Check),
        };
        div()
            .debug_selector(|| format!("diagram-{}", label.to_lowercase()))
            // A press here must not also move the caret or start editing.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(
                    ElementId::Name(format!("diagram-{label}-{block}").into()),
                    label,
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Small)
                .icon(icon)
                .on_click(cx.listener(
                    move |editor, _: &ClickEvent, window, cx| match button {
                        DiagramButton::Edit => editor.edit_diagram(block, window, cx),
                        DiagramButton::Done => editor.select_block_range(block, block, cx),
                    },
                )),
            )
    }

    /// Shows the diagram's source with the caret at its end.
    fn edit_diagram(&mut self, block: BlockId, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if self.is_read_only() {
            return;
        }
        let end = self.content(block).map_or(0, |content| content.text.len());
        self.move_caret(block, end, cx);
    }

    /// Turns `block` into a diagram, removing `markup` (the slash command typed). An empty
    /// block gets an example diagram to start from.
    pub(super) fn convert_to_diagram(
        &mut self,
        block: BlockId,
        markup: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.completion = None;
        self.apply_edit(cx, |page| {
            page.delete_text(block, markup.clone())?;
            page.set_kind(block, BlockKind::Code)?;
            page.set_block_language(block, CodeLanguage::Mermaid.id())?;
            let mut length = page.block_content(block)?.text.len();
            page.clear_marks(block, 0..length)?;
            if length == 0 {
                page.insert_text(block, 0, EXAMPLE_DIAGRAM)?;
                length = EXAMPLE_DIAGRAM.len();
            }
            Ok(Selection::caret(block, length))
        });
    }
}

#[derive(Clone, Copy)]
enum DiagramButton {
    Edit,
    Done,
}

fn start_drawing(block: BlockId, request: DiagramRequest, cx: &mut Context<PageEditor>) {
    let drawing = cx.background_executor().spawn({
        let request = request.clone();
        async move { mermaid::draw(&request.source, &request.style) }
    });
    cx.spawn(async move |editor, cx| {
        let outcome = drawing.await;
        let finished = editor.update(cx, |editor, cx| {
            editor.finish_drawing(block, request, outcome, cx)
        });
        if let Err(error) = finished {
            log::debug!("page closed before its diagram was drawn: {error}");
        }
    })
    .detach();
}

/// The diagram on the page's background, scaled down to fit the column, with a placeholder
/// while it is first drawn and any error below it.
fn diagram_preview(block: BlockId, view: &DiagramView, theme: &Theme) -> gpui::Stateful<Div> {
    let body = match &view.drawn {
        Some(drawn) => diagram_image(drawn).into_any_element(),
        None if view.is_drawing => div()
            .text_color(theme.colors.text_placeholder)
            .child("Drawing diagram…")
            .into_any_element(),
        None => div().into_any_element(),
    };
    div()
        .id(ElementId::Name(format!("diagram-{block}").into()))
        .relative()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(theme.spacing.sm)
        .p(theme.spacing.lg)
        .rounded(theme.radius.md)
        .border(theme.size.border_width)
        .border_color(theme.colors.border)
        .bg(theme.colors.surface)
        .text_size(theme.text.caption.size)
        .line_height(theme.text.caption.line_height)
        .child(body)
        .when_some(view.error.clone(), |preview, error| {
            preview.child(div().w_full().text_color(theme.colors.danger).child(error))
        })
}

/// The drawing at its own size, or narrower with the same proportions when the column is.
fn diagram_image(drawn: &DrawnDiagram) -> Div {
    let mut frame = div()
        .debug_selector(|| "diagram-image".into())
        .w(drawn.size.width)
        .max_w_full();
    frame.style().aspect_ratio = Some(drawn.size.width / drawn.size.height);
    frame.child(img(ImageSource::Render(drawn.image.clone())).size_full())
}
