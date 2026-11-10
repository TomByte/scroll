//! Questions about the block tree that need no window: list numbering, which blocks a
//! selection really covers, where a drag would drop, and which list rows changed.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::editor::document::{BlockId, BlockInfo, BlockKind, Placement};

/// The number shown before each numbered list item: a run of numbered siblings counts up
/// from 1, restarting after any other kind of block.
pub fn list_numbers(blocks: &[BlockInfo]) -> Vec<Option<usize>> {
    let mut previous_sibling: HashMap<Option<BlockId>, (BlockKind, usize)> = HashMap::new();
    blocks
        .iter()
        .map(|block| {
            let number = match previous_sibling.get(&block.parent) {
                Some((BlockKind::NumberedListItem, previous)) => previous + 1,
                _ => 1,
            };
            previous_sibling.insert(block.parent, (block.kind, number));
            (block.kind == BlockKind::NumberedListItem).then_some(number)
        })
        .collect()
}

/// Whether `block` is `ancestor` or nested anywhere under it.
pub fn is_within(blocks: &[BlockInfo], block: BlockId, ancestor: BlockId) -> bool {
    let parents: HashMap<BlockId, Option<BlockId>> =
        blocks.iter().map(|info| (info.id, info.parent)).collect();
    let mut current = Some(block);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = parents.get(&id).copied().flatten();
    }
    false
}

/// The blocks in `range` (indices in document order) that are not nested under another
/// block in the range: deleting or moving these takes their children along.
pub fn outermost_blocks(blocks: &[BlockInfo], range: Range<usize>) -> Vec<BlockId> {
    let selected: HashSet<BlockId> = blocks[range.clone()].iter().map(|block| block.id).collect();
    blocks[range]
        .iter()
        .filter(|block| {
            block
                .parent
                .is_none_or(|parent| !selected.contains(&parent))
        })
        .map(|block| block.id)
        .collect()
}

/// The sibling just before `block` under the same parent.
pub fn previous_sibling(blocks: &[BlockInfo], block: BlockId) -> Option<BlockId> {
    let (siblings, index) = siblings_of(blocks, block)?;
    index.checked_sub(1).map(|previous| siblings[previous])
}

/// The sibling just after `block` under the same parent.
pub fn next_sibling(blocks: &[BlockInfo], block: BlockId) -> Option<BlockId> {
    let (siblings, index) = siblings_of(blocks, block)?;
    siblings.get(index + 1).copied()
}

fn siblings_of(blocks: &[BlockInfo], block: BlockId) -> Option<(Vec<BlockId>, usize)> {
    let parent = blocks.iter().find(|info| info.id == block)?.parent;
    let siblings: Vec<BlockId> = blocks
        .iter()
        .filter(|info| info.parent == parent)
        .map(|info| info.id)
        .collect();
    let index = siblings.iter().position(|id| *id == block)?;
    Some((siblings, index))
}

/// Where a dragged block lands when released over `target` at a point. Over the top half it
/// goes before the target; over the bottom half after it, or inside it as the first child
/// when the pointer is pushed right by at least one indent step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropPoint {
    pub target: BlockId,
    pub is_upper_half: bool,
    pub is_indented: bool,
}

pub fn drop_placement(point: DropPoint) -> Placement {
    match (point.is_upper_half, point.is_indented) {
        (true, _) => Placement::Before(point.target),
        (false, true) => Placement::FirstChildOf(Some(point.target)),
        (false, false) => Placement::After(point.target),
    }
}

