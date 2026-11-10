//! Mermaid diagram blocks: code blocks in the Mermaid language, shown as their diagram. While
//! the caret is in one (and the editor has focus) its source shows above a live preview; the
//! button at the top right switches between the two, as does moving the caret in or out.
//!
//! Diagrams are drawn in the background (`mermaid::draw`), one drawing per block at a time,
//! and kept for the whole run in a cache all editors share (`cache`), so a page opened again
//! shows its diagrams at once. A block shows the newest drawing of its source and theme; the
//! last good one stays up while a newer one is drawn, and when the source has an error.

mod cache;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, ImageSource, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, SharedString, Size, Styled, Window, div, img,
    prelude::FluentBuilder, px,
};
use scroll_ui::{Button, ButtonSize, ButtonVariant, IconName, Theme, Transition};

use super::code_language::{CodeLanguage, code_frame};
use super::mermaid::{self, DiagramError, DiagramStyle, DrawnDiagram, EXAMPLE_DIAGRAM};
use super::{PageEditor, Selection};
use crate::editor::document::{BlockId, BlockKind};

/// The blocks at the top of a page that count as its first screen, whose diagrams (and code
/// grammars) are ready before the page is first shown.
pub(super) const FIRST_SCREEN_BLOCKS: usize = 24;
/// The room held for a diagram never drawn before, while it is drawn.
const PLACEHOLDER_HEIGHT: Pixels = px(160.0);

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
    /// The last drawing that succeeded, and what it was drawn from.
    shown: Option<(DiagramRequest, DrawnDiagram)>,
    /// Why the last drawing failed; cleared by the next one that succeeds.
    error: Option<SharedString>,
}

impl BlockDiagram {
    fn shows_wanted(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|(drawn_from, _)| self.wanted.as_ref() == Some(drawn_from))
    }

    fn show(&mut self, block: BlockId, request: DiagramRequest, drawn: DrawnDiagram, cx: &mut App) {
        cache::remember_size(block, drawn.size, cx);
        self.shown = Some((request, drawn));
        self.error = None;
    }
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
    /// now: nothing if that is already wanted or was drawn before (it shows at once), or
    /// while an older drawing of the block is under way (`finish` hands on the newest when it
    /// ends).
    fn request(
        &self,
        block: BlockId,
        source: &str,
        style: DiagramStyle,
        cx: &mut App,
    ) -> Option<DiagramRequest> {
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
        if let Some(drawn) = cache::cached(&request, cx) {
            diagram.show(block, request, drawn, cx);
            return None;
        }
        if diagram.is_drawing {
            return None;
        }
        diagram.is_drawing = true;
        Some(request)
    }

    /// Shows the outcome of drawing `finished` (already in the cache) and returns what to draw
    /// next, if the block asked for something newer meanwhile that is not drawn yet.
    fn finish(
        &self,
        block: BlockId,
        finished: DiagramRequest,
        outcome: Result<DrawnDiagram, DiagramError>,
        cx: &mut App,
    ) -> Option<DiagramRequest> {
        let mut blocks = self.blocks.borrow_mut();
        let diagram = blocks.get_mut(&block)?;
        diagram.is_drawing = false;
        match outcome {
            Ok(drawn) => {
                // A newer drawing found in the cache meanwhile is not replaced by an older one.
                if !diagram.shows_wanted() {
                    diagram.show(block, finished.clone(), drawn, cx);
                }
            }
            Err(error) if !diagram.shows_wanted() => diagram.error = Some(error.to_string().into()),
            Err(_) => {}
        }
        if diagram.shows_wanted() {
            return None;
        }
        let next = diagram
            .wanted
            .clone()
            .filter(|wanted| *wanted != finished)?;
        diagram.is_drawing = true;
        Some(next)
    }

    pub(super) fn view(&self, block: BlockId) -> DiagramView {
        let blocks = self.blocks.borrow();
        let diagram = blocks.get(&block);
        DiagramView {
            drawn: diagram
                .and_then(|diagram| diagram.shown.as_ref().map(|(_, drawn)| drawn.clone())),
            error: diagram.and_then(|diagram| diagram.error.clone()),
            is_drawing: diagram.is_some_and(|diagram| diagram.is_drawing),
        }
    }

    fn is_drawing(&self, block: BlockId) -> bool {
        self.blocks
            .borrow()
            .get(&block)
            .is_some_and(|diagram| diagram.is_drawing)
    }

    /// Lets go of every drawing, for when the editor goes away; the cache then frees what no
    /// other editor shows once it is over its budget.
    pub(super) fn release_images(&self, cx: &mut App) {
        self.blocks.borrow_mut().clear();
        cache::free_unused(cx);
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
        // A drawing that arrives after the block was first drawn fades in over its placeholder.
        let arrival = Transition::Reveal.fraction_on(
            ElementId::Name(format!("diagram-arrival-{block}").into()),
            view.drawn.is_some(),
            window,
            cx,
        );
        let body = match &view.drawn {
            Some(drawn) => diagram_image(drawn).when(arrival < 1.0, |image| image.opacity(arrival)),
            None if view.is_drawing => placeholder(cache::last_size(block, cx), &theme),
            None => div(),
        };
        let preview = diagram_preview(block, body, view.error, &theme);
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
        if let Some(request) = self.diagrams.request(block, source, style, cx) {
            start_drawing(block, request, cx);
        }
    }

    /// Starts drawing the diagrams on the page's first screen before the page is drawn, so
    /// they can be in place, at their size, in its first frame.
    pub(crate) fn draw_first_screen_diagrams(&self, theme: &Theme, cx: &mut Context<Self>) {
        let style = DiagramStyle::from_theme(theme);
        for block in self.first_screen_diagrams() {
            self.request_diagram(block, style, cx);
        }
    }

    /// Whether every diagram on the page's first screen is drawn, or failed to draw.
    pub(crate) fn are_first_screen_diagrams_drawn(&self) -> bool {
        self.first_screen_diagrams()
            .all(|block| !self.diagrams.is_drawing(block))
    }

    fn first_screen_diagrams(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks
            .iter()
            .take(FIRST_SCREEN_BLOCKS)
            .map(|block| block.info.id)
            .filter(|block| self.is_diagram(*block))
    }

    fn finish_drawing(
        &mut self,
        block: BlockId,
        finished: DiagramRequest,
        outcome: Result<DrawnDiagram, DiagramError>,
        cx: &mut Context<Self>,
    ) {
        if let Some(next) = self.diagrams.finish(block, finished, outcome, cx) {
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
        // Kept even if the page has closed meanwhile, so it shows at once when it opens again.
        let kept = cx.update(|cx| outcome.map(|drawn| cache::keep(request.clone(), drawn, cx)));
        let Ok(outcome) = kept else {
            return log::debug!("the app quit before a diagram was drawn");
        };
        let finished = editor.update(cx, |editor, cx| {
            editor.finish_drawing(block, request, outcome, cx)
        });
        if let Err(error) = finished {
            log::debug!("page closed before its diagram was drawn: {error}");
        }
    })
    .detach();
}

