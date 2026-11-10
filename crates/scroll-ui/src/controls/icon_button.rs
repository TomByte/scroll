use gpui::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};

use crate::ClickHandler;
use crate::controls::button::{ButtonSize, ButtonVariant};
use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::focus::Clickable;
use crate::foundation::story::stateless_story;
use crate::overlays::tooltip::Tooltip;
use crate::theme::Theme;
use gpui::{AnyElement, AnyView};

/// A square, icon-only button. Ghost by default; give it a tooltip so it has a name.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    variant: ButtonVariant,
    size: ButtonSize,
    tooltip: Option<SharedString>,
    is_selected: bool,
    is_disabled: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        IconButton {
            id: id.into(),
            icon,
            variant: ButtonVariant::Ghost,
            size: ButtonSize::default(),
            tooltip: None,
            is_selected: false,
            is_disabled: false,
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Shows the button as toggled on, e.g. an active formatting mark.
    pub fn selected(mut self) -> Self {
        self.is_selected = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.is_disabled = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let colors = self.variant.colors(&theme);
        let icon_size = match self.size {
            ButtonSize::Small => IconSize::Small,
            ButtonSize::Medium | ButtonSize::Large => IconSize::Medium,
        };
        let icon_color = match (self.variant, self.is_selected) {
            (ButtonVariant::Primary, _) => colors.text,
            (_, true) => theme.colors.text,
            (_, false) => theme.colors.icon,
        };
        let background = if self.is_selected {
            theme.colors.selected
        } else {
            colors.background
        };

        let id = self.id;
        div()
            .id(id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(self.size.height(&theme))
            .rounded(theme.radius.md)
            .bg(background)
            .when(self.variant == ButtonVariant::Secondary, |button| {
                button
                    .border(theme.size.border_width)
                    .border_color(theme.colors.border_strong)
                    .shadow(theme.shadow.raised.clone())
            })
            .child(Icon::new(self.icon).size(icon_size).color(icon_color))
            .when_some(self.tooltip, |button, text| {
                button.tooltip(Tooltip::text(text))
            })
            .map(|button| {
                Clickable::new(theme.radius.md)
                    .hover(colors.hover)
                    .pressed(colors.pressed)
                    .when(self.is_disabled, Clickable::disabled)
                    .when_some(self.on_click, Clickable::on_click)
                    .apply(button, window, cx)
            })
    }
}

impl IconButton {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .flex()
        .items_center()
        .gap(theme.spacing.md)
        .child(IconButton::new("icon-ghost", IconName::Sidebar).tooltip("Toggle sidebar"))
        .child(
            IconButton::new("icon-selected", IconName::Text)
                .selected()
                .tooltip("Selected"),
        )
        .child(
            IconButton::new("icon-secondary", IconName::Ellipsis)
                .variant(ButtonVariant::Secondary)
                .tooltip("More"),
        )
        .child(
            IconButton::new("icon-primary", IconName::Plus)
                .variant(ButtonVariant::Primary)
                .tooltip("New page"),
        )
        .child(
            IconButton::new("icon-small", IconName::Close)
                .size(ButtonSize::Small)
                .tooltip("Close"),
        )
        .child(IconButton::new("icon-disabled", IconName::Trash).disabled())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(IconButton::story, cx);
    }
}
