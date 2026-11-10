//! Reordering blocks by dragging their handle, including into and out of nesting. While a drag
//! is over the page the carried rows are dimmed in place and the rows where they would land
//! part to open a gap the size of what is carried (up to a cap).

use std::ops::Range;

use gpui::{
    AnyElement, App, Context, DragMoveEvent, ElementId, IntoElement, ParentElement, Pixels, Point,
    SharedString, Styled, Window, div,
};
use scroll_ui::{DropIndicator, Theme, Transition};

use super::render::top_spacing;
use super::structure::{DropPoint, drop_placement, is_within};
use super::{NEST_INDENT, PageEditor};
use crate::editor::document::{BlockId, Placement};

/// The tallest gap opened for a drag, in body lines, so a long selection does not push the
/// rest of the page out of view.
const MAX_GAP_LINES: f32 = 4.0;

/// What is being dragged: one block, or the selected blocks.
#[derive(Clone)]
pub struct DraggedBlocks {
    blocks: Vec<BlockId>,
    pub(super) label: SharedString,
}

/// Where the dragged blocks would land, and how to show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DropTarget {
    placement: Placement,
    /// The block the drop is beside, which draws the line when there is no gap.
    indicator_block: BlockId,
    indicator_depth: usize,
    /// The block whose row moves down to open a gap above it where the drop lands; `None`
    /// below the last block, where there is nothing to move aside.
    pub(super) gap_above: Option<BlockId>,
}

/// How the rows look while blocks are dragged over the page. Kept from the drag's start until
/// the rows have settled after it ends; while it is `None` rows pay nothing for it.
pub(super) struct DragMotion {
    /// The carried rows (indices in document order): the dragged blocks and everything
    /// nested under them.
    lifted: Range<usize>,
    /// How far rows part to open the gap. Zero once dropped: the blocks fill it themselves.
    gap_height: Pixels,
    /// Whether a drag was in progress as of this frame.
    is_dragging: bool,
    /// Whether any row was still animating in this frame.
    is_moving: bool,
}

/// One row's share of the drag motion this frame.
pub(super) struct RowMotion {
    /// The row's opacity: dimmed while it is carried.
    pub(super) opacity: f32,
    /// The gap opened above the row, with the drop line in it if the drop lands there.
    pub(super) gap: Option<AnyElement>,
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

    /// A drag of `dragged` began: its rows are picked up. Starting here rather than at the
    /// first target lets the rows be drawn once without a gap, so the first gap animates open.
    pub(super) fn start_drag_motion(&mut self, dragged: &DraggedBlocks, theme: &Theme) {
        let lifted = self.carried_rows(&dragged.blocks);
        self.drag_motion = Some(DragMotion {
            gap_height: self.gap_height(lifted.clone(), theme),
            lifted,
            is_dragging: true,
            is_moving: false,
        });
    }

    pub(super) fn drag_blocks_over(
        &mut self,
        event: &DragMoveEvent<DraggedBlocks>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dragged = &event.drag(cx).blocks;
        let new_target = self.drop_target_at(event.event.position, dragged);
        if new_target != self.drop_target {
            self.drop_target = new_target;
            cx.notify();
        }
    }

