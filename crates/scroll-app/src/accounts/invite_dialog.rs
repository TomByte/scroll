//! Making an invite code for the open server workspace, to send to someone.

use gpui::{
    App, ClipboardItem, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, prelude::FluentBuilder,
};
use scroll_core::id::Role;
use scroll_ui::{Button, ButtonVariant, Dialog, SegmentedControl, Theme};

pub struct InviteDialog {
    role: Role,
    code: Option<SharedString>,
    is_copied: bool,
    error: Option<SharedString>,
    is_busy: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<DismissEvent> for InviteDialog {}

impl Focusable for InviteDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl InviteDialog {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        InviteDialog {
            role: Role::Editor,
            code: None,
            is_copied: false,
            error: None,
            is_busy: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Starts with `role` chosen, e.g. from "Invite someone to view only".
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    fn choose_role(&mut self, role: Role, cx: &mut Context<Self>) {
        self.role = role;
        self.code = None;
        cx.notify();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        self.error = None;
        let creating = crate::accounts::create_invite(self.role, cx);
        cx.spawn(async move |dialog, cx| {
            let outcome = creating.await;
            let shown = dialog.update(cx, |dialog, cx| {
                dialog.is_busy = false;
                match outcome {
                    Ok(code) => {
                        dialog.code = Some(code.into());
                        // Sending the code is the next step, so it goes to the clipboard at once.
                        dialog.copy(cx);
                    }
                    Err(error) => dialog.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
            if shown.is_err() {
                log::debug!("an invite arrived after its dialog closed");
            }
        })
        .detach();
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(code) = self.code.as_ref() {
            cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
            self.is_copied = true;
            cx.notify();
        }
    }
}

impl Render for InviteDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let explanation = match self.role {
            Role::Viewer => "They can read every page but not change anything.",
            _ => "They can read and edit every page.",
        };
        let roles = [Role::Editor, Role::Viewer];
        let role_switch = SegmentedControl::new("role")
            .option("Can edit")
            .option("Can view")
            .selected(
                roles
                    .iter()
                    .position(|role| *role == self.role)
                    .unwrap_or(0),
            )
            .on_select(cx.listener(move |dialog, index: &usize, _, cx| {
                dialog.choose_role(roles[*index], cx)
            }));
        let code = self.code.clone().map(|code| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme.spacing.md)
                .p(theme.spacing.md)
                .rounded(theme.radius.md)
                .bg(theme.colors.code_background)
                .child(div().font_family(theme.font.mono.clone()).child(code))
                .child(
                    Button::new("copy", if self.is_copied { "Copied" } else { "Copy" })
                        .on_click(cx.listener(|dialog, _, _, cx| dialog.copy(cx))),
                )
        });
        let create = Button::new(
            "create",
            if self.code.is_some() {
                "Make another"
            } else {
                "Create invite"
            },
        )
        .variant(ButtonVariant::Primary)
        .when(self.is_busy, Button::disabled)
        .on_click(cx.listener(|dialog, _, _, cx| dialog.create(cx)));
        let dialog = cx.entity().downgrade();
        Dialog::new("Invite someone")
            .track_focus(&self.focus_handle)
            // Enter makes the invite, as the main button does.
            .on_confirm(move |_, cx| {
                if let Err(error) = dialog.update(cx, |dialog, cx| dialog.create(cx)) {
                    log::debug!("the invite dialog closed before Enter reached it: {error}");
                }
            })
            .child(role_switch)
            .child(div().text_color(theme.colors.text_muted).child(explanation))
            .children(code)
            .when_some(self.error.clone(), Dialog::error)
            .action(create)
    }
}
