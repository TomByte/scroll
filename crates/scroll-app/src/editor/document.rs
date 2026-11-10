//! The editor's view of a page document: core's `PageDoc`, plus what the editor needs to stay
//! fast and simple on top of it. All document logic lives in `scroll_core::page_doc`; this
//! module only adds
//!
//! - a cached block list with an index by id, rebuilt only when blocks are added, removed or
//!   moved, so keystroke cost does not grow with page size;
//! - the changes `PageDoc` reports, collected until the editor asks for them;
//! - conversion between core's marks and the editor's `RichText`;
//! - "before a block" placement, which core does not offer yet (built from `After` and
//!   `FirstChildOf` using the cached list).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use loro::Subscription;
use loro::cursor::{Cursor, Side};
use scroll_core::page_doc::{
    Block, BlockPlacement, Mark, MarkKind as CoreMarkKind, PageChange, PageDoc, PageDocError,
};
pub use scroll_core::page_doc::{BlockId, BlockKind};

use crate::editor::rich_text::{MarkKind, Marks, RichText};

/// Where a block goes. `None` as a parent means the top level of the page.
pub use scroll_core::page_doc::BlockPlacement as Placement;

/// A block's place and kind; its text is read separately because it changes far more often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockInfo {
    pub id: BlockId,
    pub parent: Option<BlockId>,
    /// 0 for top-level blocks.
    pub depth: usize,
    pub kind: BlockKind,
    pub is_checked: bool,
}

/// A position in the page that survives concurrent edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StableCursor(Cursor);

impl StableCursor {
    /// The cursor's wire form, for presence messages.
    pub fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }

    pub fn decode(bytes: &[u8]) -> Option<StableCursor> {
        Cursor::decode(bytes).ok().map(StableCursor)
    }
}

pub type PageError = PageDocError;
pub type PageResult<T> = Result<T, PageDocError>;

/// What changed since the last call to `take_changes`.
#[derive(Debug, Default)]
pub struct PageChanges {
    /// Blocks were added, removed or moved.
    pub is_structure_changed: bool,
    /// Blocks whose text, marks, kind or to-do state changed.
    pub blocks: HashSet<BlockId>,
}

#[derive(Default)]
struct BlockIndex {
    blocks: Arc<[BlockInfo]>,
    positions: HashMap<BlockId, usize>,
}

pub struct PageDocument {
    doc: PageDoc,
    changes: Arc<Mutex<PageChanges>>,
    index: RefCell<BlockIndex>,
    is_index_stale: Cell<bool>,
    outgoing_updates: Vec<Vec<u8>>,
    _subscription: Subscription,
}

impl PageDocument {
    pub fn new(doc: PageDoc) -> PageDocument {
        let changes = Arc::new(Mutex::new(PageChanges::default()));
        let subscription = doc.subscribe({
            let changes = changes.clone();
            move |change: &PageChange| {
                let mut changes = lock(&changes);
                changes.is_structure_changed |= change.is_structure_changed;
                changes.blocks.extend(change.blocks.iter().copied());
            }
        });
        PageDocument {
            doc,
            changes,
            index: RefCell::new(BlockIndex::default()),
            is_index_stale: Cell::new(true),
            outgoing_updates: Vec::new(),
            _subscription: subscription,
        }
    }

    /// A page with no blocks yet.
    pub fn empty() -> PageDocument {
        PageDocument::new(PageDoc::new())
    }

