use std::rc::Rc;

use gpui::{
    AnyElement, App, Corner, FocusHandle, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, RenderOnce, Styled, Window, anchored, deferred, div, prelude::FluentBuilder,
};

use crate::foundation::actions::{Cancel, POPOVER_CONTEXT};
use crate::theme::Theme;
use gpui::{AnyView, AppContext, ClickEvent, Context, Render};

use crate::controls::button::Button;

type DismissHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

/// A floating panel anchored at a window position, drawn above everything else and kept
/// inside the window. It calls `on_dismiss` on a click outside it or on Escape (Escape needs
/// focus inside the popover, so pass the focus handle that owns its content).
#[derive(IntoElement)]
pub struct Popover {
    position: Point<Pixels>,
    anchor: Corner,
    focus_handle: Option<FocusHandle>,
    on_dismiss: Option<DismissHandler>,
    children: Vec<AnyElement>,
}

impl Popover {
    pub fn new(position: Point<Pixels>) -> Self {
        Popover {
            position,
            anchor: Corner::TopLeft,
            focus_handle: None,
            on_dismiss: None,
            children: Vec::new(),
        }
    }

    /// Which corner of the popover sits at `position`. Defaults to the top left, so the
    /// popover hangs below; use a bottom corner to open it above the position instead.
    pub fn anchor(mut self, corner: Corner) -> Self {
        self.anchor = corner;
        self
    }

    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let panel = div()
            .key_context(POPOVER_CONTEXT)
            .occlude()
            .flex()
            .flex_col()
            .min_w(theme.size.popover_min_width)
            .p(theme.spacing.xs)
            .rounded(theme.radius.lg)
            .border(theme.size.border_width)
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_elevated)
            .shadow(theme.shadow.popover.clone())
            .text_color(theme.colors.text)
            .when_some(self.focus_handle, |panel, handle| {
                panel.track_focus(&handle)
            })
            .when_some(self.on_dismiss, |panel, on_dismiss| {
                let on_escape = on_dismiss.clone();
                panel
                    .on_action(move |_: &Cancel, window, cx| on_escape(window, cx))
                    .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx))
            })
            .children(self.children);

        deferred(
            anchored()
                .position(self.position)
                .anchor(self.anchor)
                .snap_to_window_with_margin(theme.spacing.md)
                .child(panel),
        )
        .with_priority(1)
    }
}

impl Popover {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| PopoverStory { open_at: None }).into()
    }
}

struct PopoverStory {
    open_at: Option<Point<Pixels>>,
}

impl Render for PopoverStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .child(
                Button::new("open-popover", "Show a popover").on_click(cx.listener(
                    |story, event: &ClickEvent, _, cx| {
                        story.open_at = Some(event.position());
                        cx.notify();
                    },
                )),
            )
            .when_some(self.open_at, |story, position| {
                story.child(
                    Popover::new(position)
                        .on_dismiss({
                            let story = cx.entity().downgrade();
                            move |_, cx| {
                                let closed = story.update(cx, |story, cx| {
                                    story.open_at = None;
                                    cx.notify();
                                });
                                if let Err(error) = closed {
                                    log::debug!("popover story closed before its popover: {error}");
                                }
                            }
                        })
                        .child(div().p(theme.spacing.sm).child("Click outside to close.")),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Popover::story, cx);
    }
}
