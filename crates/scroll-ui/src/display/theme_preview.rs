use gpui::{
    AnyElement, AnyView, App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, Styled,
    Window, div, prelude::FluentBuilder,
};

use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::ellipsis::Ellipsis;
use crate::foundation::story::stateless_story;
use crate::theme::{Theme, ThemeId, Themes};

/// A theme as a row for a theme picker: a sample of its page and text colours, its name, and dots
/// of its accent and code colours. The sample is drawn in that theme whatever the window's.
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

    /// The theme in use: its row is filled and ticked.
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
        let sample = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(theme.size.theme_preview_width)
            .h(theme.size.theme_preview_height)
            .rounded(theme.radius.md)
            .border(theme.size.border_width)
            .border_color(theme.colors.border_strong)
            .bg(colors.surface)
            .text_color(colors.text)
            .text_size(theme.text.small.size)
            .font_weight(FontWeight::SEMIBOLD)
            .child("Aa");
        // Kept when not selected, as an empty space, so every row's dots line up.
        let tick = div()
            .flex()
            .flex_none()
            .justify_center()
            .w(theme.size.icon_medium)
            .when(self.is_selected, |tick| {
                tick.child(
                    Icon::new(IconName::Check)
                        .size(IconSize::Small)
                        .color(theme.colors.accent),
                )
            });
        div()
            .flex()
            .items_center()
            .gap(theme.spacing.md)
            .w_full()
            .px(theme.spacing.sm)
            .py(theme.spacing.xs)
            .rounded(theme.radius.md)
            .when(self.is_selected, |row| row.bg(theme.colors.selected))
            .child(sample)
            .child(div().flex_1().min_w_0().ellipsis().child(self.label))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(theme.spacing.xs)
                    .children(dots),
            )
            .child(tick)
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

/// Every theme, as a picker lists them, with the first selected.
fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    let rows = ThemeId::ALL.iter().enumerate().map(|(index, &id)| {
        let row = ThemePreview::new(id);
        if index == 0 { row.selected() } else { row }
    });
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .w(theme.size.modal_width)
        .children(rows)
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
