//! Where the page is scrolled to, and what moves it besides the wheel: keeping the caret in
//! view, the scrollbar, and the smooth-scrolling setting.
//!
//! GPUI's list measures a row only once it comes near the screen, and again after every change
//! to it, and counts a row it has not measured as no height at all. Used as it is, the
//! scrollbar would show the page growing as it is read, a row that has just been made could
//! only be found by scrolling it to the top, and a move made in the frame a row changed would
//! skip that row. So this keeps each row's height as it was last drawn, takes rows not drawn
//! yet to be of average height, and brings the caret into view by the smallest move that shows
//! it, once the list has measured where it is.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, DispatchPhase, Hitbox, ListAlignment, ListOffset, ListState, Pixels,
    ScrollDelta, ScrollWheelEvent, WeakEntity, Window, px,
};
use scroll_settings::{AnySetting, Category, Setting};
use scroll_ui::{ScrollPosition, ScrollSource};

use super::PageEditor;

pub const SMOOTH_SCROLLING: Setting<bool> = Setting {
    key: "appearance.smooth_scrolling",
    category: Category::Appearance,
    label: "Smooth scrolling",
    description: "Glides between the steps of a mouse wheel instead of jumping. A trackpad already scrolls by the pixel.",
    default: false,
};

pub static SETTINGS: &[&dyn AnySetting] = &[&SMOOTH_SCROLLING];

/// How far beyond the visible area the list lays out rows, so fast scrolling does not show
/// gaps.
const LIST_OVERDRAW: Pixels = px(600.0);
/// What one line of a mouse wheel scrolls, as GPUI's list has it.
const WHEEL_LINE: Pixels = px(20.0);
/// What a row is taken to be before any has been drawn: a line of body text.
const UNDRAWN_ROW_HEIGHT: Pixels = px(28.0);
/// How many frames a caret that has no position yet is followed for. A block just made takes a
/// frame to be measured, one more to be scrolled onto the screen, and a last to be placed.
pub(super) const REVEAL_ATTEMPTS: u8 = 3;

/// How tall each row was when it was last drawn.
#[derive(Default)]
struct RowHeights {
    /// `None` for a row not drawn yet.
    heights: Vec<Option<Pixels>>,
}

impl RowHeights {
    /// Rows measured again where they stand most likely keep their height, so it is kept until
    /// they are drawn; rows that replace others, or are new, start unknown.
    fn splice(&mut self, rows: Range<usize>, count: usize) {
        if count != rows.len() {
            self.heights.splice(rows, std::iter::repeat_n(None, count));
        }
    }

    fn record(&mut self, row: usize, height: Pixels) {
        if let Some(known) = self.heights.get_mut(row) {
            *known = Some(height);
        }
    }

    fn drawn(&self, row: usize) -> Option<Pixels> {
        self.heights.get(row).copied().flatten()
    }

    /// What a row not drawn yet is taken to be: the average of those that have been.
    fn estimate(&self) -> Pixels {
        let (sum, count) = self
            .heights
            .iter()
            .flatten()
            .fold((Pixels::ZERO, 0), |(sum, count), height| {
                (sum + *height, count + 1)
            });
        if count == 0 {
            return UNDRAWN_ROW_HEIGHT;
        }
        sum / count as f32
    }

    /// The height of the rows above `row`, and of every row.
    fn extent(&self, row: usize) -> (Pixels, Pixels) {
        let estimate = self.estimate();
        let total = |rows: &[Option<Pixels>]| {
            rows.iter()
                .fold(Pixels::ZERO, |sum, height| sum + height.unwrap_or(estimate))
        };
        let row = row.min(self.heights.len());
        let above = total(&self.heights[..row]);
        (above, above + total(&self.heights[row..]))
    }

