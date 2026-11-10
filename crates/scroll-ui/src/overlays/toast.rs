use gpui::{
    AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div,
};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;
use gpui::AnyView;

/// A short message that floats at the bottom of the window. The owner decides when it goes
/// away.
#[derive(IntoElement)]
pub struct Toast {
    message: SharedString,
}

impl Toast {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Toast {
            message: message.into(),
        }
    }
}

impl RenderOnce for Toast {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .items_center()
            .gap(theme.spacing.md)
            .max_w(theme.size.modal_width)
            .py(theme.spacing.sm)
            .px(theme.spacing.lg)
            .rounded(theme.radius.lg)
            .bg(theme.colors.tooltip_background)
            .text_color(theme.colors.tooltip_text)
            .text_size(theme.text.ui.size)
            .shadow(theme.shadow.popover.clone())
            .child(div().flex_1().child(self.message))
    }
}

impl Toast {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(theme.spacing.md)
        .child(Toast::new("Link copied"))
        .child(Toast::new("Moved “Roadmap” to trash"))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Toast::story, cx);
    }
}
