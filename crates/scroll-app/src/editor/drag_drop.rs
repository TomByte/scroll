//! Reordering blocks by dragging their handle, including into and out of nesting.

use gpui::{AnyElement, Context, DragMoveEvent, IntoElement, SharedString, Window};
use scroll_ui::{DropIndicator, Theme};

use super::structure::{DropPoint, drop_placement, is_within};
use super::{NEST_INDENT, PageEditor};
use crate::editor::document::{BlockId, Placement};

/// What is being dragged: one block, or the selected blocks.
#[derive(Clone)]
pub struct DraggedBlocks {
    blocks: Vec<BlockId>,
    pub(super) label: SharedString,
}

/// Where the dragged blocks would land, and where to draw the line showing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DropTarget {
    placement: Placement,
    indicator_block: BlockId,
    is_above: bool,
    indicator_depth: usize,
}

impl PageEditor {
    /// The blocks a drag starting on row `index` carries: the selection if the row is part
    /// of it, otherwise just that block.
    pub(super) fn blocks_to_drag(&self, index: usize) -> DraggedBlocks {
        let blocks = if self.is_block_selected(index) {
            self.target_blocks()
        } else {
            vec![self.blocks[index].info.id]
        };
        let first_text = self.blocks[index]
            .content
            .text
            .lines()
            .next()
            .unwrap_or_default();
        let label = match blocks.len() {
            1 if first_text.is_empty() => "Empty block".to_string(),
            1 => first_text.chars().take(48).collect(),
            count => format!("{count} blocks"),
        };
        DraggedBlocks {
            blocks,
            label: label.into(),
        }
    }

    pub(super) fn drag_blocks_over(
        &mut self,
        event: &DragMoveEvent<DraggedBlocks>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = Theme::current(window, cx);
        let dragged = event.drag(cx).blocks.clone();
        let position = event.event.position;
        let infos = self.block_infos();
        let target = self.blocks.iter().find_map(|block| {
            let bounds = block.layout.as_ref()?.bounds();
            let top = bounds.top() - drop_reach_above(&theme);
            (position.y >= top && position.y <= bounds.bottom()).then_some((block.info, bounds))
        });
        let new_target = target.and_then(|(info, bounds)| {
            let is_inside_dragged = dragged
                .iter()
                .any(|block| is_within(&infos, info.id, *block));
            if is_inside_dragged {
                return None;
            }
            let point = DropPoint {
                target: info.id,
                is_upper_half: position.y < bounds.center().y,
                is_indented: position.x > bounds.left() + NEST_INDENT,
            };
            Some(DropTarget {
                placement: drop_placement(point),
                indicator_block: info.id,
                is_above: point.is_upper_half,
                indicator_depth: info.depth
                    + usize::from(!point.is_upper_half && point.is_indented),
            })
        });
        if new_target != self.drop_target {
            self.drop_target = new_target;
            cx.notify();
        }
    }

    pub(super) fn drop_blocks(
        &mut self,
        dragged: &DraggedBlocks,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.drop_target.take() else {
            return;
        };
        self.move_dragged(&dragged.blocks, target.placement, cx);
    }

    /// Moves `blocks`, in order, so the first lands at `placement` and the rest follow it. The
    /// moves are one edit, so one undo step. A block selection that was moved (by dragging one
    /// of its blocks' handle) stays selected.
    pub(super) fn move_dragged(
        &mut self,
        blocks: &[BlockId],
        placement: Placement,
        cx: &mut Context<Self>,
    ) {
        let selection = self.selection;
        let moved_selection = self
            .block_selection
            .filter(|_| blocks == self.target_blocks().as_slice());
        self.apply_edit(cx, |page| {
            let mut placement = placement;
            for block in blocks {
                page.move_block(*block, placement)?;
                placement = Placement::After(*block);
            }
            Ok(selection)
        });
        if moved_selection.is_some() {
            self.block_selection = moved_selection;
        }
    }

    pub(super) fn clear_drop_target(&mut self, cx: &mut Context<Self>) {
        if self.drop_target.take().is_some() {
            cx.notify();
        }
    }

    /// The accent line showing where a drag would drop, if it is next to `block`.
    pub(super) fn drop_indicator_for(&self, block: BlockId) -> Option<AnyElement> {
        let target = self
            .drop_target
            .filter(|target| target.indicator_block == block)?;
        let inset = NEST_INDENT * target.indicator_depth as f32;
        let indicator = if target.is_above {
            DropIndicator::above()
        } else {
            DropIndicator::below()
        };
        Some(indicator.inset(inset).into_any_element())
    }
}

/// How far above a block a drop still lands on it, so the gaps between blocks are targets too.
fn drop_reach_above(theme: &Theme) -> gpui::Pixels {
    theme.spacing.sm
}