    /// The row `offset` from the top of the page falls in, and how far into it.
    fn row_at(&self, offset: Pixels) -> ListOffset {
        let estimate = self.estimate();
        let mut top = Pixels::ZERO;
        for (row, height) in self.heights.iter().enumerate() {
            let height = height.unwrap_or(estimate);
            if offset < top + height {
                return ListOffset {
                    item_ix: row,
                    offset_in_item: offset - top,
                };
            }
            top += height;
        }
        ListOffset {
            item_ix: self.heights.len(),
            offset_in_item: Pixels::ZERO,
        }
    }
}

/// The row at the top of the page area and how far it is scrolled past the top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScrollPlace {
    row: usize,
    offset: Pixels,
}

/// The page's rows as the list scrolls them. Cheap to clone: every clone is the same list.
#[derive(Clone)]
pub(super) struct PageScroll {
    list: ListState,
    heights: Rc<RefCell<RowHeights>>,
}

impl PageScroll {
    pub(super) fn new(row_count: usize) -> Self {
        PageScroll {
            list: ListState::new(row_count, ListAlignment::Top, LIST_OVERDRAW),
            heights: Rc::new(RefCell::new(RowHeights {
                heights: vec![None; row_count],
            })),
        }
    }

    pub(super) fn list(&self) -> ListState {
        self.list.clone()
    }

    pub(super) fn row_count(&self) -> usize {
        self.list.item_count()
    }

    pub(super) fn top(&self) -> ListOffset {
        self.list.logical_scroll_top()
    }

    /// Where the page is scrolled to, as a value that can be compared with an earlier one.
    pub(super) fn place(&self) -> ScrollPlace {
        let top = self.top();
        ScrollPlace {
            row: top.item_ix,
            offset: top.offset_in_item,
        }
    }

    /// Scrolls by `distance` (positive scrolls down). The list places a move by the heights it
    /// has measured, so when the row at the top is waiting to be measured again (it was just
    /// edited) the move is taken through that row by the height it was last drawn at.
    pub(super) fn scroll_by(&self, distance: Pixels) {
        let top = self.top();
        let drawn = self.heights.borrow().drawn(top.item_ix);
        let Some(height) = drawn.filter(|_| !self.is_measured(top.item_ix)) else {
            return self.list.scroll_by(distance);
        };
        let offset = top.offset_in_item + distance;
        if (Pixels::ZERO..height).contains(&offset) {
            return self.list.scroll_to(ListOffset {
                item_ix: top.item_ix,
                offset_in_item: offset,
            });
        }
        // Out of the row at its start or its end, and on from there by what is left.
        let (row, rest) = if offset < Pixels::ZERO {
            (top.item_ix, offset)
        } else {
            (top.item_ix + 1, offset - height)
        };
        self.scroll_row_to_top(row);
        self.list.scroll_by(rest);
    }

    /// Scrolls the least that shows all of `row`, a row the list has measured.
    pub(super) fn reveal_row(&self, row: usize) {
        self.list.scroll_to_reveal_item(row);
    }

