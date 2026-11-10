//! A dialog asking for one piece of text, such as a workspace name or an invite code, that
//! runs a request with it and shows why it failed if it did.

use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Subscription, Task, Window,
    prelude::FluentBuilder,
};
use scroll_ui::{Button, ButtonVariant, Dialog, TextInput, TextInputEvent};

type Submit = Box<dyn Fn(String, &mut App) -> Task<Result<(), String>>>;

/// What the dialog says: its title, a sentence under it, the field's hint and the button.
pub struct PromptText {
    pub title: &'static str,
    pub explanation: &'static str,
    pub placeholder: &'static str,
    pub action: &'static str,
}

pub struct PromptDialog {
    text: PromptText,
    input: Entity<TextInput>,
    submit: Submit,
    error: Option<SharedString>,
    is_busy: bool,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for PromptDialog {}

impl Focusable for PromptDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl PromptDialog {
    pub fn new(
        text: PromptText,
        submit: impl Fn(String, &mut App) -> Task<Result<(), String>> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new(cx).placeholder(text.placeholder));
        let subscription = cx.subscribe_in(&input, window, |dialog, _, event, _, cx| {
            if *event == TextInputEvent::Submitted {
                dialog.run(cx);
            }
        });
        PromptDialog {
            text,
            input,
            submit: Box::new(submit),
            error: None,
            is_busy: false,
            _subscription: subscription,
        }
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).text().trim().to_string();
        if self.is_busy || value.is_empty() {
            return;
        }
        self.is_busy = true;
        self.error = None;
        let running = (self.submit)(value, cx);
        cx.spawn(async move |dialog, cx| {
            let outcome = running.await;
            let shown = dialog.update(cx, |dialog, cx| {
                dialog.is_busy = false;
                match outcome {
                    Ok(()) => cx.emit(DismissEvent),
                    Err(error) => dialog.error = Some(error.into()),
                }
                cx.notify();
            });
            if shown.is_err() {
                log::debug!("a request finished after its dialog closed");
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for PromptDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.is_busy {
            "Working…"
        } else {
            self.text.action
        };
        Dialog::new(self.text.title)
            .description(self.text.explanation)
            .child(self.input.clone())
            .when_some(self.error.clone(), Dialog::error)
            .action(
                Button::new("submit", label)
                    .variant(ButtonVariant::Primary)
                    .when(self.is_busy, Button::disabled)
                    .on_click(cx.listener(|dialog, _, _, cx| dialog.run(cx))),
            )
    }
}
