use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, Styled, Window, div};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;
use gpui::AnyView;

/// The panel a modal's content sits in. Show it through a `ModalLayer`.
#[derive(IntoElement, Default)]
pub struct Modal {
    children: Vec<AnyElement>,
}

impl Modal {
    pub fn new() -> Self {
        Modal::default()
    }
}

impl ParentElement for Modal {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Modal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .flex_col()
            .w(theme.size.modal_width)
            .overflow_hidden()
            .rounded(theme.radius.xl)
            .border(theme.size.border_width)
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_elevated)
            .shadow(theme.shadow.modal.clone())
            .text_color(theme.colors.text)
            .children(self.children)
    }
}

impl Modal {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    Modal::new()
        .child(
            div()
                .p(theme.spacing.lg)
                .child("The panel every modal's content sits in."),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Modal::story, cx);
    }
}
