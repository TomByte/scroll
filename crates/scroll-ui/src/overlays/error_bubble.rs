use gpui::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, Styled, Window, anchored, deferred, div,
};

use crate::controls::text_input::TextInput;
use crate::motion::{Motion, MotionExt};
use crate::theme::Theme;

/// Says why a value cannot be saved, in a small bubble just under the control holding it,
/// drawn above everything so a table cell or list row does not clip it. Put it inside that
/// control, which must be relatively positioned.
#[derive(IntoElement)]
pub struct ErrorBubble {
    message: SharedString,
}

impl ErrorBubble {
    pub fn new(message: impl Into<SharedString>) -> Self {
        ErrorBubble {
            message: message.into(),
        }
    }
}

impl RenderOnce for ErrorBubble {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let bubble = div()
            .mt(theme.spacing.xxs)
            .px(theme.spacing.md)
            .py(theme.spacing.xxs)
            .rounded(theme.radius.md)
            .bg(theme.colors.surface_elevated)
            .border(theme.size.border_width)
            .border_color(theme.colors.danger)
            .shadow(theme.shadow.popover.clone())
            .text_size(theme.text.small.size)
            .line_height(theme.text.small.line_height)
            .text_color(theme.colors.danger)
            .child(self.message)
            .motion("error-bubble", Motion::DropIn, window, cx);
        div()
            .absolute()
            .top_full()
            .left_0()
            .child(deferred(anchored().child(bubble)).with_priority(1))
    }
}

impl ErrorBubble {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let input = cx.new(|cx| TextInput::new(cx).with_text("12 Smarch 2026"));
        cx.new(|_| ErrorBubbleStory { input }).into()
    }
}

struct ErrorBubbleStory {
    input: Entity<TextInput>,
}

impl Render for ErrorBubbleStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div().pb(theme.spacing.xxl).child(
            div()
                .relative()
                .child(self.input.clone())
                .child(ErrorBubble::new("Not a date: try 2026-03-12.")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(ErrorBubble::story, cx);
    }
}
