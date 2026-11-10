use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, AppContext, Axis, Bounds, Context, DispatchPhase, ElementId,
    InteractiveElement, IntoElement, ListAlignment, ListState, MouseButton, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, RenderOnce, Styled,
    UniformListScrollHandle, Window, canvas, div, list, point, prelude::FluentBuilder, px,
    uniform_list,
};

use crate::display::icon::IconName;
use crate::lists::list_item::ListItem;
use crate::theme::Theme;

/// How far a scrolled area has moved along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollPosition {
    /// From the start of the content; 0 at the start, `max_offset` at the end.
    pub offset: Pixels,
    pub max_offset: Pixels,
    /// The length of the visible part.
    pub viewport: Pixels,
}

/// Something a `Scrollbar` can show and move. The kit provides it for GPUI's `ListState` and
/// `UniformListScrollHandle` (vertically); a view that scrolls by itself implements it.
pub trait ScrollSource: 'static {
    fn position(&self, cx: &App) -> ScrollPosition;
    fn scroll_to(&self, offset: Pixels, cx: &mut App);
    fn drag_started(&self, _: &mut App) {}
    fn drag_ended(&self, _: &mut App) {}
}

impl ScrollSource for ListState {
    fn position(&self, _: &App) -> ScrollPosition {
        ScrollPosition {
            offset: -self.scroll_px_offset_for_scrollbar().y,
            max_offset: self.max_offset_for_scrollbar().height,
            viewport: self.viewport_bounds().size.height,
        }
    }

    fn scroll_to(&self, offset: Pixels, _: &mut App) {
        self.set_offset_from_scrollbar(point(px(0.0), -offset));
    }

    fn drag_started(&self, _: &mut App) {
        self.scrollbar_drag_started();
    }

    fn drag_ended(&self, _: &mut App) {
        self.scrollbar_drag_ended();
    }
}

impl ScrollSource for UniformListScrollHandle {
    fn position(&self, _: &App) -> ScrollPosition {
        let state = self.0.borrow();
        let content = state
            .last_item_size
            .map_or(px(0.0), |size| size.contents.height);
        let viewport = state.base_handle.bounds().size.height;
        ScrollPosition {
            offset: -state.base_handle.offset().y,
            max_offset: content - viewport,
            viewport,
        }
    }

    fn scroll_to(&self, offset: Pixels, _: &mut App) {
        let scroll = self.0.borrow().base_handle.clone();
        let x = scroll.offset().x;
        scroll.set_offset(point(x, -offset));
    }
}

/// A thin overlay scrollbar. Place it as an absolutely positioned sibling over the scrolled
/// area: vertical bars sit at the right edge, horizontal ones at the bottom. It shows a thumb
/// only when the content overflows, and the thumb can be dragged.
#[derive(IntoElement)]
pub struct Scrollbar {
    id: ElementId,
    source: Rc<dyn ScrollSource>,
    axis: Axis,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, source: impl ScrollSource) -> Self {
        Scrollbar {
            id: id.into(),
            source: Rc::new(source),
            axis: Axis::Vertical,
        }
    }

    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }
}

/// Where a drag on the thumb started, kept across frames.
#[derive(Default)]
struct DragState {
    grab_offset: Cell<Option<Pixels>>,
    track: Cell<Option<Bounds<Pixels>>>,
}

/// Where the thumb is and how long it is, along the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ThumbGeometry {
    start: Pixels,
    length: Pixels,
}

fn thumb_geometry(position: ScrollPosition, track: Pixels, minimum: Pixels) -> ThumbGeometry {
    let content = position.viewport + position.max_offset;
    let length = (track * (position.viewport / content)).max(minimum);
    let start = (track - length) * (position.offset / position.max_offset).clamp(0.0, 1.0);
    ThumbGeometry { start, length }
}

/// The scroll offset for the pointer at `pointer` along a track, with the thumb grabbed at
/// `grab` from its start.
fn offset_for_pointer(
    pointer: Pixels,
    track: (Pixels, Pixels),
    thumb_length: Pixels,
    grab: Pixels,
    max_offset: Pixels,
) -> Pixels {
    let (track_start, track_length) = track;
    let travel = (track_length - thumb_length).max(px(1.0));
    max_offset * ((pointer - track_start - grab) / travel).clamp(0.0, 1.0)
}

fn along(axis: Axis, point: Point<Pixels>) -> Pixels {
    match axis {
        Axis::Vertical => point.y,
        Axis::Horizontal => point.x,
    }
}

fn track_of(axis: Axis, bounds: Bounds<Pixels>) -> (Pixels, Pixels) {
    match axis {
        Axis::Vertical => (bounds.top(), bounds.size.height),
        Axis::Horizontal => (bounds.left(), bounds.size.width),
    }
}

