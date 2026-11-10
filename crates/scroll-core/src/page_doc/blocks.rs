//! The block tree: kinds, placement, structural edits and reading blocks back.

use std::sync::Arc;

use loro::cursor::PosType;
use loro::{LoroMap, LoroText, LoroValue, TextDelta, TreeParentId, ValueOrContainer};

use super::text::{TextSpan, read_spans, to_unicode_offset};
use super::{
    BlockId, CHECKED_KEY, ICON_KEY, KIND_KEY, PageDoc, PageDocError, TEXT_KEY, lock, text_of,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    ToDo,
    BulletedListItem,
    NumberedListItem,
    Quote,
    Code,
    Divider,
    Callout,
}

impl BlockKind {
    const ALL: [BlockKind; 11] = [
        BlockKind::Paragraph,
        BlockKind::Heading1,
        BlockKind::Heading2,
        BlockKind::Heading3,
        BlockKind::ToDo,
        BlockKind::BulletedListItem,
        BlockKind::NumberedListItem,
        BlockKind::Quote,
        BlockKind::Code,
        BlockKind::Divider,
        BlockKind::Callout,
    ];

    /// Stored in the document, so these strings must never change.
    fn name(self) -> &'static str {
        match self {
            BlockKind::Paragraph => "paragraph",
            BlockKind::Heading1 => "heading_1",
            BlockKind::Heading2 => "heading_2",
            BlockKind::Heading3 => "heading_3",
            BlockKind::ToDo => "to_do",
            BlockKind::BulletedListItem => "bulleted_list_item",
            BlockKind::NumberedListItem => "numbered_list_item",
            BlockKind::Quote => "quote",
            BlockKind::Code => "code",
            BlockKind::Divider => "divider",
            BlockKind::Callout => "callout",
        }
    }

    /// A kind written by a newer app version reads as a paragraph, so its text still shows.
    fn from_name(name: &str) -> BlockKind {
        BlockKind::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .unwrap_or(BlockKind::Paragraph)
    }
}

/// Where a block goes. `None` as a parent means the top level of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockPlacement {
    Before(BlockId),
    After(BlockId),
    FirstChildOf(Option<BlockId>),
    LastChildOf(Option<BlockId>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    pub parent: Option<BlockId>,
    /// 0 for top-level blocks.
    pub depth: usize,
    pub kind: BlockKind,
    /// Only meaningful for to-dos; kept across kind changes so switching back restores it.
    pub is_checked: bool,
    /// An emoji shown with the block, used by callouts. Kept across kind changes.
    pub icon: Option<String>,
    pub text: String,
    pub spans: Vec<TextSpan>,
}

impl PageDoc {
    pub fn insert_block(
        &self,
        placement: BlockPlacement,
        kind: BlockKind,
    ) -> Result<BlockId, PageDocError> {
        let id = self.create_block(placement, kind)?;
        self.doc.commit();
        Ok(id)
    }

    /// Moves the text after `offset`, with its marks, into a new block of `new_kind`. The new block
    /// follows `id`, or becomes its first child if `id` has children, so it stays right below the
    /// split text. One undo step.
    pub fn split_block(
        &self,
        id: BlockId,
        offset: usize,
        new_kind: BlockKind,
    ) -> Result<BlockId, PageDocError> {
        let text = self.block_text(id)?;
        let start = to_unicode_offset(&text, id, offset)?;
        let end = text.len_unicode();
        let tail = text.slice_delta(start, end, PosType::Unicode)?;
        text.delete(start, end - start)?;
        let has_children = self.tree().children_num(id).unwrap_or(0) > 0;
        let placement = if has_children {
            BlockPlacement::FirstChildOf(Some(id))
        } else {
            BlockPlacement::After(id)
        };
        let new_block = self.create_block(placement, new_kind)?;
        self.block_text(new_block)?.apply_delta(&tail)?;
        self.doc.commit();
        Ok(new_block)
    }

    /// Appends the block's text, with its marks, to the block above it in document order, and
    /// removes the block. Its children move up one level into its place, keeping document order.
    /// Returns the block merged into and the byte offset where the two texts meet, or `None` for
    /// the first block on the page. One undo step.
    pub fn merge_into_previous(
        &self,
        id: BlockId,
    ) -> Result<Option<(BlockId, usize)>, PageDocError> {
        self.require_live_block(id)?;
        let Some(previous) = self.block_above(id)? else {
            return Ok(None);
        };
        let previous_text = self.block_text(previous)?;
        let join_offset = previous_text.len_utf8();
        let mut delta = vec![TextDelta::Retain {
            retain: previous_text.len_unicode(),
            attributes: None,
        }];
        delta.extend(self.block_text(id)?.to_delta());
        previous_text.apply_delta(&delta)?;
        self.lift_children(id)?;
        self.tree().delete(id)?;
        self.doc.commit();
        Ok(Some((previous, join_offset)))
    }

