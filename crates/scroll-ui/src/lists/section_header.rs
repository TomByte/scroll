use gpui::{
    AnyElement, AnyView, App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div,
};

use crate::foundation::story::stateless_story;
use crate::lists::list_item::ListItem;
use crate::theme::Theme;

/// A small heading over a group of rows, as in a sidebar, menu or popover.
#[derive(IntoElement)]
pub struct SectionHeader {
    label: SharedString,
}

impl SectionHeader {
    pub fn new(label: impl Into<SharedString>) -> Self {
        SectionHeader {
            label: label.into(),
        }
    }
}

impl RenderOnce for SectionHeader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .px(theme.spacing.sm)
            .pt(theme.spacing.md)
            .pb(theme.spacing.xs)
            .text_size(theme.text.caption.size)
            .line_height(theme.text.caption.line_height)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.colors.text_placeholder)
            .child(self.label)
    }
}

impl SectionHeader {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(_: &mut Window, _: &mut App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(SectionHeader::new("Pages"))
        .child(ListItem::new("section-row", "Getting started"))
        .child(SectionHeader::new("Tags"))
        .child(ListItem::new("section-tag", "#planning"))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(SectionHeader::story, cx);
    }
}
