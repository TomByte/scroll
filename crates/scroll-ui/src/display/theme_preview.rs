use gpui::{
    AnyElement, AnyView, App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, Styled,
    Window, div, prelude::FluentBuilder,
};

use crate::foundation::story::stateless_story;
use crate::theme::{Theme, ThemeId, Themes};

/// A theme's sample card and name, for a theme picker: its page colour with "Aa" in its text
/// colour, and dots of its accent and code colours. Drawn in that theme whatever the window's.
#[derive(IntoElement)]
pub struct ThemePreview {
    id: ThemeId,
    label: &'static str,
    is_selected: bool,
}

impl ThemePreview {
    pub fn new(id: ThemeId) -> Self {
        ThemePreview {
            id,
            label: id.label(),
            is_selected: false,
        }
    }

    /// A name other than the theme's own, such as "System" over the theme the system gives.
    pub fn label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    /// The theme in use: its card is outlined in the accent.
    pub fn selected(mut self) -> Self {
        self.is_selected = true;
        self
    }
}

impl RenderOnce for ThemePreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let previewed = cx.global::<Themes>().theme(self.id);
        let colors = &previewed.colors;
        let syntax = &previewed.syntax;
        let dots = [
            colors.accent,
            syntax.keyword,
            syntax.string,
            syntax.function,
        ]
        .map(|color| dot(color, &theme));
        let card = div()
            .w(theme.size.theme_preview_width)
            .h(theme.size.theme_preview_height)
            .p(theme.spacing.md)
            .flex()
            .flex_col()
            .justify_between()
            .rounded(theme.radius.lg)
            .border(theme.size.border_width)
            .border_color(theme.colors.border_strong)
            .when(self.is_selected, |card| {
                card.border(theme.size.focus_ring_width)
                    .border_color(theme.colors.accent)
            })
            .bg(colors.surface)
            .child(
                div()
                    .text_color(colors.text)
                    .text_size(theme.text.heading_3.size)
                    .line_height(theme.text.heading_3.line_height)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Aa"),
            )
            .child(div().flex().gap(theme.spacing.xs).children(dots));
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .child(card)
            .child(
                div()
                    .text_color(theme.colors.text_muted)
                    .text_size(theme.text.small.size)
                    .line_height(theme.text.small.line_height)
                    .when(self.is_selected, |label| {
                        label.text_color(theme.colors.text)
                    })
                    .child(self.label),
            )
    }
}

fn dot(color: Hsla, theme: &Theme) -> impl IntoElement {
    div().size(theme.size.icon_xsmall).rounded_full().bg(color)
}

impl ThemePreview {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

/// Every theme, one row per family.
fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    let families = ThemeId::ALL
        .chunk_by(|a, b| a.family() == b.family())
        .map(|family| {
            div()
                .flex()
                .gap(theme.spacing.lg)
                .children(family.iter().map(|&id| ThemePreview::new(id)))
        });
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.lg)
        .children(families)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(ThemePreview::story, cx);
    }
}
