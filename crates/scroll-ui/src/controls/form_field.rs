use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    RenderOnce, SharedString, Styled, Window, div, prelude::FluentBuilder,
};

use crate::controls::field_error::FieldError;
use crate::controls::text_input::TextInput;
use crate::theme::Theme;

/// A labelled input, with an optional line saying why its value is not accepted.
#[derive(IntoElement)]
pub struct FormField {
    label: SharedString,
    input: AnyElement,
    error: Option<SharedString>,
}

impl FormField {
    pub fn new(label: impl Into<SharedString>, input: impl IntoElement) -> Self {
        FormField {
            label: label.into(),
            input: input.into_any_element(),
            error: None,
        }
    }

    pub fn error(mut self, error: impl Into<SharedString>) -> Self {
        self.error = Some(error.into());
        self
    }
}

impl RenderOnce for FormField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .child(
                div()
                    .text_size(theme.text.small.size)
                    .text_color(theme.colors.text_muted)
                    .child(self.label),
            )
            .child(self.input)
            .when_some(self.error, |field, error| {
                field.child(FieldError::new(error))
            })
    }
}

impl FormField {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let email = cx.new(|cx| TextInput::new(cx).placeholder("you@example.com"));
        let invite = cx.new(|cx| TextInput::new(cx).with_text("not-a-code"));
        cx.new(|_| FormFieldStory { email, invite }).into()
    }
}

struct FormFieldStory {
    email: Entity<TextInput>,
    invite: Entity<TextInput>,
}

impl Render for FormFieldStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.lg)
            .child(FormField::new("Email", self.email.clone()))
            .child(
                FormField::new("Invite code", self.invite.clone())
                    .error("This invite code is not valid."),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(FormField::story, cx);
    }
}
