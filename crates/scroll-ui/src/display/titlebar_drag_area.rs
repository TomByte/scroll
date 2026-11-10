use gpui::{
    AnyElement, AnyView, App, InteractiveElement, IntoElement, MouseButton, ParentElement,
    RenderOnce, StyleRefinement, Styled, Window, div,
};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;

/// Part of a window's title bar drawn by the app: pressing on it and dragging moves the
/// window. Style and fill it like a `div`.
#[derive(IntoElement, Default)]
pub struct TitlebarDragArea {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl TitlebarDragArea {
    pub fn new() -> Self {
        TitlebarDragArea::default()
    }
}

impl Styled for TitlebarDragArea {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for TitlebarDragArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for TitlebarDragArea {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut area = div();
        *area.style() = self.style;
        area.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .children(self.children)
    }
}

impl TitlebarDragArea {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    TitlebarDragArea::new()
        .flex()
        .items_center()
        .h(theme.size.control_large)
        .px(theme.spacing.md)
        .rounded(theme.radius.md)
        .bg(theme.colors.surface_secondary)
        .text_color(theme.colors.text_muted)
        .child("Drag here to move the window")
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(TitlebarDragArea::story, cx);
    }
}