    /// A page rebuilt from storage: an optional snapshot, then updates in order.
    pub fn load<'a>(
        snapshot: Option<&[u8]>,
        updates: impl IntoIterator<Item = &'a [u8]>,
    ) -> PageResult<PageDocument> {
        let doc = match snapshot {
            Some(snapshot) => PageDoc::from_snapshot(snapshot)?,
            None => PageDoc::new(),
        };
        for update in updates {
            doc.import(update)?;
        }
        Ok(PageDocument::new(doc))
    }

    /// A page with `paragraphs`, under a fixed peer id (for tests).
    pub fn create(peer_id: u64, paragraphs: &[&str]) -> PageResult<PageDocument> {
        let mut doc = PageDoc::new();
        doc.set_peer_id(peer_id)?;
        for paragraph in paragraphs {
            let block =
                doc.insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)?;
            doc.insert_text(block, 0, paragraph)?;
        }
        doc.take_local_update()?;
        // Setting the id again clears undo history, so creating the page is not undoable.
        doc.set_peer_id(peer_id)?;
        let mut page = PageDocument::new(doc);
        page.take_changes();
        Ok(page)
    }

    /// Another replica under a fixed peer id, starting from a snapshot.
    pub fn from_snapshot(peer_id: u64, snapshot: &[u8]) -> PageResult<PageDocument> {
        let mut doc = PageDoc::from_snapshot(snapshot)?;
        doc.set_peer_id(peer_id)?;
        Ok(PageDocument::new(doc))
    }

    pub fn export_snapshot(&self) -> PageResult<Vec<u8>> {
        self.doc.export_snapshot()
    }

    /// Every block in document order: each block is followed by the blocks nested under it.
    pub fn blocks(&self) -> Arc<[BlockInfo]> {
        self.refresh_index();
        self.index.borrow().blocks.clone()
    }

    pub fn block_ids(&self) -> Vec<BlockId> {
        self.blocks().iter().map(|block| block.id).collect()
    }

    /// The first block, creating an empty paragraph if the page has none.
    pub fn ensure_block(&self) -> PageResult<BlockId> {
        if let Some(first) = self.blocks().first() {
            return Ok(first.id);
        }
        self.doc
            .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
    }

    /// The block's text with its marks, and its current kind and to-do state.
    pub fn read_block(&self, block: BlockId) -> PageResult<(BlockInfo, RichText)> {
        let block = self.doc.block(block)?;
        Ok((info_of(&block), rich_text_of(&block)))
    }

    pub fn block_content(&self, block: BlockId) -> PageResult<RichText> {
        Ok(self.read_block(block)?.1)
    }

    pub fn insert_block(&self, placement: Placement, kind: BlockKind) -> PageResult<BlockId> {
        self.doc.insert_block(placement, kind)
    }

    pub fn move_block(&self, block: BlockId, placement: Placement) -> PageResult<()> {
        self.doc.move_block(block, placement)
    }

    pub fn delete_block(&self, block: BlockId) -> PageResult<()> {
        self.doc.delete_block(block)
    }

    /// The block's own icon (callouts), if one was chosen.
    pub fn block_icon(&self, block: BlockId) -> Option<String> {
        self.doc
            .block(block)
            .ok()
            .and_then(|block| block.icon.clone())
    }

    pub fn set_block_icon(&self, block: BlockId, icon: Option<&str>) -> PageResult<()> {
        self.doc.set_block_icon(block, icon)
    }

    /// A code block's language id, if one was chosen.
    pub fn block_language(&self, block: BlockId) -> Option<String> {
        self.doc
            .block(block)
            .ok()
            .and_then(|block| block.language.clone())
    }

    pub fn set_block_language(&self, block: BlockId, language: Option<&str>) -> PageResult<()> {
        self.doc.set_block_language(block, language)
    }

    pub fn set_kind(&self, block: BlockId, kind: BlockKind) -> PageResult<()> {
        self.doc.set_block_kind(block, kind)
    }

    pub fn set_checked(&self, block: BlockId, is_checked: bool) -> PageResult<()> {
        if self.doc.block(block)?.is_checked == is_checked {
            return Ok(());
        }
        self.doc.toggle_to_do(block)
    }

    pub fn insert_text(&self, block: BlockId, offset: usize, text: &str) -> PageResult<()> {
        self.doc.insert_text(block, offset, text)
    }

    pub fn delete_text(&self, block: BlockId, range: Range<usize>) -> PageResult<()> {
        if range.is_empty() {
            return Ok(());
        }
        self.doc.delete_text(block, range)
    }

    /// Applies a boolean mark (bold, italic, strikethrough, code) to `range`.
    pub fn add_mark(&self, block: BlockId, range: Range<usize>, kind: MarkKind) -> PageResult<()> {
        let mark = match kind {
            MarkKind::Bold => Mark::Bold,
            MarkKind::Italic => Mark::Italic,
            MarkKind::Strikethrough => Mark::Strikethrough,
            MarkKind::Code => Mark::InlineCode,
            MarkKind::Link => return Ok(()),
        };
        self.doc.apply_mark(block, range, &mark)
    }

    pub fn add_link(&self, block: BlockId, range: Range<usize>, url: &str) -> PageResult<()> {
        self.doc
            .apply_mark(block, range, &Mark::Link(url.to_string()))
    }

    pub fn remove_mark(
        &self,
        block: BlockId,
        range: Range<usize>,
        kind: MarkKind,
    ) -> PageResult<()> {
        self.doc.remove_mark(block, range, core_mark_kind(kind))
    }

    /// Removes every mark from `range`, e.g. when text becomes a code block.
    pub fn clear_marks(&self, block: BlockId, range: Range<usize>) -> PageResult<()> {
        if range.is_empty() {
            return Ok(());
        }
        for kind in [
            MarkKind::Bold,
            MarkKind::Italic,
            MarkKind::Strikethrough,
            MarkKind::Code,
            MarkKind::Link,
        ] {
            self.remove_mark(block, range.clone(), kind)?;
        }
        Ok(())
    }

    /// Moves the text after `offset`, with its marks, into a new block of `kind` below.
    pub fn split_block(
        &self,
        block: BlockId,
        offset: usize,
        kind: BlockKind,
    ) -> PageResult<BlockId> {
        self.doc.split_block(block, offset, kind)
    }

    /// Joins the block onto the one above it in document order; `None` for the first block.
    pub fn merge_into_previous(&self, block: BlockId) -> PageResult<Option<(BlockId, usize)>> {
        self.doc.merge_into_previous(block)
    }

    pub fn cursor_at(&self, block: BlockId, offset: usize) -> Option<StableCursor> {
        self.doc
            .cursor(block, offset, Side::Middle)
            .ok()
            .map(StableCursor)
    }

    /// Where a cursor is now. `None` if its block has been removed.
    pub fn resolve_cursor(&self, cursor: &StableCursor) -> Option<(BlockId, usize)> {
        self.doc.cursor_position(&cursor.0).ok()
    }

    pub fn remember_selection_for_undo(&self, cursors: &[StableCursor]) {
        let cursors: Vec<Cursor> = cursors.iter().map(|cursor| cursor.0.clone()).collect();
        self.doc.remember_selection(&cursors);
    }

    /// Undoes this replica's latest edit step and returns the selection from before it.
    pub fn undo(&mut self) -> PageResult<Vec<StableCursor>> {
        Ok(self.doc.undo()?.into_iter().map(StableCursor).collect())
    }

    pub fn redo(&mut self) -> PageResult<Vec<StableCursor>> {
        Ok(self.doc.redo()?.into_iter().map(StableCursor).collect())
    }

    pub fn import(&self, update: &[u8]) -> PageResult<()> {
        self.doc.import(update)
    }

    /// Collects this replica's latest edits as an update for storage and other replicas.
    pub fn commit(&mut self) {
        match self.doc.take_local_update() {
            Ok(Some(update)) => self.outgoing_updates.push(update),
            Ok(None) => {}
            Err(error) => log::error!("could not export local edits: {error}"),
        }
    }

    pub fn take_outgoing_updates(&mut self) -> Vec<Vec<u8>> {
        self.commit();
        std::mem::take(&mut self.outgoing_updates)
    }

    pub fn take_changes(&mut self) -> PageChanges {
        let changes = std::mem::take(&mut *lock(&self.changes));
        if changes.is_structure_changed {
            self.is_index_stale.set(true);
        } else if !changes.blocks.is_empty() {
            self.update_infos(&changes.blocks);
        }
        changes
    }

    /// Kind and to-do changes come as block changes, not structure; keep the cache current.
    fn update_infos(&self, blocks: &HashSet<BlockId>) {
        if self.is_index_stale.get() {
            return;
        }
        let mut index = self.index.borrow_mut();
        let mut infos: Vec<BlockInfo> = index.blocks.to_vec();
        let mut is_changed = false;
        for block in blocks {
            let (Some(&position), Ok(read)) = (index.positions.get(block), self.doc.block(*block))
            else {
                continue;
            };
            let info = info_of(&read);
            if infos[position] != info {
                infos[position] = info;
                is_changed = true;
            }
        }
        if is_changed {
            index.blocks = infos.into();
        }
    }

    fn refresh_index(&self) {
        let has_pending_structure = lock(&self.changes).is_structure_changed;
        if !self.is_index_stale.get() && !has_pending_structure {
            return;
        }
        let blocks: Vec<BlockInfo> = match self.doc.blocks() {
            Ok(blocks) => blocks.iter().map(|block| info_of(block)).collect(),
            Err(error) => {
                log::error!("could not read the page's blocks: {error}");
                Vec::new()
            }
        };
        let positions = blocks
            .iter()
            .enumerate()
            .map(|(position, block)| (block.id, position))
            .collect();
        *self.index.borrow_mut() = BlockIndex {
            blocks: blocks.into(),
            positions,
        };
        self.is_index_stale.set(false);
    }
}

