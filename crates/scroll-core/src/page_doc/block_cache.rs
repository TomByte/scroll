//! The blocks read from the document, kept between reads. A subscription notes which blocks each
//! committed change touched (local edits, imports, undo and redo all commit); the next read
//! re-reads only those, and rebuilds everything only when blocks were added, removed or moved.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::{Block, BlockId, PageChange};

#[derive(Default)]
pub(super) struct BlockCache {
    blocks: Vec<Arc<Block>>,
    index: HashMap<BlockId, usize>,
    is_built: bool,
}

/// What changed since the cache was last brought up to date, filled in by the subscription.
#[derive(Default)]
pub(super) struct PendingChanges {
    is_structure_changed: bool,
    blocks: HashSet<BlockId>,
}

impl PendingChanges {
    pub(super) fn record(&mut self, change: &PageChange) {
        self.is_structure_changed |= change.is_structure_changed;
        self.blocks.extend(change.blocks.iter().copied());
    }
}

impl BlockCache {
    pub(super) fn needs_rebuild(&self, pending: &PendingChanges) -> bool {
        !self.is_built || pending.is_structure_changed
    }

    pub(super) fn rebuild(&mut self, blocks: Vec<Block>) {
        self.index = blocks
            .iter()
            .enumerate()
            .map(|(position, block)| (block.id, position))
            .collect();
        self.blocks = blocks.into_iter().map(Arc::new).collect();
        self.is_built = true;
    }

    /// The cached blocks a change touched, with their depths, so they can be re-read.
    pub(super) fn touched<'a>(
        &'a self,
        pending: &'a PendingChanges,
    ) -> impl Iterator<Item = (BlockId, usize)> + 'a {
        pending.blocks.iter().filter_map(|id| {
            let position = self.index.get(id)?;
            Some((*id, self.blocks[*position].depth))
        })
    }

    pub(super) fn replace(&mut self, block: Block) {
        if let Some(position) = self.index.get(&block.id) {
            self.blocks[*position] = Arc::new(block);
        }
    }

    pub(super) fn blocks(&self) -> Vec<Arc<Block>> {
        self.blocks.clone()
    }

    pub(super) fn block(&self, id: BlockId) -> Option<Arc<Block>> {
        self.index
            .get(&id)
            .map(|position| self.blocks[*position].clone())
    }
}