/// The diagram (`body`) on the page's background, with any error below it.
fn diagram_preview(
    block: BlockId,
    body: Div,
    error: Option<SharedString>,
    theme: &Theme,
) -> gpui::Stateful<Div> {
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
        .when_some(error, |preview, error| {
            preview.child(div().w_full().text_color(theme.colors.danger).child(error))
        })
}

/// Holds the diagram's place while it is drawn: at the size it was last shown in this run,
/// else at a usual height, so the page moves as little as possible when it arrives.
fn placeholder(last_size: Option<Size<Pixels>>, theme: &Theme) -> Div {
    let frame = match last_size {
        Some(size) => diagram_frame(size),
        None => div().w_full().h(PLACEHOLDER_HEIGHT),
    };
    frame
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.colors.text_placeholder)
        .child("Drawing diagram…")
}

/// The drawing at its own size, or narrower with the same proportions when the column is.
fn diagram_image(drawn: &DrawnDiagram) -> Div {
    diagram_frame(drawn.size)
        .debug_selector(|| "diagram-image".into())
        .child(img(ImageSource::Render(drawn.image.clone())).size_full())
}

/// `size`, or narrower with the same proportions when the column is.
fn diagram_frame(size: Size<Pixels>) -> Div {
    let mut frame = div().w(size.width).max_w_full();
    frame.style().aspect_ratio = Some(size.width / size.height);
    frame
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, TestAppContext, VisualTestContext};

    use super::*;
    use crate::editor::document::PageDocument;

    /// A page holding one diagram of `source`, as stored.
    fn diagram_page(source: &str) -> Vec<u8> {
        let page = PageDocument::create(1, &[source]).unwrap();
        let block = page.ensure_block().unwrap();
        page.set_kind(block, BlockKind::Code).unwrap();
        page.set_block_language(block, CodeLanguage::Mermaid.id())
            .unwrap();
        page.export_snapshot().unwrap()
    }

    /// Opens the stored page in a new editor and starts its first screen's diagrams, without
    /// letting anything run in the background: whether they are all drawn at once.
    fn open_again(snapshot: &[u8], cx: &mut VisualTestContext) -> bool {
        cx.update(|window, cx| {
            let theme = Theme::current(window, cx);
            let page = PageDocument::from_snapshot(2, snapshot).unwrap();
            let first = page.ensure_block().unwrap();
            let editor = cx.new(|cx| PageEditor::new(page, Selection::caret(first, 0), cx));
            editor.update(cx, |editor, cx| {
                editor.draw_first_screen_diagrams(&theme, cx)
            });
            editor.read(cx).are_first_screen_diagrams_drawn()
        })
    }

    #[gpui::test]
    fn a_diagram_drawn_once_shows_at_once_when_its_page_opens_again(cx: &mut TestAppContext) {
        cx.update(|cx| {
            scroll_ui::init(cx);
            crate::editor::init(cx);
        });
        let snapshot = diagram_page(EXAMPLE_DIAGRAM);
        let page = PageDocument::from_snapshot(1, &snapshot).unwrap();
        let first = page.ensure_block().unwrap();
        let (editor, cx) =
            cx.add_window_view(|_, cx| PageEditor::new(page, Selection::caret(first, 0), cx));
        cx.run_until_parked();
        let drawn = editor.read_with(cx, |editor, _| editor.diagrams.view(first).drawn);
        assert!(drawn.is_some(), "the first editor drew it");

        assert!(
            open_again(&snapshot, cx),
            "drawn already, with nothing left to draw"
        );
        assert!(
            !open_again(&diagram_page("flowchart LR\n    A --> B"), cx),
            "a diagram never drawn has to be drawn first"
        );
    }

    #[gpui::test]
    fn a_diagram_finished_after_its_page_closed_is_kept(cx: &mut TestAppContext) {
        cx.update(|cx| {
            scroll_ui::init(cx);
            crate::editor::init(cx);
        });
        let (_, cx) = cx.add_window_view(|_, _| gpui::Empty);
        let snapshot = diagram_page(EXAMPLE_DIAGRAM);
        assert!(
            !open_again(&snapshot, cx),
            "the editor started drawing and closed"
        );

        cx.run_until_parked();
        assert!(open_again(&snapshot, cx));
    }
}