impl RenderOnce for Scrollbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let state = window
            .use_keyed_state(self.id.clone(), cx, |_, _| Rc::new(DragState::default()))
            .read(cx)
            .clone();
        let position = self.source.position(cx);
        if position.max_offset <= px(0.0) || position.viewport <= px(0.0) {
            return div().into_any_element();
        }
        let axis = self.axis;
        let thumb = thumb_geometry(position, position.viewport, theme.spacing.xxxl);
        let is_dragging = state.grab_offset.get().is_some();

        let drag_state = state.clone();
        let drag_source = self.source.clone();
        let on_move = move |event: &MouseMoveEvent,
                            phase: DispatchPhase,
                            window: &mut Window,
                            cx: &mut App| {
            let (Some(grab), Some(track)) = (drag_state.grab_offset.get(), drag_state.track.get())
            else {
                return;
            };
            if phase != DispatchPhase::Bubble {
                return;
            }
            let offset = offset_for_pointer(
                along(axis, event.position),
                track_of(axis, track),
                thumb.length,
                grab,
                position.max_offset,
            );
            drag_source.scroll_to(offset, cx);
            window.refresh();
        };
        let release_state = state.clone();
        let release_source = self.source.clone();
        let on_release =
            move |_: &MouseUpEvent, _: DispatchPhase, window: &mut Window, cx: &mut App| {
                if release_state.grab_offset.take().is_some() {
                    release_source.drag_ended(cx);
                    window.refresh();
                }
            };
        let grab_state = state.clone();
        let grab_source = self.source.clone();
        let track_state = state.clone();
        let bar_thickness = theme.size.scrollbar_width + theme.spacing.xs * 2.0;

        div()
            .id(self.id)
            .absolute()
            .map(|bar| match axis {
                Axis::Vertical => bar.top_0().right_0().h_full().w(bar_thickness),
                Axis::Horizontal => bar.left_0().bottom_0().w_full().h(bar_thickness),
            })
            .child(
                canvas(
                    move |bounds, _, _| track_state.track.set(Some(bounds)),
                    // Window-wide listeners, so a drag keeps working once the pointer leaves the bar.
                    move |_, _, window, _| {
                        window.on_mouse_event(on_move);
                        window.on_mouse_event(on_release);
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("thumb")
                    .absolute()
                    .map(|bar| match axis {
                        Axis::Vertical => bar
                            .top(thumb.start)
                            .right(theme.spacing.xs)
                            .w(theme.size.scrollbar_width)
                            .h(thumb.length),
                        Axis::Horizontal => bar
                            .left(thumb.start)
                            .bottom(theme.spacing.xs)
                            .h(theme.size.scrollbar_width)
                            .w(thumb.length),
                    })
                    .rounded(theme.size.scrollbar_width)
                    .bg(theme.colors.scrollbar_thumb)
                    .when(is_dragging, |thumb| thumb.bg(theme.colors.text_placeholder))
                    .hover(|thumb| thumb.bg(theme.colors.text_placeholder))
                    .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                        cx.stop_propagation();
                        let track_start = grab_state
                            .track
                            .get()
                            .map_or(px(0.0), |track| track_of(axis, track).0);
                        grab_state.grab_offset.set(Some(
                            along(axis, event.position) - track_start - thumb.start,
                        ));
                        grab_source.drag_started(cx);
                    }),
            )
            .into_any_element()
    }
}

const STORY_ROWS: usize = 200;

impl Scrollbar {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| ScrollbarStory {
            list: ListState::new(STORY_ROWS, ListAlignment::Top, px(200.0)),
            uniform: UniformListScrollHandle::new(),
        })
        .into()
    }
}

struct ScrollbarStory {
    list: ListState,
    uniform: UniformListScrollHandle,
}

impl Render for ScrollbarStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let frame = || {
            div()
                .relative()
                .flex_1()
                .h(theme.size.popover_min_width * 0.8)
                .rounded(theme.radius.md)
                .border(theme.size.border_width)
                .border_color(theme.colors.border)
        };
        let row = |index: usize, icon: IconName| -> AnyElement {
            ListItem::new(("row", index), format!("Row {}", index + 1))
                .icon(icon)
                .into_any_element()
        };
        div()
            .flex()
            .gap(theme.spacing.md)
            .child(
                frame()
                    .child(
                        list(self.list.clone(), move |index, _, _| {
                            row(index, IconName::Page)
                        })
                        .size_full()
                        .p(theme.spacing.xs),
                    )
                    .child(Scrollbar::new("list-scrollbar", self.list.clone())),
            )
            .child(
                frame()
                    .child(
                        uniform_list(
                            "uniform-rows",
                            STORY_ROWS,
                            cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                                range
                                    .map(|index| row(index, IconName::Table))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(self.uniform.clone())
                        .size_full()
                        .p(theme.spacing.xs),
                    )
                    .child(Scrollbar::new("uniform-scrollbar", self.uniform.clone())),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset: f32, max_offset: f32, viewport: f32) -> ScrollPosition {
        ScrollPosition {
            offset: px(offset),
            max_offset: px(max_offset),
            viewport: px(viewport),
        }
    }

    #[test]
    fn the_thumb_shows_the_visible_share_at_its_place() {
        // A 100-long view of 400 of content, a quarter of the way down.
        let thumb = thumb_geometry(at(75.0, 300.0, 100.0), px(100.0), px(10.0));
        assert_eq!(
            thumb,
            ThumbGeometry {
                start: px(18.75),
                length: px(25.0)
            }
        );
        let short = thumb_geometry(at(0.0, 9_900.0, 100.0), px(100.0), px(10.0));
        assert_eq!(short.length, px(10.0), "never shorter than the minimum");
    }

    #[test]
    fn dragging_the_thumb_maps_the_pointer_back_to_an_offset() {
        let track = (px(50.0), px(100.0));
        assert_eq!(
            offset_for_pointer(px(50.0), track, px(25.0), px(0.0), px(300.0)),
            px(0.0)
        );
        assert_eq!(
            offset_for_pointer(px(125.0), track, px(25.0), px(0.0), px(300.0)),
            px(300.0)
        );
        assert_eq!(
            offset_for_pointer(px(500.0), track, px(25.0), px(0.0), px(300.0)),
            px(300.0)
        );
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Scrollbar::story, cx);
    }
}
