use gpui::{
    AnyView, App, AppContext, Context, FontWeight, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div,
};

use crate::motion::{Motion, MotionExt};
use crate::theme::Theme;

/// A small label shown on hover. Use with GPUI's `.tooltip(Tooltip::text("…"))`.
pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let text = text.into();
        move |_, cx| {
            let text = text.clone();
            cx.new(|_| Tooltip { text }).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        // GPUI places the tooltip at the cursor; the margin keeps it clear of the pointer.
        div()
            .flex()
            .pl(theme.spacing.md)
            .pt(theme.spacing.lg)
            .child(
                div()
                    .px(theme.spacing.sm)
                    .py(theme.spacing.xxs)
                    .rounded(theme.radius.sm)
                    .bg(theme.colors.tooltip_background)
                    .text_color(theme.colors.tooltip_text)
                    .text_size(theme.text.caption.size)
                    .line_height(theme.text.caption.line_height)
                    .font_weight(FontWeight::MEDIUM)
                    .shadow(theme.shadow.raised.clone())
                    .child(self.text.clone()),
            )
            .motion("tooltip", Motion::FadeIn, window, cx)
    }
}

impl Tooltip {
    pub fn story(window: &mut Window, cx: &mut App) -> AnyView {
        Tooltip::text("Bold  ⌘B")(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Tooltip::story, cx);
    }
}