    /// Deletes the block and everything nested under it.
    pub fn delete_block(&self, id: BlockId) -> Result<(), PageDocError> {
        self.require_live_block(id)?;
        self.tree().delete(id)?;
        self.doc.commit();
        Ok(())
    }

    /// Moves the block with everything nested under it. Moving a block under itself is an error.
    pub fn move_block(&self, id: BlockId, placement: BlockPlacement) -> Result<(), PageDocError> {
        self.require_live_block(id)?;
        let tree = self.tree();
        match placement {
            BlockPlacement::Before(sibling) => {
                self.require_live_block(sibling)?;
                tree.mov_before(id, sibling)?;
            }
            BlockPlacement::After(sibling) => {
                self.require_live_block(sibling)?;
                tree.mov_after(id, sibling)?;
            }
            BlockPlacement::FirstChildOf(parent) => tree.mov_to(id, self.parent_id(parent)?, 0)?,
            BlockPlacement::LastChildOf(parent) => {
                let parent = self.parent_id(parent)?;
                let child_count = tree.children_num(parent).unwrap_or(0);
                // The index is where the block ends up after leaving its old place.
                let index = if tree.parent(id) == Some(parent) {
                    child_count - 1
                } else {
                    child_count
                };
                tree.mov_to(id, parent, index)?;
            }
        }
        self.doc.commit();
        Ok(())
    }

    pub fn set_block_kind(&self, id: BlockId, kind: BlockKind) -> Result<(), PageDocError> {
        self.block_meta(id)?.insert(KIND_KEY, kind.name())?;
        self.doc.commit();
        Ok(())
    }

    /// Sets or, with `None`, removes the block's icon.
    pub fn set_block_icon(&self, id: BlockId, icon: Option<&str>) -> Result<(), PageDocError> {
        let meta = self.block_meta(id)?;
        match icon {
            Some(icon) => meta.insert(ICON_KEY, icon)?,
            None => meta.delete(ICON_KEY)?,
        }
        self.doc.commit();
        Ok(())
    }

    pub fn toggle_to_do(&self, id: BlockId) -> Result<(), PageDocError> {
        let meta = self.block_meta(id)?;
        if read_kind(&meta) != BlockKind::ToDo {
            return Err(PageDocError::NotAToDo(id));
        }
        meta.insert(CHECKED_KEY, !read_is_checked(&meta))?;
        self.doc.commit();
        Ok(())
    }

    /// Every block in document order: each block is followed by the blocks nested under it.
    /// Cheap to call often: blocks are kept between calls and only changed ones are re-read.
    pub fn blocks(&self) -> Result<Vec<Arc<Block>>, PageDocError> {
        self.bring_cache_up_to_date()?;
        Ok(self.cache.borrow().blocks())
    }

    pub fn block(&self, id: BlockId) -> Result<Arc<Block>, PageDocError> {
        self.bring_cache_up_to_date()?;
        self.cache
            .borrow()
            .block(id)
            .ok_or(PageDocError::BlockNotFound(id))
    }

    fn bring_cache_up_to_date(&self) -> Result<(), PageDocError> {
        let pending = std::mem::take(&mut *lock(&self.pending_changes));
        if self.cache.borrow().needs_rebuild(&pending) {
            let mut blocks = Vec::new();
            self.collect_blocks(TreeParentId::Root, 0, &mut blocks)?;
            self.cache.borrow_mut().rebuild(blocks);
            return Ok(());
        }
        let touched: Vec<(BlockId, usize)> = self.cache.borrow().touched(&pending).collect();
        for (id, depth) in touched {
            let block = self.read_block(id, depth)?;
            self.cache.borrow_mut().replace(block);
        }
        Ok(())
    }

    /// The text of all blocks, one per line, for search.
    pub fn plain_text(&self) -> Result<String, PageDocError> {
        let texts: Vec<String> = self
            .blocks()?
            .iter()
            .map(|block| block.text.clone())
            .collect();
        Ok(texts.join("\n"))
    }

    /// Creates a block without committing, for edits that commit once at the end.
    fn create_block(
        &self,
        placement: BlockPlacement,
        kind: BlockKind,
    ) -> Result<BlockId, PageDocError> {
        let (parent, index) = self.insertion_point(placement)?;
        let id = self.tree().create_at(parent, index)?;
        let meta = self.tree().get_meta(id)?;
        meta.insert(KIND_KEY, kind.name())?;
        meta.insert_container(TEXT_KEY, LoroText::new())?;
        Ok(id)
    }