fn info_of(block: &Block) -> BlockInfo {
    BlockInfo {
        id: block.id,
        parent: block.parent,
        depth: block.depth,
        kind: block.kind,
        is_checked: block.is_checked,
    }
}

fn rich_text_of(block: &Block) -> RichText {
    let mut rich_text = RichText::default();
    for span in &block.spans {
        let mut marks = Marks::default();
        for mark in &span.marks {
            match mark {
                Mark::Bold => marks.bold = true,
                Mark::Italic => marks.italic = true,
                Mark::Strikethrough => marks.strikethrough = true,
                Mark::InlineCode => marks.code = true,
                Mark::Link(url) => marks.link = Some(url.clone()),
            }
        }
        rich_text.push(&block.text[span.range.clone()], marks);
    }
    rich_text
}

fn core_mark_kind(kind: MarkKind) -> CoreMarkKind {
    match kind {
        MarkKind::Bold => CoreMarkKind::Bold,
        MarkKind::Italic => CoreMarkKind::Italic,
        MarkKind::Strikethrough => CoreMarkKind::Strikethrough,
        MarkKind::Code => CoreMarkKind::InlineCode,
        MarkKind::Link => CoreMarkKind::Link,
    }
}

/// A poisoned lock only means a change callback panicked; the data is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(page: &PageDocument) -> Vec<String> {
        page.block_ids()
            .into_iter()
            .map(|block| page.block_content(block).unwrap().text)
            .collect()
    }

    #[test]
    fn marks_convert_both_ways() {
        let page = PageDocument::create(1, &["bold link"]).unwrap();
        let block = page.block_ids()[0];
        page.add_mark(block, 0..4, MarkKind::Bold).unwrap();
        page.add_link(block, 5..9, "https://loro.dev").unwrap();
        let content = page.block_content(block).unwrap();
        assert!(content.is_marked_throughout(&(0..4), MarkKind::Bold));
        assert_eq!(content.link_at(6), Some("https://loro.dev"));
        page.remove_mark(block, 0..2, MarkKind::Bold).unwrap();
        assert!(
            !page
                .block_content(block)
                .unwrap()
                .is_marked_throughout(&(0..2), MarkKind::Bold)
        );
    }

    #[test]
    fn before_places_ahead_of_first_and_middle_children() {
        let page = PageDocument::create(1, &["a", "c"]).unwrap();
        let [a, c] = [page.block_ids()[0], page.block_ids()[1]];
        let b = page
            .insert_block(Placement::Before(c), BlockKind::Paragraph)
            .unwrap();
        page.insert_text(b, 0, "b").unwrap();
        let first = page
            .insert_block(Placement::Before(a), BlockKind::Paragraph)
            .unwrap();
        page.insert_text(first, 0, "0").unwrap();
        assert_eq!(texts(&page), ["0", "a", "b", "c"]);
    }

    #[test]
    fn the_block_index_follows_kind_changes_without_a_rebuild() {
        let mut page = PageDocument::create(1, &["x", "y"]).unwrap();
        let y = page.block_ids()[1];
        page.set_kind(y, BlockKind::ToDo).unwrap();
        page.set_checked(y, true).unwrap();
        let changes = page.take_changes();
        assert!(!changes.is_structure_changed);
        let blocks = page.blocks();
        assert_eq!(
            (blocks[1].kind, blocks[1].is_checked),
            (BlockKind::ToDo, true)
        );
    }

    #[test]
    fn replicas_converge_and_cursors_travel() {
        let mut left = PageDocument::create(1, &["shared"]).unwrap();
        let mut right = PageDocument::from_snapshot(2, &left.export_snapshot().unwrap()).unwrap();
        let block = left.block_ids()[0];
        let caret = right.cursor_at(block, 3).unwrap();
        left.insert_text(block, 0, "left ").unwrap();
        right.insert_text(block, 6, " right").unwrap();
        for update in left.take_outgoing_updates() {
            right.import(&update).unwrap();
        }
        for update in right.take_outgoing_updates() {
            left.import(&update).unwrap();
        }
        assert_eq!(texts(&left), ["left shared right"]);
        assert_eq!(texts(&left), texts(&right));
        assert_eq!(right.resolve_cursor(&caret), Some((block, 8)));
        let encoded = caret.encode();
        assert_eq!(
            StableCursor::decode(&encoded).and_then(|cursor| left.resolve_cursor(&cursor)),
            Some((block, 8))
        );
    }

    #[test]
    fn undo_restores_the_remembered_selection() {
        let mut page = PageDocument::create(1, &["base"]).unwrap();
        let block = page.block_ids()[0];
        let before = page.cursor_at(block, 4).unwrap();
        page.remember_selection_for_undo(std::slice::from_ref(&before));
        page.insert_text(block, 4, " more").unwrap();
        page.commit();
        let restored = page.undo().unwrap();
        assert_eq!(texts(&page), ["base"]);
        assert_eq!(
            restored
                .first()
                .and_then(|cursor| page.resolve_cursor(cursor)),
            Some((block, 4))
        );
    }
}
