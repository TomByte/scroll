//! A page of rich-text blocks backed by a Loro document.
//!
//! Local edits go straight into the `PageDocument`; the view never edits its own copy of the
//! text. After every commit or import the editor drains Loro's change events, re-reads only
//! the blocks they name, and tells the virtual list which rows to measure again. IME
//! composition is the one exception: half-composed text lives only in the view until the
//! input method commits it, so neither the document nor other peers ever see it.

mod block_element;
mod block_layout;
mod block_paint;
mod block_selection;
mod callout_icon;
mod clipboard;
mod code_language;
pub mod commands;
pub mod document;
mod drag_drop;
mod editing;
mod formatting;
mod history;
mod inline_completion;
mod input_handler;
pub mod markdown;
mod navigation;
mod render;
#[cfg(test)]
mod replica_tests;
pub mod rich_text;
pub mod slash_commands;
pub mod structure;
mod syntax;
#[cfg(test)]
mod tests;
mod visual_lines;

use std::borrow::Cow;
use std::ops::Range;
use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    ListAlignment, ListState, Pixels, Point, SharedString, Subscription, Task, Window, actions,
};
use scroll_settings::Settings;
use scroll_ui::{PopupHost, TextSelection};

use crate::editor::document::{BlockId, BlockInfo, BlockKind, PageDocument, StableCursor};
use crate::page_layout::{PAGE_WIDTH, PageWidth};
use block_layout::BlockLayout;
use rich_text::{Marks, RichText};

/// The key context the editor binds its keys in; commands scoped to the editor use it too.
pub const KEY_CONTEXT: &str = "PageEditor";
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// How far each nesting level of blocks is indented.
pub(crate) const NEST_INDENT: Pixels = gpui::px(26.0);
/// Width of the column holding a block's bullet, number or checkbox.
pub(crate) const BLOCK_MARKER_WIDTH: Pixels = gpui::px(26.0);
pub(crate) const QUOTE_BAR_WIDTH: Pixels = gpui::px(3.0);

actions!(
    page_editor,
    [
        ToggleBold,
        ToggleItalic,
        ToggleStrikethrough,
        ToggleCode,
        ToggleLink,
        InsertLineBreak,
        Indent,
        Outdent,
        ToggleToDo,
        SelectBlocks,
        MoveBlocksUp,
        MoveBlocksDown,
    ]
);

/// Formatting and block commands are bound from the app's command list
/// (`commands`); only text editing keys are bound here.
pub fn init(cx: &mut App) {
    scroll_ui::actions::bind_text_editing_keys(KEY_CONTEXT, cx);
}

/// Turns the block with the caret (or every selected block) into `kind`.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = page_editor, no_json)]
pub struct SetBlockKind {
    pub kind: BlockKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub block: BlockId,
    pub range: TextSelection,
}

impl Selection {
    pub fn caret(block: BlockId, offset: usize) -> Self {
        Selection {
            block,
            range: TextSelection::caret(offset),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageEditorEvent {
    /// Local edits were committed; outgoing updates are waiting in `take_outgoing_updates`.
    Edited,
    /// The local selection moved, so presence should be re-sent.
    CaretMoved,
    /// The slash menu asked for a new database inside this page.
    CreateDatabase,
}

/// Whether this person may change the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAccess {
    Edit,
    /// A viewer: the page can be read, selected and copied, never changed.
    ReadOnly,
}

pub(crate) struct BlockState {
    info: BlockInfo,
    content: RichText,
    layout: Option<BlockLayout>,
    /// The render pass that last laid this block out; older layouts have stale positions.
    layout_frame: u64,
}

/// Another peer as last reported, resolved against this replica on each render.
#[derive(Clone)]
pub struct Collaborator {
    pub name: SharedString,
    /// Which of the theme's collaborator colours marks this person.
    pub color_index: usize,
    pub head: StableCursor,
    pub anchor: Option<StableCursor>,
}

/// Whole blocks selected after Escape; `anchor` and `head` are the ends in document order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockSelection {
    anchor: BlockId,
    head: BlockId,
}

/// Text being composed by an input method. It is shown at `offset` in `block` but not
/// written to the document until the input method commits it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Composition {
    block: BlockId,
    offset: usize,
    text: String,
    /// The input method's selection, relative to `text`.
    selected: Range<usize>,
}