    /// The block just above in document order: the previous sibling's deepest last descendant,
    /// or the parent for a first child.
    fn block_above(&self, id: BlockId) -> Result<Option<BlockId>, PageDocError> {
        let tree = self.tree();
        let parent = tree.parent(id).ok_or(PageDocError::BlockNotFound(id))?;
        let siblings = tree.children(parent).unwrap_or_default();
        let index = siblings
            .iter()
            .position(|sibling| *sibling == id)
            .ok_or(PageDocError::BlockNotFound(id))?;
        let Some(index_above) = index.checked_sub(1) else {
            return Ok(match parent {
                TreeParentId::Node(parent) => Some(parent),
                _ => None,
            });
        };
        let mut above = siblings[index_above];
        while let Some(last_child) = tree
            .children(above)
            .and_then(|children| children.last().copied())
        {
            above = last_child;
        }
        Ok(Some(above))
    }

    /// Moves the block's children to just after it, under its parent, in order.
    fn lift_children(&self, id: BlockId) -> Result<(), PageDocError> {
        let tree = self.tree();
        let parent = tree.parent(id).ok_or(PageDocError::BlockNotFound(id))?;
        let index = tree
            .children(parent)
            .unwrap_or_default()
            .iter()
            .position(|sibling| *sibling == id)
            .ok_or(PageDocError::BlockNotFound(id))?;
        for (offset, child) in tree
            .children(id)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            tree.mov_to(child, parent, index + 1 + offset)?;
        }
        Ok(())
    }

    fn collect_blocks(
        &self,
        parent: TreeParentId,
        depth: usize,
        blocks: &mut Vec<Block>,
    ) -> Result<(), PageDocError> {
        for id in self.tree().children(parent).unwrap_or_default() {
            blocks.push(self.read_block(id, depth)?);
            self.collect_blocks(TreeParentId::Node(id), depth + 1, blocks)?;
        }
        Ok(())
    }

    fn read_block(&self, id: BlockId, depth: usize) -> Result<Block, PageDocError> {
        let meta = self.tree().get_meta(id)?;
        let block_text = text_of(&meta).ok_or(PageDocError::MalformedBlock(id))?;
        let parent = match self.tree().parent(id) {
            Some(TreeParentId::Node(parent)) => Some(parent),
            _ => None,
        };
        Ok(Block {
            id,
            parent,
            depth,
            kind: read_kind(&meta),
            is_checked: read_is_checked(&meta),
            icon: read_icon(&meta),
            text: block_text.to_string(),
            spans: read_spans(&block_text),
        })
    }

    fn insertion_point(
        &self,
        placement: BlockPlacement,
    ) -> Result<(TreeParentId, usize), PageDocError> {
        let tree = self.tree();
        match placement {
            BlockPlacement::Before(sibling) => self.sibling_slot(sibling),
            BlockPlacement::After(sibling) => {
                let (parent, index) = self.sibling_slot(sibling)?;
                Ok((parent, index + 1))
            }
            BlockPlacement::FirstChildOf(parent) => Ok((self.parent_id(parent)?, 0)),
            BlockPlacement::LastChildOf(parent) => {
                let parent = self.parent_id(parent)?;
                Ok((parent, tree.children_num(parent).unwrap_or(0)))
            }
        }
    }

    /// The sibling's parent and its index among that parent's children.
    fn sibling_slot(&self, sibling: BlockId) -> Result<(TreeParentId, usize), PageDocError> {
        self.require_live_block(sibling)?;
        let tree = self.tree();
        let parent = tree
            .parent(sibling)
            .ok_or(PageDocError::BlockNotFound(sibling))?;
        let index = tree
            .children(parent)
            .unwrap_or_default()
            .iter()
            .position(|candidate| *candidate == sibling)
            .ok_or(PageDocError::BlockNotFound(sibling))?;
        Ok((parent, index))
    }

    fn parent_id(&self, parent: Option<BlockId>) -> Result<TreeParentId, PageDocError> {
        match parent {
            Some(id) => {
                self.require_live_block(id)?;
                Ok(TreeParentId::Node(id))
            }
            None => Ok(TreeParentId::Root),
        }
    }

    pub(super) fn require_live_block(&self, id: BlockId) -> Result<(), PageDocError> {
        let tree = self.tree();
        if !tree.contains(id) || tree.is_node_deleted(&id)? {
            return Err(PageDocError::BlockNotFound(id));
        }
        Ok(())
    }
}

fn read_kind(meta: &LoroMap) -> BlockKind {
    match meta.get(KIND_KEY) {
        Some(ValueOrContainer::Value(LoroValue::String(name))) => BlockKind::from_name(&name),
        _ => BlockKind::Paragraph,
    }
}

fn read_is_checked(meta: &LoroMap) -> bool {
    matches!(
        meta.get(CHECKED_KEY),
        Some(ValueOrContainer::Value(LoroValue::Bool(true)))
    )
}

fn read_icon(meta: &LoroMap) -> Option<String> {
    match meta.get(ICON_KEY) {
        Some(ValueOrContainer::Value(LoroValue::String(icon))) => Some(icon.to_string()),
        _ => None,
    }
}
