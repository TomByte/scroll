use gpui::{App, IntoElement, Pixels, RenderOnce, Styled, Window, div, px};

use crate::display::icon::IconName;
use crate::foundation::story::stateless_story;
use crate::lists::list_item::ListItem;
use crate::theme::Theme;
use gpui::{AnyElement, AnyView};
use gpui::{ParentElement, SharedString};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
}

/// The accent line that shows where a dragged item will land. Place it inside a relatively
/// positioned row; it spans the row from `inset` to the right edge.
#[derive(IntoElement)]
pub struct DropIndicator {
    edge: Edge,
    inset: Pixels,
}

impl DropIndicator {
    pub fn above() -> Self {
        DropIndicator {
            edge: Edge::Top,
            inset: px(0.0),
        }
    }

    pub fn below() -> Self {
        DropIndicator {
            edge: Edge::Bottom,
            inset: px(0.0),
        }
    }

    /// Starts the line further right, e.g. to show the item will be nested.
    pub fn inset(mut self, inset: Pixels) -> Self {
        self.inset = inset;
        self
    }
}

impl RenderOnce for DropIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let thickness = theme.size.drop_indicator_width;
        let line = div()
            .absolute()
            .left(self.inset)
            .right(px(0.0))
            .h(thickness)
            .rounded(thickness)
            .bg(theme.colors.accent);
        match self.edge {
            Edge::Top => line.top(-thickness / 2.0),
            Edge::Bottom => line.bottom(-thickness / 2.0),
        }
    }
}

impl DropIndicator {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    let row =
        |label: &'static str| ListItem::new(SharedString::from(label), label).icon(IconName::Page);
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .relative()
                .child(row("Drop before this row"))
                .child(DropIndicator::above()),
        )
        .child(
            div()
                .relative()
                .child(row("Drop after, nested one level"))
                .child(DropIndicator::below().inset(theme.size.tree_indent)),
        )
        .child(
            div()
                .rounded(theme.radius.md)
                .bg(theme.colors.text_selection)
                .child(row("Drop inside this row")),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(DropIndicator::story, cx);
    }
}
