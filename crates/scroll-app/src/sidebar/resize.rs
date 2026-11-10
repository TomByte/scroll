//! Resizing the sidebar by its right edge, following the pointer anywhere in the window. The
//! window keeps the width when the drag ends; a double click on the edge resets it.

use gpui::{
    Context, DispatchPhase, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Styled, canvas, px,
};

use super::{Sidebar, SidebarEvent};

pub(super) const DEFAULT_WIDTH: Pixels = px(248.0);
const MIN_WIDTH: Pixels = px(200.0);
const MAX_WIDTH: Pixels = px(480.0);

/// A drag of the sidebar's edge in progress.
#[derive(Clone, Copy)]
pub(super) struct Resizing {
    start_x: Pixels,
    start_width: Pixels,
}

/// `width` kept between the narrowest and widest the sidebar may be.
pub(super) fn clamp_width(width: Pixels) -> Pixels {
    width.clamp(MIN_WIDTH, MAX_WIDTH)
}

impl Sidebar {
    pub(super) fn start_resizing(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if event.click_count == 2 {
            self.resizing = None;
            self.width = DEFAULT_WIDTH;
            cx.emit(SidebarEvent::Resized(self.width));
            return cx.notify();
        }
        self.resizing = Some(Resizing {
            start_x: event.position.x,
            start_width: self.width,
        });
    }

    fn resize_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(resizing) = self.resizing else {
            return;
        };
        self.width = clamp_width(resizing.start_width + (x - resizing.start_x));
        cx.notify();
    }

    fn end_resizing(&mut self, cx: &mut Context<Self>) {
        if self.resizing.take().is_some() {
            cx.emit(SidebarEvent::Resized(self.width));
        }
    }

    /// While the edge is being dragged, follows the pointer anywhere in the window until
    /// release.
    pub(super) fn resize_tracker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let on_move = sidebar.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && let Some(sidebar) = on_move.upgrade()
                    {
                        sidebar.update(cx, |sidebar, cx| sidebar.resize_to(event.position.x, cx));
                    }
                });
                let on_up = sidebar.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && let Some(sidebar) = on_up.upgrade()
                    {
                        sidebar.update(cx, |sidebar, cx| sidebar.end_resizing(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_width_stays_between_its_limits() {
        assert_eq!(clamp_width(px(100.0)), MIN_WIDTH);
        assert_eq!(clamp_width(px(300.0)), px(300.0));
        assert_eq!(clamp_width(px(2000.0)), MAX_WIDTH);
    }
}