    /// Where `dragged` would land with the pointer at `position`, judged by the rows laid out
    /// in the last frame. A row reaches up to the text of the block above it, so the gap
    /// opened above a row is part of that row: as the gap opens under a still pointer, the
    /// target stays put.
    pub(super) fn drop_target_at(
        &self,
        position: Point<Pixels>,
        dragged: &[BlockId],
    ) -> Option<DropTarget> {
        if !self
            .viewport
            .is_some_and(|viewport| viewport.contains(&position))
        {
            return None;
        }
        let (index, bounds) = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, state)| state.layout_frame == self.frame)
            .filter_map(|(index, state)| Some((index, state.layout.as_ref()?.bounds())))
            .find(|(_, bounds)| position.y <= bounds.bottom())?;
        let info = self.blocks[index].info;
        let infos = self.block_infos();
        if dragged
            .iter()
            .any(|block| is_within(&infos, info.id, *block))
        {
            return None;
        }
        let point = DropPoint {
            target: info.id,
            is_upper_half: position.y < bounds.center().y,
            is_indented: position.x > bounds.left() + NEST_INDENT,
        };
        let gap_above = if point.is_upper_half {
            Some(info.id)
        } else {
            self.blocks.get(index + 1).map(|next| next.info.id)
        };
        Some(DropTarget {
            placement: drop_placement(point),
            indicator_block: info.id,
            indicator_depth: info.depth + usize::from(!point.is_upper_half && point.is_indented),
            gap_above,
        })
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
        // The moved blocks now stand where the gap was, so it goes at once rather than closing
        // under them.
        if let Some(motion) = self.drag_motion.as_mut() {
            motion.gap_height = Pixels::ZERO;
        }
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

    /// The accent line showing where a drag would drop, if it is below `block` with no gap to
    /// draw it in (below the last block).
    pub(super) fn drop_indicator_for(&self, block: BlockId) -> Option<AnyElement> {
        let target = self
            .drop_target
            .filter(|target| target.gap_above.is_none() && target.indicator_block == block)?;
        let inset = NEST_INDENT * target.indicator_depth as f32;
        Some(DropIndicator::below().inset(inset).into_any_element())
    }

    /// Called once per frame before rows are drawn: notes whether a drag is still in progress,
    /// and lets the drag motion go once nothing is left to animate after it ended.
    pub(super) fn begin_drag_motion_frame(&mut self, cx: &App) {
        let Some(motion) = self.drag_motion.as_mut() else {
            return;
        };
        let is_dragging = cx.has_active_drag() || self.drop_target.is_some();
        if !is_dragging && !motion.is_dragging && !motion.is_moving {
            self.drag_motion = None;
            return;
        }
        motion.is_dragging = is_dragging;
        motion.is_moving = false;
    }

    /// How the row of the block at `index` is drawn this frame while blocks are dragged or
    /// settling; `None` when no drag motion is under way.
    pub(super) fn row_motion(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &App,
    ) -> Option<RowMotion> {
        let motion = self.drag_motion.as_ref()?;
        let block = self.blocks[index].info.id;
        let is_lifted = motion.is_dragging && motion.lifted.contains(&index);
        let target = self
            .drop_target
            .filter(|target| target.gap_above == Some(block));
        let lift_progress = Transition::Reorder.progress(
            ElementId::Name(format!("drag-lift-{block}").into()),
            is_lifted,
            window,
            cx,
        );
        let gap_progress = Transition::Reorder.progress(
            ElementId::Name(format!("drop-gap-{block}").into()),
            target.is_some(),
            window,
            cx,
        );
        let gap_height = motion.gap_height * toward(target.is_some(), gap_progress);
        let lifted = toward(is_lifted, lift_progress);
        let lifted_opacity = Theme::current(window, cx).disabled_opacity;
        if let Some(motion) = self.drag_motion.as_mut() {
            motion.is_moving |= lift_progress < 1.0 || gap_progress < 1.0;
        }
        Some(RowMotion {
            opacity: 1.0 - (1.0 - lifted_opacity) * lifted,
            gap: (gap_height > Pixels::ZERO).then(|| render_gap(gap_height, target)),
        })
    }

    /// The rows carried by a drag of `dragged`: from the first dragged block to the end of
    /// whatever is nested under the last.
    fn carried_rows(&self, dragged: &[BlockId]) -> Range<usize> {
        let (Some(first), Some(last)) = (
            dragged.first().and_then(|block| self.block_index(*block)),
            dragged.last().and_then(|block| self.block_index(*block)),
        ) else {
            return 0..0;
        };
        let depth = self.blocks[last].info.depth;
        let nested = self.blocks[last + 1..]
            .iter()
            .take_while(|state| state.info.depth > depth)
            .count();
        first..last + 1 + nested
    }

    /// The gap to open for the carried `rows`: their height as last laid out, at least one
    /// line and at most `MAX_GAP_LINES`.
    fn gap_height(&self, rows: Range<usize>, theme: &Theme) -> Pixels {
        let line = theme.text.body.line_height;
        let most = line * MAX_GAP_LINES;
        let mut height = Pixels::ZERO;
        for state in &self.blocks[rows] {
            if height >= most {
                break;
            }
            if let Some(layout) = state.layout.as_ref() {
                height += top_spacing(state.info.kind, theme) + layout.bounds().size.height;
            }
        }
        height.clamp(line, most)
    }
}

/// From 0 to 1, how far a transition toward `is_on` has brought a two-state value to on.
fn toward(is_on: bool, progress: f32) -> f32 {
    if is_on { progress } else { 1.0 - progress }
}

/// The space opened above a row, with the drop line across its middle if `target` lands there.
fn render_gap(height: Pixels, target: Option<DropTarget>) -> AnyElement {
    let line = target
        .map(|target| DropIndicator::above().inset(NEST_INDENT * target.indicator_depth as f32));
    div()
        .relative()
        .w_full()
        .h(height)
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(height / 2.0)
                .children(line),
        )
        .into_any_element()
}
