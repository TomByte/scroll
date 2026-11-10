//! One Loro document per page: a movable tree of blocks, each with a kind and rich text.
//!
//! Text offsets in this API are UTF-8 byte offsets into the block's text, matching Rust `String`
//! indexing. Every editing call commits, so subscribers see the change and it can be exported at
//! once; `edit_as_one_step` makes several calls undo together.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use loro::cursor::Cursor;
use loro::{
    Container, Counter, ExportMode, IdSpan, LoroDoc, LoroMap, LoroText, LoroTree, StyleConfig,
    StyleConfigMap, Subscription, UndoManager, ValueOrContainer,
};

mod blocks;
mod events;
mod history;
mod text;

pub use blocks::{Block, BlockKind, BlockPlacement};
pub use events::PageChange;
use history::undo_manager;
pub use text::{Mark, MarkKind, TextSpan};

pub type BlockId = loro::TreeID;

const BLOCKS: &str = "blocks";
const KIND_KEY: &str = "kind";
const CHECKED_KEY: &str = "checked";
const ICON_KEY: &str = "icon";
const LANGUAGE_KEY: &str = "language";
const TEXT_KEY: &str = "text";

#[derive(Debug, thiserror::Error)]
pub enum PageDocError {
    #[error(transparent)]
    Loro(#[from] loro::LoroError),
    #[error(transparent)]
    Encode(#[from] loro::LoroEncodeError),
    #[error(transparent)]
    Cursor(#[from] loro::CannotFindRelativePosition),
    #[error("block {0:?} does not exist or was deleted")]
    BlockNotFound(BlockId),
    #[error("block {0:?} has no text container")]
    MalformedBlock(BlockId),
    #[error("block {0:?} is not a to-do")]
    NotAToDo(BlockId),
    #[error(
        "offset {offset} is outside the text of block {block:?} or not on a character boundary"
    )]
    InvalidOffset { block: BlockId, offset: usize },
    #[error("the cursor does not point into the text of a block on this page")]
    CursorNotInPage,
    #[error("local edits must be taken with `take_local_update` before the peer id changes")]
    UnsentLocalEdits,
}

pub struct PageDoc {
    doc: LoroDoc,
    undo: UndoManager,
    /// End of this peer's operations already handed out by `take_local_update`.
    exported_until: Counter,
    /// The selection to restore if the next local edit is undone; read by the undo manager.
    selection_before_edit: Arc<Mutex<Vec<Cursor>>>,
    /// The selection the undo manager handed back from the last undo or redo.
    restored_selection: Arc<Mutex<Vec<Cursor>>>,
    cache: RefCell<BlockCache>,
    pending_changes: Arc<Mutex<PendingChanges>>,
    _change_subscription: Subscription,
}

impl Default for PageDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl PageDoc {
    pub fn new() -> Self {
        Self::wrap(LoroDoc::new())
    }

    pub fn from_snapshot(snapshot: &[u8]) -> Result<Self, PageDocError> {
        Ok(Self::wrap(LoroDoc::from_snapshot(snapshot)?))
    }

    fn wrap(doc: LoroDoc) -> Self {
        // Mark expansion is local configuration, not stored in the document, so every instance sets it.
        let mut styles = StyleConfigMap::new();
        for kind in MarkKind::ALL {
            styles.insert(
                kind.key().into(),
                StyleConfig {
                    expand: kind.expand(),
                },
            );
        }
        doc.config_text_style(styles);
        let selection_before_edit = Arc::new(Mutex::new(Vec::new()));
        let restored_selection = Arc::new(Mutex::new(Vec::new()));
        let pending_changes = Arc::new(Mutex::new(PendingChanges::default()));
        let recorded = pending_changes.clone();
        let change_subscription = doc.subscribe_root(Arc::new(move |event| {
            lock(&recorded).record(&PageChange::from_event(&event));
        }));
        Self {
            undo: undo_manager(&doc, &selection_before_edit, &restored_selection),
            doc,
            exported_until: 0,
            selection_before_edit,
            restored_selection,
            cache: RefCell::new(BlockCache::default()),
            pending_changes,
            _change_subscription: change_subscription,
        }
    }

    /// Replaces the random peer id Loro picks. Only needed when an id must be predictable (tests,
    /// or attributing edits); two replicas editing at once must never share one. Undo history is
    /// cleared, since it belongs to the old id.
    pub fn set_peer_id(&mut self, peer_id: u64) -> Result<(), PageDocError> {
        self.doc.commit();
        let old_peer = self.doc.peer_id();
        if self.doc.oplog_vv().get(&old_peer).copied().unwrap_or(0) > self.exported_until {
            return Err(PageDocError::UnsentLocalEdits);
        }
        self.doc.set_peer_id(peer_id)?;
        // Operations already in the document under this id were made, and sent, by an earlier session.
        self.exported_until = self.doc.oplog_vv().get(&peer_id).copied().unwrap_or(0);
        self.undo = undo_manager(
            &self.doc,
            &self.selection_before_edit,
            &self.restored_selection,
        );
        Ok(())
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, PageDocError> {
        Ok(self.doc.export(ExportMode::Snapshot)?)
    }

    /// Imports a remote update or snapshot. Updates may arrive out of order: Loro holds back
    /// any whose dependencies are missing and applies them once those arrive.
    pub fn import(&self, bytes: &[u8]) -> Result<(), PageDocError> {
        self.doc.import(bytes)?;
        Ok(())
    }

    /// The local edits (including undo and redo) made since the last call, as one Loro update,
    /// or `None` if there are none. Remote changes imported meanwhile are not included.
    pub fn take_local_update(&mut self) -> Result<Option<Vec<u8>>, PageDocError> {
        self.doc.commit();
        let peer = self.doc.peer_id();
        let end = self.doc.oplog_vv().get(&peer).copied().unwrap_or(0);
        if end <= self.exported_until {
            return Ok(None);
        }
        let span = IdSpan::new(peer, self.exported_until, end);
        let update = self.doc.export(ExportMode::updates_in_range(vec![span]))?;
        self.exported_until = end;
        Ok(Some(update))
    }

    /// Calls `callback` synchronously after every committed change, local or imported.
    /// Dropping the returned subscription stops the calls.
    pub fn subscribe(
        &self,
        callback: impl Fn(&PageChange) + Send + Sync + 'static,
    ) -> Subscription {
        self.doc.subscribe_root(Arc::new(move |event| {
            callback(&PageChange::from_event(&event))
        }))
    }

    fn tree(&self) -> LoroTree {
        self.doc.get_tree(BLOCKS)
    }

    fn block_meta(&self, id: BlockId) -> Result<LoroMap, PageDocError> {
        self.require_live_block(id)?;
        Ok(self.tree().get_meta(id)?)
    }

    fn block_text(&self, id: BlockId) -> Result<LoroText, PageDocError> {
        text_of(&self.block_meta(id)?).ok_or(PageDocError::MalformedBlock(id))
    }
}

/// A poisoned lock only means a callback panicked mid-update; the selection it guards is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn text_of(meta: &LoroMap) -> Option<LoroText> {
    match meta.get(TEXT_KEY)? {
        ValueOrContainer::Container(Container::Text(text)) => Some(text),
        _ => None,
    }
}

mod block_cache;

use block_cache::{BlockCache, PendingChanges};

#[cfg(test)]
mod tests;
