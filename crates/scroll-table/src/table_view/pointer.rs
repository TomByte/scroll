//! Resizing a column by its header edge, following the pointer anywhere in the window. The new
//! width is saved to the view when the drag ends, not on every move.

use gpui::{Context, MouseDownEvent, Pixels};
use scroll_core::id::PropertyId;

use super::{PointerDrag, TableView};
use crate::transactions::set_column_widths;

impl TableView {
    pub(super) fn start_resize(
        &mut self,
        property: PropertyId,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(column) = self
            .columns
            .columns()
            .iter()
            .find(|column| column.property == property)
        else {
            return;
        };
        self.pointer_drag = Some(PointerDrag::ResizeColumn {
            property,
            start_x: event.position.x,
            start_width: column.width,
        });
        cx.stop_propagation();
    }

    pub(super) fn drag_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        match self.pointer_drag {
            Some(PointerDrag::ResizeColumn {
                property,
                start_x,
                start_width,
            }) => {
                self.columns = self.columns.resized(property, start_width + (x - start_x));
            }
            None => return,
        }
        cx.notify();
    }

    pub(super) fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.pointer_drag.take().is_none() {
            return;
        }
        let widths = self.columns.widths_to_save();
        self.write(set_column_widths(self.model.view(), widths), cx);
        self.clamp_horizontal_offset();
        cx.notify();
    }
}
