//! What a committed change touched, for repainting.

use loro::Index;
use loro::event::{Diff, DiffEvent};

use super::BlockId;

/// What one committed change touched, for repainting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageChange {
    /// Blocks were added, removed or moved; re-read the block list.
    pub is_structure_changed: bool,
    /// Blocks whose text, marks, kind or to-do state changed.
    pub blocks: Vec<BlockId>,
}

impl PageChange {
    pub(super) fn from_event(event: &DiffEvent) -> PageChange {
        let mut change = PageChange::default();
        for diff in &event.events {
            if matches!(diff.diff, Diff::Tree(_)) {
                change.is_structure_changed = true;
                continue;
            }
            if let Some(block) = block_in_path(diff.path)
                && !change.blocks.contains(&block)
            {
                change.blocks.push(block);
            }
        }
        change
    }
}

/// The block a container belongs to, from the container's path in the document.
pub(super) fn block_in_path(path: &[(loro::ContainerID, Index)]) -> Option<BlockId> {
    path.iter().find_map(|(_, index)| match index {
        Index::Node(block) => Some(*block),
        _ => None,
    })
}