pub struct PageEditor {
    page: PageDocument,
    blocks: Vec<BlockState>,
    positions: std::collections::HashMap<BlockId, usize>,
    list_numbers: Vec<Option<usize>>,
    list_state: ListState,
    /// Rendered above the first block and scrolled with the page, e.g. the page title.
    header: Option<AnyView>,
    selection: Selection,
    block_selection: Option<BlockSelection>,
    composition: Option<Composition>,
    /// Marks toggled with no text selected, applied to whatever is typed next.
    pending_marks: Option<Marks>,
    /// Horizontal position kept while moving up and down through lines of different length.
    goal_x: Option<Pixels>,
    is_dragging: bool,
    /// Set when the caret moved, so the next render scrolls it into view.
    needs_caret_revealed: bool,
    viewport: Option<Bounds<Pixels>>,
    /// Counts render passes, to tell current block layouts from stale ones.
    frame: u64,
    /// Other people on this page, by whatever key the caller uses for a connection.
    collaborators: std::collections::BTreeMap<u128, Collaborator>,
    access: PageAccess,
    link_prompt: Option<formatting::LinkPrompt>,
    /// The block-kind or emoji list at the caret, while one is open.
    completion: Option<inline_completion::InlineCompletion>,
    /// Popups opened from the page, such as a callout's emoji picker.
    popups: Entity<PopupHost>,
    callout_picker_subscription: Option<Subscription>,
    code_highlights: syntax::CodeHighlights,
    drop_target: Option<drag_drop::DropTarget>,
    /// The caret moved since the pointer last did, so the drag handle shows on the caret's
    /// block rather than under a pointer that has not moved.
    is_handle_at_caret: bool,
    /// As last laid out, so a change of the setting re-measures the rows.
    page_width: PageWidth,
    focus_handle: FocusHandle,
    is_caret_visible: bool,
    _blink_task: Task<()>,
    _settings: Subscription,
}

impl EventEmitter<PageEditorEvent> for PageEditor {}

impl Focusable for PageEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PageEditor {
    /// `selection` must lie in one of the page's blocks; `PageDocument::ensure_block` gives a
    /// block to start in.
    pub fn new(mut page: PageDocument, selection: Selection, cx: &mut Context<Self>) -> Self {
        page.take_changes();
        let blocks: Vec<BlockState> = page
            .blocks()
            .iter()
            .map(|info| BlockState {
                info: *info,
                content: read_content(&page, info.id),
                layout: None,
                layout_frame: 0,
            })
            .collect();
        let infos: Vec<BlockInfo> = blocks.iter().map(|block| block.info).collect();
        let list_state = ListState::new(blocks.len(), ListAlignment::Top, LIST_OVERDRAW);
        let positions = infos
            .iter()
            .enumerate()
            .map(|(index, info)| (info.id, index))
            .collect();
        PageEditor {
            positions,
            list_numbers: structure::list_numbers(&infos),
            list_state,
            header: None,
            selection,
            page,
            blocks,
            block_selection: None,
            composition: None,
            pending_marks: None,
            goal_x: None,
            is_dragging: false,
            needs_caret_revealed: false,
            viewport: None,
            frame: 0,
            collaborators: std::collections::BTreeMap::new(),
            access: PageAccess::Edit,
            link_prompt: None,
            completion: None,
            popups: cx.new(|_| PopupHost::new()),
            callout_picker_subscription: None,
            code_highlights: syntax::CodeHighlights::default(),
            drop_target: None,
            is_handle_at_caret: false,
            page_width: PAGE_WIDTH.get(cx),
            focus_handle: cx.focus_handle(),
            is_caret_visible: true,
            _blink_task: Task::ready(()),
            _settings: cx.observe_global::<Settings>(Self::follow_page_width),
        }
    }

