use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;
use gpui::{AnyElement, AnyView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Orientation {
    Horizontal,
    Vertical,
}

/// A hairline separator that fills its parent along one axis.
#[derive(IntoElement)]
pub struct Divider {
    orientation: Orientation,
}

impl Divider {
    pub fn horizontal() -> Self {
        Divider {
            orientation: Orientation::Horizontal,
        }
    }

    pub fn vertical() -> Self {
        Divider {
            orientation: Orientation::Vertical,
        }
    }
}

impl RenderOnce for Divider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let line = div().flex_none().bg(theme.colors.border);
        match self.orientation {
            Orientation::Horizontal => line.w_full().h(theme.size.border_width),
            Orientation::Vertical => line.h_full().w(theme.size.border_width),
        }
    }
}

impl Divider {
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
        .child("Above")
        .child(Divider::horizontal())
        .child(
            div()
                .flex()
                .h(theme.size.row_height)
                .gap(theme.spacing.md)
                .child("Left")
                .child(Divider::vertical())
                .child("Right"),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Divider::story, cx);
    }
}
