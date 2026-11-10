use gpui::{
    AnyElement, AnyView, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div,
};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;

/// A line saying why a value is not accepted, under the field it belongs to. `FormField`
/// shows one under its input; use it alone where a field has no label.
#[derive(IntoElement)]
pub struct FieldError {
    message: SharedString,
}

impl FieldError {
    pub fn new(message: impl Into<SharedString>) -> Self {
        FieldError {
            message: message.into(),
        }
    }
}

impl RenderOnce for FieldError {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .text_size(theme.text.small.size)
            .text_color(theme.colors.danger)
            .child(self.message)
    }
}

impl FieldError {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(_: &mut Window, _: &mut App) -> AnyElement {
    FieldError::new("Type a date such as 9 Nov 2026.").into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(FieldError::story, cx);
    }
}