    /// Puts the top of `row` at the top of the page area.
    pub(super) fn scroll_row_to_top(&self, row: usize) {
        self.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: Pixels::ZERO,
        });
    }

    /// Tells the list that `rows` were replaced by `count` rows to measure again. Rows measured
    /// again where they stand leave the page where it was; the list by itself would jump to the
    /// top of the first of them whenever the row at the top of the screen is one.
    pub(super) fn splice(&self, rows: Range<usize>, count: usize) {
        let top = self.top();
        self.list.splice(rows.clone(), count);
        self.heights.borrow_mut().splice(rows.clone(), count);
        if count == rows.len() && rows.contains(&top.item_ix) {
            self.list.scroll_to(top);
        }
    }

    /// The rows on screen as last laid out, as far as the list has measured them: a row not
    /// measured yet (one just made, say) ends the range. Notes how tall each one is.
    pub(super) fn visible_rows(&self) -> Range<usize> {
        let bottom = self.list.viewport_bounds().bottom();
        let first = self.top().item_ix;
        let mut end = first;
        let mut heights = self.heights.borrow_mut();
        while let Some(bounds) = self.list.bounds_for_item(end) {
            if bounds.top() >= bottom {
                break;
            }
            heights.record(end, bounds.size.height);
            end += 1;
        }
        first..end
    }

    /// Moves the page the least that brings `row` onto the screen; one already on it is left
    /// alone. `visible` is what `visible_rows` gave for this frame. A row the list has not
    /// measured cannot be placed: one in a run of new rows just below those on screen is left
    /// for the list to measure first, and one further off is scrolled to the top.
    pub(super) fn bring_row_into_view(&self, row: usize, visible: &Range<usize>) {
        if visible.contains(&row) {
            return;
        }
        if row < visible.start {
            return self.scroll_row_to_top(row);
        }
        if self.is_measured(row) {
            return self.reveal_row(row);
        }
        let is_in_new_run = (visible.end..row).all(|between| !self.is_measured(between));
        if !is_in_new_run {
            self.scroll_row_to_top(row);
        }
    }

    /// Whether the list has measured `row`, a row at or below the top of the screen.
    pub(super) fn is_measured(&self, row: usize) -> bool {
        self.list.bounds_for_item(row).is_some()
    }
}

impl ScrollSource for PageScroll {
    fn position(&self, _: &App) -> ScrollPosition {
        let top = self.top();
        let viewport = self.list.viewport_bounds().size.height;
        let (above, total) = self.heights.borrow().extent(top.item_ix);
        ScrollPosition {
            offset: above + top.offset_in_item,
            max_offset: (total - viewport).max(Pixels::ZERO),
            viewport,
        }
    }

    fn scroll_to(&self, offset: Pixels, _: &mut App) {
        let top = self.heights.borrow().row_at(offset);
        self.list.scroll_to(top);
    }
}

impl PageEditor {
    /// Scrolls the page by `distance` (positive scrolls down).
    pub fn scroll_by(&mut self, distance: Pixels, cx: &mut Context<Self>) {
        self.scroll.scroll_by(distance);
        cx.notify();
    }

    /// The row of the page's list that shows the block at `index`; a table's parts are shown in
    /// their table's row.
    pub(super) fn list_row(&self, index: usize) -> usize {
        self.header_rows() + self.rows.row_of(index)
    }

    /// Has the row showing the block at `index` measured again, after its height may have
    /// changed.
    pub(super) fn remeasure_row_of(&mut self, index: usize) {
        let row = self.list_row(index);
        self.scroll.splice(row..row + 1, 1);
    }

    /// Scrolls so the caret's line is inside the page area, after the caret moved. Uses the
    /// caret block's layout from the previous frame, which says where the caret is only while
    /// the page has not been scrolled since and the block's row has not changed. Otherwise (a
    /// block just made, off screen, with no text of its own, or just edited) its row is brought
    /// into view instead, and the caret is looked at again next frame.
    pub(super) fn reveal_caret_if_needed(&mut self, visible: &Range<usize>, window: &mut Window) {
        let attempts_left = std::mem::take(&mut self.reveal_attempts);
        if attempts_left == 0 {
            return;
        }
        let Some(index) = self.block_index(self.selection.block) else {
            return;
        };
        let row = self.list_row(index);
        let caret_block = &self.blocks[index];
        let is_layout_current = caret_block.layout_frame + 1 == self.frame
            && self.place_at_layout == Some(self.scroll.place())
            && self.scroll.is_measured(row);
        let current_layout = caret_block.layout.as_ref().filter(|_| is_layout_current);
        let (Some(viewport), Some(layout)) = (self.viewport, current_layout) else {
            self.scroll.bring_row_into_view(row, visible);
            self.reveal_attempts = attempts_left - 1;
            return window.request_animation_frame();
        };
        let caret_top = layout.caret_origin(self.display_selection().head).y;
        let caret_bottom = caret_top + layout.line_height();
        let margin = layout.line_height() * 2.0;
        if caret_top < viewport.top() + margin {
            self.scroll.scroll_by(caret_top - viewport.top() - margin);
        } else if caret_bottom > viewport.bottom() - margin {
            self.scroll
                .scroll_by(caret_bottom - viewport.bottom() + margin);
        }
    }

