use gpui::{
    AnyElement, AnyView, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div, prelude::FluentBuilder,
};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;

/// What an empty area says instead of content, such as "No pages yet". It centres itself in
/// the space it is given; `inline` keeps it to one quiet line at the start, for lists.
#[derive(IntoElement)]
pub struct EmptyState {
    message: SharedString,
    title: Option<SharedString>,
    is_inline: bool,
}

impl EmptyState {
    pub fn new(message: impl Into<SharedString>) -> Self {
        EmptyState {
            message: message.into(),
            title: None,
            is_inline: false,
        }
    }

    /// A heading above the message, for a whole window or pane.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn inline(mut self) -> Self {
        self.is_inline = true;
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        if self.is_inline {
            return div()
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .text_size(theme.text.small.size)
                .text_color(theme.colors.text_placeholder)
                .child(self.message);
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(theme.spacing.sm)
            .when_some(self.title, |state, title| {
                state.child(
                    div()
                        .text_size(theme.text.heading_3.size)
                        .line_height(theme.text.heading_3.line_height)
                        .text_color(theme.colors.text)
                        .child(title),
                )
            })
            .child(
                div()
                    .text_size(theme.text.ui.size)
                    .text_color(theme.colors.text_placeholder)
                    .child(self.message),
            )
    }
}

impl EmptyState {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .child(EmptyState::new("No pages yet").inline())
        .child(
            div()
                .h(theme.size.popover_min_width / 2.0)
                .rounded(theme.radius.md)
                .border(theme.size.border_width)
                .border_color(theme.colors.border)
                .child(
                    EmptyState::new("This happens once after Scroll is updated.")
                        .title("Updating workspace…"),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(EmptyState::story, cx);
    }
}
