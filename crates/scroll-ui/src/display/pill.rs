use gpui::{
    AnyElement, AnyView, App, ClickEvent, ElementId, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, Styled, Window, div,
};

use crate::ClickHandler;
use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::ellipsis::Ellipsis;
use crate::foundation::focus::Clickable;
use crate::foundation::story::stateless_story;
use crate::theme::{SWATCH_COUNT, Theme};

/// A small rounded label in a swatch colour, for tags and select options.
#[derive(IntoElement)]
pub struct Pill {
    label: SharedString,
    swatch: usize,
    is_neutral: bool,
    remove: Option<(ElementId, ClickHandler)>,
}

impl Pill {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Pill {
            label: label.into(),
            swatch: 0,
            is_neutral: false,
            remove: None,
        }
    }

    /// Grey instead of a swatch, e.g. a "+3" count of pills not shown.
    pub(crate) fn neutral(mut self) -> Self {
        self.is_neutral = true;
        self
    }

    /// Which swatch (an index, wrapping) the pill takes.
    pub fn swatch(mut self, index: usize) -> Self {
        self.swatch = index;
        self
    }

    /// Adds a remove mark after the label; clicking the pill calls `handler`.
    pub fn on_remove(
        mut self,
        id: impl Into<ElementId>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.remove = Some((id.into(), Box::new(handler)));
        self
    }
}

impl RenderOnce for Pill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let (background, text) = if self.is_neutral {
            (theme.colors.hover, theme.colors.text_muted)
        } else {
            theme.swatch_fill(self.swatch)
        };
        let pill = div()
            .min_w_0()
            .px(theme.spacing.sm)
            .rounded(theme.radius.sm)
            .bg(background)
            .text_color(text)
            .text_size(theme.text.small.size)
            .line_height(theme.text.small.line_height)
            .ellipsis()
            .child(self.label);
        let Some((id, on_remove)) = self.remove else {
            return pill.into_any_element();
        };
        let removable = div()
            .id(id)
            .flex()
            .items_center()
            .gap(theme.spacing.xxs)
            .rounded(theme.radius.sm)
            .child(pill)
            .child(
                Icon::new(IconName::Close)
                    .size(IconSize::XSmall)
                    .color(theme.colors.icon),
            );
        Clickable::new(theme.radius.sm)
            .on_click(on_remove)
            .apply(removable, window, cx)
            .into_any_element()
    }
}

impl Pill {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .flex()
        .flex_wrap()
        .gap(theme.spacing.xs)
        .children((0..SWATCH_COUNT).map(|index| Pill::new(format!("Swatch {index}")).swatch(index)))
        .child(
            Pill::new("Removable")
                .swatch(5)
                .on_remove("remove", |_, _, _| {}),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Pill::story, cx);
    }
}
