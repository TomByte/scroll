use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, RenderOnce, SharedString, Styled,
    Window, div, prelude::FluentBuilder,
};

use crate::controls::button::{Button, ButtonVariant};
use crate::controls::form_field::FormField;
use crate::controls::text_input::TextInput;
use crate::foundation::actions::{Confirm, DIALOG_CONTEXT};
use crate::overlays::modal::Modal;
use crate::theme::Theme;

type ConfirmHandler = Box<dyn Fn(&mut Window, &mut App) + 'static>;

/// A modal dialog: a title, an optional description, its content, a line saying what went
/// wrong if anything did, and its buttons at the trailing edge. Show it through a `ModalLayer`.
#[derive(IntoElement)]
pub struct Dialog {
    title: SharedString,
    description: Option<SharedString>,
    error: Option<SharedString>,
    focus_handle: Option<FocusHandle>,
    on_confirm: Option<ConfirmHandler>,
    children: Vec<AnyElement>,
    actions: Vec<AnyElement>,
}

impl Dialog {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Dialog {
            title: title.into(),
            description: None,
            error: None,
            focus_handle: None,
            on_confirm: None,
            children: Vec::new(),
            actions: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn error(mut self, error: impl Into<SharedString>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// The dialog view's focus handle, so its keys (such as Enter for `on_confirm`) reach it.
    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    /// What Enter does while focus is in the dialog but not in a text field, normally what the
    /// main action does. Needs `track_focus`.
    pub fn on_confirm(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Box::new(handler));
        self
    }

    /// A button (or other control) in the row at the bottom; the last one is the main action.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        Modal::new().child(
            div()
                .when_some(self.focus_handle, |dialog, handle| {
                    dialog.track_focus(&handle)
                })
                .when_some(self.on_confirm, |dialog, on_confirm| {
                    dialog
                        .key_context(DIALOG_CONTEXT)
                        .on_action(move |_: &Confirm, window, cx| on_confirm(window, cx))
                })
                .flex()
                .flex_col()
                .gap(theme.spacing.lg)
                .p(theme.spacing.xl)
                .text_size(theme.text.ui.size)
                .child(
                    div()
                        .text_size(theme.text.heading_3.size)
                        .line_height(theme.text.heading_3.line_height)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.title),
                )
                .when_some(self.description, |dialog, description| {
                    dialog.child(div().text_color(theme.colors.text_muted).child(description))
                })
                .children(self.children)
                .when_some(self.error, |dialog, error| {
                    dialog.child(div().text_color(theme.colors.danger).child(error))
                })
                .when(!self.actions.is_empty(), |dialog| {
                    dialog.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(theme.spacing.sm)
                            .children(self.actions),
                    )
                }),
        )
    }
}

impl Dialog {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let name = cx.new(|cx| TextInput::new(cx).with_text("Team notes"));
        cx.new(|cx| DialogStory {
            name,
            focus_handle: cx.focus_handle(),
            confirmations: 0,
        })
        .into()
    }
}

/// Shown in place; in the app a `ModalLayer` places it over the window.
struct DialogStory {
    name: Entity<TextInput>,
    focus_handle: FocusHandle,
    confirmations: usize,
}

impl Render for DialogStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let story = cx.entity().downgrade();
        Dialog::new("Name the workspace")
            .description("People you invite see this name.")
            .track_focus(&self.focus_handle)
            .on_confirm(move |_, cx| {
                let confirmed = story.update(cx, |story, cx| {
                    story.confirmations += 1;
                    cx.notify();
                });
                if let Err(error) = confirmed {
                    log::debug!("the dialog story closed before Enter reached it: {error}");
                }
            })
            .child(FormField::new("Name", self.name.clone()))
            .error("A workspace with this name is already on this Mac.")
            .action(Button::new("cancel", "Cancel").variant(ButtonVariant::Ghost))
            .action(Button::new("create", "Create").variant(ButtonVariant::Primary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Focusable;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Dialog::story, cx);
    }

    #[gpui::test]
    fn enter_confirms_unless_a_text_field_has_focus(cx: &mut gpui::TestAppContext) {
        let (story, cx) = crate::foundation::story::testing::open_story(Dialog::story, cx);
        let story = story.downcast::<DialogStory>().expect("the dialog story");
        let (dialog, name) = story.read_with(cx, |story, _| {
            (story.focus_handle.clone(), story.name.clone())
        });

        cx.update(|window, _| window.focus(&dialog));
        cx.simulate_keystrokes("enter");
        assert_eq!(story.read_with(cx, |story, _| story.confirmations), 1);

        cx.update(|window, cx| window.focus(&name.focus_handle(cx)));
        cx.simulate_keystrokes("enter");
        assert_eq!(story.read_with(cx, |story, _| story.confirmations), 1);
    }
}
