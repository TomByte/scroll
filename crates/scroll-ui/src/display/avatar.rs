use gpui::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};

use crate::foundation::story::stateless_story;
use crate::overlays::tooltip::Tooltip;
use crate::theme::Theme;
use gpui::{AnyElement, AnyView};

/// A person's initial on a disc of their colour, e.g. for who else is on a page.
#[derive(IntoElement)]
pub struct Avatar {
    id: ElementId,
    name: SharedString,
    collaborator: usize,
}

impl Avatar {
    /// `id` tells avatars apart, e.g. two people with the same name.
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>) -> Self {
        Avatar {
            id: id.into(),
            name: name.into(),
            collaborator: 0,
        }
    }

    /// Which collaborator colour (wrapping) the disc takes, the same as their caret's.
    pub fn collaborator(mut self, index: usize) -> Self {
        self.collaborator = index;
        self
    }

    fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .map(|first| first.to_uppercase().collect())
            .unwrap_or_default()
    }
}

impl RenderOnce for Avatar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let initial = self.initial();
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(theme.size.control_small)
            .rounded_full()
            .border(theme.size.avatar_ring_width)
            .border_color(theme.colors.surface)
            .bg(theme.collaborator_color(self.collaborator))
            .text_color(theme.colors.text_on_accent)
            .text_size(theme.text.caption.size)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .tooltip(Tooltip::text(self.name))
            .child(initial)
    }
}

impl Avatar {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .flex()
        .gap(theme.spacing.xs)
        .children(
            ["Ada", "Bob", "Chen", "Dana"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| Avatar::new(index, name).collaborator(index)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initial_is_the_first_letter_capitalised() {
        assert_eq!(Avatar::new(0, "ada").initial(), "A");
        assert_eq!(Avatar::new(1, "").initial(), "");
        assert_eq!(Avatar::new(2, "élodie").initial(), "É");
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Avatar::story, cx);
    }
}