    /// Shows `header` above the first block, scrolling with the page.
    pub fn set_header(&mut self, header: AnyView, cx: &mut Context<Self>) {
        if self.header.is_none() {
            self.list_state.splice(0..0, 1);
        }
        self.header = Some(header);
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Scrolls the page by `distance` (positive scrolls down).
    pub fn scroll_by(&mut self, distance: Pixels, cx: &mut Context<Self>) {
        self.list_state.scroll_by(distance);
        cx.notify();
    }

    /// Puts the caret at the start of the first block and focuses the editor.
    pub fn focus_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(first) = self
            .blocks
            .iter()
            .find(|block| block.info.kind != BlockKind::Divider)
        {
            self.selection = Selection::caret(first.info.id, 0);
        }
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// Read access for tests; the app itself talks to the page only through the editor.
    #[cfg(test)]
    pub fn page(&self) -> &PageDocument {
        &self.page
    }

    #[cfg(test)]
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Where the first other person's caret currently is on this replica.
    pub fn collaborator_position(&self) -> Option<(BlockId, usize)> {
        self.page
            .resolve_cursor(&self.collaborators.values().next()?.head)
    }

    pub fn set_access(&mut self, access: PageAccess, cx: &mut Context<Self>) {
        self.access = access;
        cx.notify();
    }

    pub fn is_read_only(&self) -> bool {
        self.access == PageAccess::ReadOnly
    }

    pub fn take_outgoing_updates(&mut self) -> Vec<Vec<u8>> {
        self.page.take_outgoing_updates()
    }

    /// The local selection as cursors another replica can resolve: head, then anchor.
    pub fn presence(&self, name: SharedString) -> Option<Collaborator> {
        let block = self.selection.block;
        let head = self.page.cursor_at(block, self.selection.range.head)?;
        let anchor = (!self.selection.range.is_empty())
            .then(|| self.page.cursor_at(block, self.selection.range.anchor))
            .flatten();
        Some(Collaborator {
            name,
            color_index: 0,
            head,
            anchor,
        })
    }

    /// Shows (or with `None`, removes) the person behind connection `key`.
    pub fn set_collaborator(
        &mut self,
        key: u128,
        collaborator: Option<Collaborator>,
        cx: &mut Context<Self>,
    ) {
        match collaborator {
            Some(collaborator) => self.collaborators.insert(key, collaborator),
            None => self.collaborators.remove(&key),
        };
        cx.notify();
    }

    /// Imports another replica's updates, keeping the local selection on the same
    /// characters even when text was inserted or removed before it.
    pub fn apply_remote_updates(&mut self, updates: &[Vec<u8>], cx: &mut Context<Self>) {
        if updates.is_empty() {
            return;
        }
        let anchored = self.anchor_selection();
        for update in updates {
            if let Err(error) = self.page.import(update) {
                log::error!("failed to import remote update: {error}");
            }
        }
        self.refresh_blocks();
        self.restore_selection(anchored);
        cx.notify();
    }

    /// Re-reads the blocks Loro reported as changed and tells the list which rows to measure
    /// again. The block list itself is rebuilt only when blocks were added, removed, moved or
    /// changed kind.
    fn refresh_blocks(&mut self) {
        let changes = self.page.take_changes();
        if changes.is_structure_changed {
            self.rebuild_block_list(&changes.blocks);
        } else {
            self.refresh_changed_blocks(&changes.blocks);
        }
        if self.blocks.is_empty() {
            match self.page.ensure_block() {
                Ok(_) => self.refresh_blocks(),
                Err(error) => {
                    log::error!("page has no blocks and one could not be created: {error}")
                }
            }
        }
    }

    /// Re-reads blocks whose text, kind or to-do state changed, without touching the rest.
    fn refresh_changed_blocks(&mut self, changed: &std::collections::HashSet<BlockId>) {
        let header = self.header_rows();
        let mut is_kind_changed = false;
        for block in changed {
            let Some(&index) = self.positions.get(block) else {
                continue;
            };
            let (info, content) = read_block(&self.page, *block);
            let state = &mut self.blocks[index];
            if let Some(info) = info {
                is_kind_changed |= state.info.kind != info.kind;
                state.info = info;
            }
            state.content = content;
            self.list_state
                .splice(header + index..header + index + 1, 1);
        }
        if is_kind_changed {
            self.list_numbers = structure::list_numbers(&self.block_infos());
        }
    }

    fn rebuild_block_list(&mut self, changed_text: &std::collections::HashSet<BlockId>) {
        let infos = self.page.blocks();
        let old_rows: Vec<BlockInfo> = self.blocks.iter().map(|block| block.info).collect();
        let mut previous: std::collections::HashMap<BlockId, BlockState> =
            std::mem::take(&mut self.blocks)
                .into_iter()
                .map(|block| (block.info.id, block))
                .collect();
        self.blocks = infos
            .iter()
            .map(|info| match previous.remove(&info.id) {
                Some(mut state) if !changed_text.contains(&info.id) => {
                    state.info = *info;
                    state
                }
                _ => BlockState {
                    info: *info,
                    content: read_content(&self.page, info.id),
                    layout: None,
                    layout_frame: 0,
                },
            })
            .collect();
        let (changed, count) = structure::changed_span(&old_rows, &infos);
        let header = self.header_rows();
        self.list_state
            .splice(header + changed.start..header + changed.end, count);
        for (index, state) in self.blocks.iter().enumerate() {
            if changed_text.contains(&state.info.id)
                && !(changed.start..changed.start + count).contains(&index)
            {
                self.list_state
                    .splice(header + index..header + index + 1, 1);
            }
        }
        self.list_numbers = structure::list_numbers(&infos);
        self.reindex_blocks();
    }

    pub(crate) fn record_layout(&mut self, block: BlockId, layout: BlockLayout) {
        let frame = self.frame;
        let index = self.block_index(block);
        if let Some(state) = index.and_then(|index| self.blocks.get_mut(index)) {
            state.layout = Some(layout);
            state.layout_frame = frame;
        }
    }

    fn header_rows(&self) -> usize {
        usize::from(self.header.is_some())
    }

    fn block_index(&self, block: BlockId) -> Option<usize> {
        self.positions.get(&block).copied()
    }

    fn block_state(&self, block: BlockId) -> Option<&BlockState> {
        self.blocks.get(self.block_index(block)?)
    }

    /// Keeps lookups by block id constant-time, so keystroke cost does not grow with the page.
    fn reindex_blocks(&mut self) {
        self.positions = self
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.info.id, index))
            .collect();
    }

    fn block_infos(&self) -> Vec<BlockInfo> {
        self.blocks.iter().map(|block| block.info).collect()
    }

    fn content(&self, block: BlockId) -> Option<&RichText> {
        self.block_state(block).map(|state| &state.content)
    }

    fn kind_of(&self, block: BlockId) -> BlockKind {
        self.block_state(block)
            .map_or(BlockKind::Paragraph, |state| state.info.kind)
    }

    fn selected_text(&self) -> &str {
        self.content(self.selection.block)
            .map_or("", |content| content.text.as_str())
    }

    /// The block's text as shown, with any composition in progress spliced in.
    fn display_content(&self, block: BlockId) -> Cow<'_, RichText> {
        let Some(content) = self.content(block) else {
            return Cow::Owned(RichText::default());
        };
        match self
            .composition
            .as_ref()
            .filter(|composition| composition.block == block)
        {
            Some(composition) => {
                Cow::Owned(content.with_insertion(composition.offset, &composition.text))
            }
            None => Cow::Borrowed(content),
        }
    }

    /// The selection as shown: inside the composition while one is in progress.
    fn display_selection(&self) -> TextSelection {
        match self.composition.as_ref() {
            Some(composition) => TextSelection::from_range(
                composition.offset + composition.selected.start
                    ..composition.offset + composition.selected.end,
                false,
            ),
            None => self.selection.range,
        }
    }

    fn display_marked_range(&self) -> Option<Range<usize>> {
        self.composition
            .as_ref()
            .map(|composition| composition.offset..composition.offset + composition.text.len())
    }

    fn restart_caret_blink(&mut self, cx: &mut Context<Self>) {
        self.is_caret_visible = true;
        self.needs_caret_revealed = true;
        self.is_handle_at_caret = true;
        self._blink_task = cx.spawn(async move |editor, cx| {
            loop {
                cx.background_executor().timer(CARET_BLINK_INTERVAL).await;
                let is_alive = editor.update(cx, |editor, cx| {
                    editor.is_caret_visible = !editor.is_caret_visible;
                    cx.notify();
                });
                if is_alive.is_err() {
                    break;
                }
            }
        });
    }

    /// Window position just below the caret at `offset`, for anchoring popovers.
    fn caret_anchor(&self, block: BlockId, offset: usize) -> Point<Pixels> {
        self.block_state(block)
            .and_then(|state| state.layout.as_ref())
            .map(|layout| {
                let caret = layout.caret_origin(offset);
                gpui::point(caret.x, caret.y + layout.line_height())
            })
            .unwrap_or_default()
    }
}

/// How far beyond the visible area the virtual list lays out blocks, so fast scrolling does
/// not show gaps.
const LIST_OVERDRAW: Pixels = gpui::px(600.0);

/// The block's kind and text as stored now; `None` for the kind if it could not be read.
fn read_block(page: &PageDocument, block: BlockId) -> (Option<BlockInfo>, RichText) {
    match page.read_block(block) {
        Ok((info, content)) => (Some(info), content),
        Err(error) => {
            log::error!("failed to read block {block:?}: {error}");
            (None, RichText::default())
        }
    }
}

fn read_content(page: &PageDocument, block: BlockId) -> RichText {
    page.block_content(block).unwrap_or_else(|error| {
        log::error!("failed to read block {block:?}: {error}");
        RichText::default()
    })
}
