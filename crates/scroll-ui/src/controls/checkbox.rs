use gpui::{
    App, AppContext, Context, ElementId, InteractiveElement, IntoElement, ParentElement, Render,
    RenderOnce, SharedString, Styled, Window, div, prelude::FluentBuilder,
};

use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::focus::Clickable;
use crate::theme::Theme;
use gpui::AnyView;

type ToggleHandler = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// A checkbox with an optional label. The caller owns the checked state and receives the
/// requested new state in `on_toggle`.
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    is_checked: bool,
    label: Option<SharedString>,
    is_disabled: bool,
    on_toggle: Option<ToggleHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Checkbox {
            id: id.into(),
            is_checked: false,
            label: None,
            is_disabled: false,
            on_toggle: None,
        }
    }

    pub fn checked(mut self) -> Self {
        self.is_checked = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.is_disabled = true;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let colors = &theme.colors;
        let group_name = SharedString::from(format!("checkbox-{}", self.id));
        let (fill, border) = if self.is_checked {
            (colors.accent, colors.accent)
        } else {
            (colors.control, colors.border_strong)
        };
        let hover_fill = if self.is_checked {
            colors.accent_hover
        } else {
            colors.control_hover
        };
        let is_interactive = !self.is_disabled;

        let indicator = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(theme.size.checkbox)
            .rounded(theme.radius.sm)
            .border(theme.size.border_width)
            .border_color(border)
            .bg(fill)
            .when(is_interactive, |indicator| {
                indicator.group_hover(group_name.clone(), |style| style.bg(hover_fill))
            })
            .when(self.is_checked, |indicator| {
                indicator.child(
                    Icon::new(IconName::Check)
                        .size(IconSize::XSmall)
                        .color(colors.text_on_accent),
                )
            });

        let is_checked = self.is_checked;
        let id = self.id;
        div()
            .id(id.clone())
            .group(group_name)
            .flex()
            .items_center()
            .gap(theme.spacing.md)
            .text_size(theme.text.ui.size)
            .line_height(theme.text.ui.line_height)
            .text_color(colors.text)
            .child(indicator)
            .when_some(self.label, |checkbox, label| checkbox.child(label))
            .map(|checkbox| {
                Clickable::new(theme.radius.sm)
                    .when(self.is_disabled, Clickable::disabled)
                    .when_some(self.on_toggle, |clickable, handler| {
                        clickable.on_click(move |_, window, cx| handler(&!is_checked, window, cx))
                    })
                    .apply(checkbox, window, cx)
            })
    }
}

impl Checkbox {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| CheckboxStory {
            checked: [false, true, true],
        })
        .into()
    }
}

struct CheckboxStory {
    checked: [bool; 3],
}

impl Render for CheckboxStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let labels = ["Show page icons", "Spell check", "Smart quotes"];
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.md)
            .children(labels.into_iter().enumerate().map(|(index, label)| {
                Checkbox::new(("checkbox", index))
                    .when(self.checked[index], Checkbox::checked)
                    .label(label)
                    .on_toggle(cx.listener(move |story, is_checked: &bool, _, cx| {
                        story.checked[index] = *is_checked;
                        cx.notify();
                    }))
            }))
            .child(
                Checkbox::new("checkbox-disabled")
                    .checked()
                    .label("Disabled")
                    .disabled(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn space_asks_for_the_opposite_state(cx: &mut gpui::TestAppContext) {
        use gpui::{KeyUpEvent, Keystroke};
        let (story, cx) = crate::foundation::story::testing::open_story(Checkbox::story, cx);
        let story = story
            .downcast::<CheckboxStory>()
            .expect("the checkbox story");
        cx.simulate_keystrokes("tab space");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").expect("a key"),
        });
        assert_eq!(
            story.read_with(cx, |story, _| story.checked),
            [true, true, true]
        );
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Checkbox::story, cx);
    }
}