/// The smallest replacement that turns `old` into `new`: the index range of `old` that
/// changed and how many items replace it. Lets the virtual list keep its scroll position
/// and the measured heights of untouched rows.
pub fn changed_span<T: PartialEq>(old: &[T], new: &[T]) -> (Range<usize>, usize) {
    let common_prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let longest_suffix = old.len().min(new.len()) - common_prefix;
    let common_suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(longest_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    (
        common_prefix..old.len() - common_suffix,
        new.len() - common_prefix - common_suffix,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::PageDocument;

    fn page_with(kinds: &[(BlockKind, usize)]) -> (PageDocument, Vec<BlockInfo>) {
        let mut page = PageDocument::create(1, &[]).unwrap();
        let mut stack: Vec<BlockId> = Vec::new();
        for &(kind, depth) in kinds {
            stack.truncate(depth);
            let parent = stack.last().copied();
            let block = page
                .insert_block(Placement::LastChildOf(parent), kind)
                .unwrap();
            stack.push(block);
        }
        page.commit();
        let blocks = page.blocks().to_vec();
        (page, blocks)
    }

    #[test]
    fn numbering_counts_runs_and_restarts() {
        use BlockKind::*;
        let (_, blocks) = page_with(&[
            (NumberedListItem, 0),
            (NumberedListItem, 0),
            (NumberedListItem, 1),
            (NumberedListItem, 1),
            (NumberedListItem, 0),
            (Paragraph, 0),
            (NumberedListItem, 0),
        ]);
        assert_eq!(
            list_numbers(&blocks),
            vec![Some(1), Some(2), Some(1), Some(2), Some(3), None, Some(1)]
        );
    }

    #[test]
    fn outermost_blocks_skip_selected_descendants() {
        use BlockKind::*;
        let (_, blocks) = page_with(&[
            (Paragraph, 0),
            (Paragraph, 1),
            (Paragraph, 0),
            (Paragraph, 1),
        ]);
        assert_eq!(
            outermost_blocks(&blocks, 0..3),
            vec![blocks[0].id, blocks[2].id]
        );
        assert_eq!(
            outermost_blocks(&blocks, 1..4),
            vec![blocks[1].id, blocks[2].id]
        );
    }

    #[test]
    fn nesting_checks_follow_the_parent_chain() {
        use BlockKind::*;
        let (_, blocks) = page_with(&[
            (Paragraph, 0),
            (Paragraph, 1),
            (Paragraph, 2),
            (Paragraph, 0),
        ]);
        assert!(is_within(&blocks, blocks[2].id, blocks[0].id));
        assert!(is_within(&blocks, blocks[0].id, blocks[0].id));
        assert!(!is_within(&blocks, blocks[3].id, blocks[0].id));
    }

    #[test]
    fn siblings_ignore_nested_blocks() {
        use BlockKind::*;
        let (_, blocks) = page_with(&[(Paragraph, 0), (Paragraph, 1), (Paragraph, 0)]);
        assert_eq!(next_sibling(&blocks, blocks[0].id), Some(blocks[2].id));
        assert_eq!(previous_sibling(&blocks, blocks[2].id), Some(blocks[0].id));
        assert_eq!(next_sibling(&blocks, blocks[1].id), None);
    }

    #[test]
    fn drops_go_before_after_or_inside() {
        let (_, blocks) = page_with(&[(BlockKind::Paragraph, 0)]);
        let target = blocks[0].id;
        let point = |is_upper_half, is_indented| DropPoint {
            target,
            is_upper_half,
            is_indented,
        };
        assert_eq!(drop_placement(point(true, true)), Placement::Before(target));
        assert_eq!(
            drop_placement(point(false, false)),
            Placement::After(target)
        );
        assert_eq!(
            drop_placement(point(false, true)),
            Placement::FirstChildOf(Some(target))
        );
    }

    #[test]
    fn changed_span_finds_the_minimal_middle() {
        assert_eq!(changed_span(&[1, 2, 3, 4], &[1, 2, 3, 4]), (4..4, 0));
        assert_eq!(changed_span(&[1, 2, 3, 4], &[1, 9, 3, 4]), (1..2, 1));
        assert_eq!(changed_span(&[1, 2, 3], &[1, 2, 7, 3]), (2..2, 1));
        assert_eq!(changed_span(&[1, 2, 3], &[1, 3]), (1..2, 0));
        assert_eq!(changed_span(&[1, 1], &[1, 1, 1]), (2..2, 1));
        assert_eq!(changed_span::<u8>(&[], &[5]), (0..0, 1));
    }
}