    /// Notes where the page is scrolled to as its blocks are about to be laid out, so the next
    /// frame can tell whether their positions still hold.
    pub(super) fn note_place_at_layout(&mut self) {
        self.place_at_layout = Some(self.scroll.place());
    }

    /// Moves the page along the glide a mouse wheel started, if one is under way.
    pub(super) fn advance_glide(&mut self, window: &mut Window, cx: &App) {
        if self.glide.is_moving() {
            self.scroll.scroll_by(self.glide.advance(window, cx));
        }
    }

    fn glide_by(&mut self, distance: Pixels, cx: &mut Context<Self>) {
        self.glide.push(distance);
        cx.notify();
    }
}

/// While smooth scrolling is on, takes a mouse wheel's steps over the page before the list sees
/// them and glides the page that far instead. A trackpad's movement is left to the list: it
/// arrives pixel by pixel and is smooth as it is.
pub(super) fn glide_on_wheel(
    editor: WeakEntity<PageEditor>,
    page_area: &Hitbox,
    window: &mut Window,
) {
    let page_area = page_area.id;
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        let ScrollDelta::Lines(lines) = event.delta else {
            return;
        };
        let is_over_page =
            phase == DispatchPhase::Capture && page_area.should_handle_scroll(window);
        if !is_over_page || !SMOOTH_SCROLLING.get(cx) {
            return;
        }
        let glided = editor.update(cx, |editor, cx| {
            editor.glide_by(WHEEL_LINE * -lines.y, cx);
        });
        if glided.is_ok() {
            cx.stop_propagation();
        }
    });
}

/// Where the page area is, kept for hit-testing the wheel.
pub(super) fn page_area_hitbox(bounds: Bounds<Pixels>, window: &mut Window) -> Hitbox {
    window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heights(known: &[Option<f32>]) -> RowHeights {
        RowHeights {
            heights: known.iter().map(|height| height.map(px)).collect(),
        }
    }

    #[test]
    fn rows_not_drawn_yet_count_as_the_average_of_those_drawn() {
        let rows = heights(&[Some(20.0), None, Some(40.0), None]);
        assert_eq!(rows.estimate(), px(30.0));
        assert_eq!(rows.extent(0), (px(0.0), px(120.0)));
        assert_eq!(rows.extent(2), (px(50.0), px(120.0)));
        assert_eq!(rows.extent(9), (px(120.0), px(120.0)), "past the end");
        assert_eq!(heights(&[None, None]).estimate(), UNDRAWN_ROW_HEIGHT);
    }

    #[test]
    fn an_offset_maps_back_to_the_row_it_falls_in() {
        let rows = heights(&[Some(20.0), None, Some(40.0)]);
        let at = |offset: f32| {
            let top = rows.row_at(px(offset));
            (top.item_ix, top.offset_in_item)
        };
        assert_eq!(at(0.0), (0, px(0.0)));
        assert_eq!(at(25.0), (1, px(5.0)));
        assert_eq!(at(89.0), (2, px(39.0)));
        assert_eq!(at(500.0), (3, px(0.0)), "past the end is the end");
    }

    #[test]
    fn remeasured_rows_keep_their_height_and_replaced_ones_forget_it() {
        let mut rows = heights(&[Some(20.0), Some(30.0), Some(40.0)]);
        rows.splice(1..2, 1);
        assert_eq!(rows.heights[1], Some(px(30.0)));
        rows.splice(1..2, 3);
        assert_eq!(
            rows.heights,
            [Some(px(20.0)), None, None, None, Some(px(40.0))]
        );
        rows.record(2, px(12.0));
        rows.record(99, px(12.0));
        assert_eq!(rows.heights[2], Some(px(12.0)));
    }
}
