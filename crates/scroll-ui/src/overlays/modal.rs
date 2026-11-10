use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce,
    StatefulInteractiveElement, Styled, Window, div,
};

use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::actions::Cancel;
use crate::overlays::tooltip::Tooltip;

use crate::foundation::story::stateless_story;
use crate::theme::Theme;
use gpui::AnyView;

/// The panel a modal's content sits in. Show it through a `ModalLayer`.
#[derive(IntoElement, Default)]
pub struct Modal {
    width: Option<Pixels>,
    children: Vec<AnyElement>,
}

impl Modal {
    pub fn new() -> Self {
        Modal::default()
    }

    /// For a modal with more to show than a dialog, such as the settings page.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
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
            .w(self.width.unwrap_or(theme.size.modal_width))
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

/// Closes the modal the way Escape does, so the `ModalLayer` showing it handles both. For the
/// pointer only: Escape already closes from the keyboard, and as a Tab stop it would come before
/// the modal's first control.
#[derive(IntoElement)]
pub struct ModalCloseButton;

impl RenderOnce for ModalCloseButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let hover = theme.colors.hover;
        div()
            .id("close-modal")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(theme.size.control_small)
            .rounded(theme.radius.md)
            .cursor_pointer()
            .hover(move |button| button.bg(hover))
            .child(
                Icon::new(IconName::Close)
                    .size(IconSize::Small)
                    .color(theme.colors.icon),
            )
            .tooltip(Tooltip::text("Close"))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx))
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
