//! The window's layers above its content, handed to each feature so it can show its own
//! dialogs, prompts and notices without going through the window.

pub mod prompt_dialog;

use gpui::{App, AppContext, Entity, ManagedView, SharedString, Task, Window};
use scroll_ui::{ModalLayer, ToastHost};

use prompt_dialog::{PromptDialog, PromptText};

#[derive(Clone)]
pub struct Overlays {
    modals: Entity<ModalLayer>,
    toasts: Entity<ToastHost>,
}

impl Overlays {
    pub fn new(cx: &mut App) -> Self {
        Overlays {
            modals: cx.new(|_| ModalLayer::new()),
            toasts: cx.new(|_| ToastHost::new()),
        }
    }

    /// The layers, for the window to draw above everything else.
    pub fn layers(&self) -> [gpui::AnyView; 2] {
        [self.toasts.clone().into(), self.modals.clone().into()]
    }

    /// Shows `message` at the bottom of the window for a few seconds.
    pub fn toast(&self, message: impl Into<SharedString>, cx: &mut App) {
        let message = message.into();
        self.toasts
            .update(cx, |toasts, cx| toasts.show(message, cx));
    }

    /// Shows the modal `build` makes, replacing any open one.
    pub fn show_modal<V: ManagedView>(
        &self,
        build: impl FnOnce(&mut Window, &mut gpui::Context<V>) -> V,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<V> {
        self.modals
            .update(cx, |modals, cx| modals.show(build, window, cx))
    }

    pub fn dismiss_modal(&self, window: &mut Window, cx: &mut App) {
        self.modals
            .update(cx, |modals, cx| modals.dismiss(window, cx));
    }

    pub fn showing<V: 'static>(&self, cx: &App) -> Option<Entity<V>> {
        self.modals.read(cx).showing::<V>()
    }

    /// Asks for one piece of text and runs `submit` with it.
    pub fn prompt(
        &self,
        text: PromptText,
        submit: impl Fn(String, &mut App) -> Task<Result<(), String>> + 'static,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.show_modal(
            move |window, cx| PromptDialog::new(text, submit, window, cx),
            window,
            cx,
        );
    }
}
