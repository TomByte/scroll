use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, CursorStyle, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, RenderOnce, Styled, Window, div,
};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;

type PressHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// A thin strip on the right edge of its (relatively positioned) parent, such as a column
/// header, that shows where to drag to resize it. The owner tracks the drag from `on_press`.
#[derive(IntoElement)]
pub struct ResizeHandle {
    id: ElementId,
    on_press: Option<PressHandler>,
}

impl ResizeHandle {
    pub fn new(id: impl Into<ElementId>) -> Self {
        ResizeHandle {
            id: id.into(),
            on_press: None,
        }
    }

    pub fn on_press(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_press = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ResizeHandle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let on_press = self.on_press;
        div()
            .id(self.id)
            .absolute()
            .top_0()
            .right_0()
            .h_full()
            .w(theme.spacing.xs)
            .cursor(CursorStyle::ResizeColumn)
            .hover(|handle| handle.bg(theme.colors.border_strong))
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                if let Some(on_press) = on_press.as_ref() {
                    on_press(event, window, cx);
                }
            })
    }
}

impl ResizeHandle {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .relative()
        .flex()
        .items_center()
        .w(theme.size.popover_min_width)
        .h(theme.size.row_height)
        .px(theme.spacing.md)
        .border(theme.size.border_width)
        .border_color(theme.colors.border)
        .child("Hover the right edge")
        .child(ResizeHandle::new("resize"))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(ResizeHandle::story, cx);
    }
}
