use gpui::{
    AnyView, App, AppContext, Context, FontWeight, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::FluentBuilder,
};

use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::ellipsis::Ellipsis;
use crate::theme::Theme;

/// What follows the pointer while something is dragged: its name in a small raised label.
/// Return it from GPUI's `on_drag` as `cx.new(|_| DragPreview::new(name))`.
pub struct DragPreview {
    label: SharedString,
    icon: Option<IconName>,
}

impl DragPreview {
    pub fn new(label: impl Into<SharedString>) -> Self {
        DragPreview {
            label: label.into(),
            icon: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

impl Render for DragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .items_center()
            .gap(theme.spacing.sm)
            .max_w(theme.size.drag_preview_max_width)
            .px(theme.spacing.md)
            .py(theme.spacing.xs)
            .rounded(theme.radius.md)
            .border(theme.size.border_width)
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_elevated)
            .shadow(theme.shadow.popover.clone())
            .text_size(theme.text.ui.size)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.colors.text)
            .when_some(self.icon, |preview, icon| {
                preview.child(Icon::new(icon).size(IconSize::Small))
            })
            .child(div().ellipsis().child(self.label.clone()))
    }
}

impl DragPreview {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| DragPreview::new("Three blocks").icon(IconName::Grip))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(DragPreview::story, cx);
    }
}
