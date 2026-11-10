//! Adding a server and signing in to it. The address is checked first (is it reachable, is it
//! a Scroll server, does it speak this app's protocol); then the dialog offers exactly the
//! ways of signing in that server lists: Google in the browser, email and password, or both.
//! Nobody is ever shown or asked for a token.

use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder,
};
use scroll_sync::{AccountError, ServerInfo, SignInMethod, SignUpMode};
use scroll_ui::{
    Button, ButtonVariant, Dialog, Divider, FormField, SegmentedControl, TextInput, TextInputEvent,
    Theme,
};

use crate::accounts::{Credentials, PasswordIntent, SessionError};

/// Whether the person is signing in to an account they have, or making one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    SignIn,
    CreateAccount,
}

/// What the checked server lets this version of the app offer.
#[derive(Clone, Debug)]
struct Offer {
    server_url: String,
    info: ServerInfo,
}

impl Offer {
    fn has(&self, method: SignInMethod) -> bool {
        self.info.sign_in_methods.contains(&method)
    }

    fn needs_invite(&self) -> bool {
        self.info.sign_up == SignUpMode::Invite
    }
}

pub struct SignInDialog {
    mode: Mode,
    server: Entity<TextInput>,
    email: Entity<TextInput>,
    password: Entity<TextInput>,
    invite_code: Entity<TextInput>,
    offer: Option<Offer>,
    error: Option<SharedString>,
    is_busy: bool,
    /// A Google sign-in waiting for the browser; dropping it cancels.
    browser_sign_in: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SignInDialog {}

impl Focusable for SignInDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let is_password_offered = self
            .offer
            .as_ref()
            .is_some_and(|offer| offer.has(SignInMethod::Password));
        if !is_password_offered {
            return self.server.focus_handle(cx);
        }
        if self.email.read(cx).text().is_empty() {
            self.email.focus_handle(cx)
        } else {
            self.password.focus_handle(cx)
        }
    }
}

impl SignInDialog {
    /// `server_url` and `email` prefill the form, e.g. after being signed out.
    pub fn new(
        server_url: String,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let server = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("scroll.example.com")
                .with_text(server_url)
        });
        let email = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("you@example.com")
                .with_text(email)
        });
        let password = cx.new(|cx| TextInput::new(cx).placeholder("Password").masked());
        let invite_code = cx.new(|cx| TextInput::new(cx).placeholder("Invite code"));
        let mut subscriptions: Vec<Subscription> = [&email, &password, &invite_code]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |dialog, _, event, _, cx| match event {
                    TextInputEvent::Submitted => dialog.sign_in_with_password(cx),
                    TextInputEvent::Changed => cx.notify(),
                    TextInputEvent::Cancelled => {}
                })
            })
            .collect();
        subscriptions.push(cx.subscribe_in(&server, window, Self::server_changed));
        let has_address = !server.read(cx).text().is_empty();
        let mut dialog = SignInDialog {
            mode: Mode::SignIn,
            server,
            email,
            password,
            invite_code,
            offer: None,
            error: None,
            is_busy: false,
            browser_sign_in: None,
            _subscriptions: subscriptions,
        };
        if has_address {
            dialog.check(window, cx);
        }
        dialog
    }

    fn server_changed(
        &mut self,
        _: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Submitted => self.check(window, cx),
            // A different address needs checking again.
            TextInputEvent::Changed => {
                self.offer = None;
                self.error = None;
                cx.notify();
            }
            TextInputEvent::Cancelled => {}
        }
    }

    /// Asks the server about itself; on success the ways of signing in it offers appear.
    fn check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self.server.read(cx).text().trim().to_string();
        if address.is_empty() {
            self.error = Some("Enter the server's address, for example scroll.example.com.".into());
            return cx.notify();
        }
        self.is_busy = true;
        self.error = None;
        cx.notify();
        let checking = crate::accounts::check_server(&address, cx);
        cx.spawn_in(window, async move |dialog, cx| {
            let outcome = checking.await;
            let shown = dialog.update_in(cx, |dialog, window, cx| {
                dialog.is_busy = false;
                match outcome {
                    Ok((server_url, info)) => {
                        dialog.offer = Some(Offer { server_url, info });
                        let next = dialog.focus_handle(cx);
                        window.focus(&next);
                    }
                    Err(problem) => dialog.error = Some(problem.into()),
                }
                cx.notify();
            });
            if shown.is_err() {
                log::debug!("server check finished after its dialog closed");
            }
        })
        .detach();
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.error = None;
        cx.notify();
    }

    fn invite_code(&self, cx: &App) -> Option<String> {
        Some(self.invite_code.read(cx).text().trim().to_string()).filter(|code| !code.is_empty())
    }

    fn credentials(&self, offer: &Offer, cx: &App) -> Result<Credentials, &'static str> {
        let email = self.email.read(cx).text().trim().to_string();
        let password = self.password.read(cx).text().to_string();
        if !email.contains('@') {
            return Err("Enter your email address.");
        }
        if password.is_empty() {
            return Err("Enter your password.");
        }
        // On an invite-only server the code itself says "new account", so no mode switch is
        // needed and the whole form works from the keyboard.
        let intent = match (offer.needs_invite(), self.invite_code(cx), self.mode) {
            (true, Some(code), _) => PasswordIntent::CreateAccount {
                invite_code: Some(code),
            },
            (true, None, _) | (false, _, Mode::SignIn) => PasswordIntent::SignIn,
            (false, _, Mode::CreateAccount) => PasswordIntent::CreateAccount { invite_code: None },
        };
        Ok(Credentials {
            server_url: offer.server_url.clone(),
            email,
            password,
            intent,
        })
    }

    fn sign_in_with_password(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self
            .offer
            .clone()
            .filter(|offer| offer.has(SignInMethod::Password))
        else {
            return;
        };
        if self.is_busy {
            return;
        }
        let credentials = match self.credentials(&offer, cx) {
            Ok(credentials) => credentials,
            Err(problem) => {
                self.error = Some(problem.into());
                return cx.notify();
            }
        };
        self.is_busy = true;
        self.error = None;
        cx.notify();
        let signing_in = crate::accounts::sign_in(credentials, cx);
        cx.spawn(async move |dialog, cx| {
            let outcome = signing_in.await;
            let shown = dialog.update(cx, |dialog, cx| dialog.finish(outcome, cx));
            if shown.is_err() {
                log::debug!("sign-in finished after its dialog closed");
            }
        })
        .detach();
    }

    /// Opens Google's sign-in in the browser and waits for it to come back.
    fn sign_in_with_google(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self
            .offer
            .clone()
            .filter(|offer| offer.has(SignInMethod::Google))
        else {
            return;
        };
        self.is_busy = true;
        self.error = None;
        let signing_in =
            crate::accounts::sign_in_with_google(&offer.server_url, self.invite_code(cx), cx);
        self.browser_sign_in = Some(cx.spawn(async move |dialog, cx| {
            let outcome = signing_in.await;
            let shown = dialog.update(cx, |dialog, cx| {
                dialog.browser_sign_in = None;
                dialog.finish(outcome, cx);
            });
            if shown.is_err() {
                log::debug!("browser sign-in finished after its dialog closed");
            }
        }));
        cx.notify();
    }

    /// Stops waiting for the browser; dropping the task closes the app's listener.
    fn cancel_browser_sign_in(&mut self, cx: &mut Context<Self>) {
        self.browser_sign_in = None;
        self.is_busy = false;
        cx.notify();
    }

    fn finish(&mut self, outcome: Result<(), SessionError>, cx: &mut Context<Self>) {
        self.is_busy = false;
        match outcome {
            Ok(()) => cx.emit(DismissEvent),
            Err(error) if is_invite_needed(&error) => {
                // Not a dead end: the code goes in the field and the same button tries again.
                self.error = Some("No account yet. Enter your invite code and try again.".into());
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn render_methods(
        &self,
        offer: &Offer,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_google = offer.has(SignInMethod::Google);
        let has_password = offer.has(SignInMethod::Password);
        if !has_google && !has_password {
            return div()
                .text_color(theme.colors.danger)
                .child("This server signs people in a way this version of Scroll does not support. Update Scroll.")
                .into_any_element();
        }
        let shows_invite = offer.needs_invite();
        let shows_mode_switch = has_password && !offer.needs_invite();
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.lg)
            .when(has_google, |form| {
                form.child(
                    Button::new("google", "Continue with Google")
                        .variant(if has_password {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Primary
                        })
                        .when(self.is_busy, Button::disabled)
                        .on_click(cx.listener(|dialog, _, _, cx| dialog.sign_in_with_google(cx))),
                )
            })
            .when(has_google && has_password, |form| {
                form.child(Divider::horizontal())
            })
            .when(shows_mode_switch, |form| {
                form.child(self.render_mode_switch(cx))
            })
            .when(has_password, |form| {
                form.child(FormField::new("Email", self.email.clone()))
                    .child(FormField::new("Password", self.password.clone()))
            })
            .when(shows_invite, |form| {
                form.child(FormField::new(
                    "Invite code, only for a new account",
                    self.invite_code.clone(),
                ))
            })
            .into_any_element()
    }

    fn render_mode_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = match self.mode {
            Mode::SignIn => 0,
            Mode::CreateAccount => 1,
        };
        SegmentedControl::new("mode")
            .option("Sign in")
            .option("Create account")
            .selected(selected)
            .on_select(cx.listener(|dialog, index: &usize, _, cx| {
                let mode = if *index == 0 {
                    Mode::SignIn
                } else {
                    Mode::CreateAccount
                };
                dialog.set_mode(mode, cx)
            }))
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.browser_sign_in.is_some() {
            return Button::new("cancel", "Cancel")
                .on_click(cx.listener(|dialog, _, _, cx| dialog.cancel_browser_sign_in(cx)))
                .into_any_element();
        }
        let Some(offer) = self.offer.as_ref() else {
            return Button::new(
                "check",
                if self.is_busy {
                    "Checking…"
                } else {
                    "Continue"
                },
            )
            .variant(ButtonVariant::Primary)
            .when(self.is_busy, Button::disabled)
            .on_click(cx.listener(|dialog, _, window, cx| dialog.check(window, cx)))
            .into_any_element();
        };
        if !offer.has(SignInMethod::Password) {
            return div().into_any_element();
        }
        let is_creating = if offer.needs_invite() {
            !self.invite_code.read(cx).text().trim().is_empty()
        } else {
            self.mode == Mode::CreateAccount
        };
        let label = match (self.is_busy, is_creating) {
            (true, _) => "Working…",
            (false, false) => "Sign in",
            (false, true) => "Create account",
        };
        Button::new("submit", label)
            .variant(ButtonVariant::Primary)
            .when(self.is_busy, Button::disabled)
            .on_click(cx.listener(|dialog, _, _, cx| dialog.sign_in_with_password(cx)))
            .into_any_element()
    }
}

/// The server wants an invite code before it creates the account.
fn is_invite_needed(error: &SessionError) -> bool {
    matches!(
        error,
        SessionError::Account(
            AccountError::InviteRequired
                | AccountError::BrowserSignInRefused(
                    scroll_sync::BrowserSignInRefusal::InviteRequired
                )
        )
    )
}

impl Render for SignInDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let offer = self.offer.clone();
        let server_line = offer.as_ref().map(|offer| {
            let sign_up = match offer.info.sign_up {
                SignUpMode::Invite => "New accounts need an invite code.",
                SignUpMode::Open => "Anyone can create an account.",
            };
            div()
                .text_size(theme.text.small.size)
                .text_color(theme.colors.text_muted)
                .child(format!(
                    "{} · version {} · {sign_up}",
                    offer.info.name, offer.info.version
                ))
        });
        let methods = offer
            .as_ref()
            .map(|offer| self.render_methods(offer, &theme, cx));
        let waiting = self.browser_sign_in.is_some().then(|| {
            div()
                .text_color(theme.colors.text_muted)
                .child("Waiting for your browser… Finish signing in there, then come back.")
        });
        let actions = self.render_actions(cx);
        let title = if offer.is_some() {
            "Sign in"
        } else {
            "Add a server"
        };
        Dialog::new(title)
            .child(FormField::new("Server address", self.server.clone()))
            .children(server_line)
            .children(methods)
            .children(waiting)
            .when_some(self.error.clone(), Dialog::error)
            .action(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyUpEvent, Keystroke, TestAppContext, VisualTestContext};
    use scroll_sync::BrowserSignInRefusal;
    use scroll_ui::ModalLayer;
    use scroll_ui::tab_navigation;

    /// A window with one control behind the modal layer, as the app window has many.
    struct Host {
        layer: Entity<ModalLayer>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            tab_navigation(div())
                .child(Button::new("behind", "Behind"))
                .child(self.layer.clone())
        }
    }

    fn open_dialog_for_open_server(
        cx: &mut TestAppContext,
    ) -> (Entity<SignInDialog>, &mut VisualTestContext) {
        cx.update(scroll_ui::init);
        let (host, cx) = cx.add_window_view(|_, cx| Host {
            layer: cx.new(|_| ModalLayer::new()),
        });
        let layer = host.read_with(cx, |host, _| host.layer.clone());
        let dialog = cx.update(|window, cx| {
            layer.update(cx, |layer, cx| {
                layer.show(
                    |window, cx| SignInDialog::new(String::new(), String::new(), window, cx),
                    window,
                    cx,
                )
            })
        });
        dialog.update(cx, |dialog, cx| {
            dialog.offer = Some(Offer {
                server_url: "http://scroll.test".into(),
                info: ServerInfo {
                    name: "Test".into(),
                    version: "1".into(),
                    protocol_version: 1,
                    sign_up_requires_invite: false,
                    sign_up: SignUpMode::Open,
                    sign_in_methods: vec![SignInMethod::Password],
                },
            });
            cx.notify();
        });
        let server = dialog.read_with(cx, |dialog, cx| dialog.server.focus_handle(cx));
        cx.update(|window, _| window.focus(&server));
        cx.run_until_parked();
        (dialog, cx)
    }

    /// Presses and releases `key`; GPUI clicks a focused button on key up.
    fn press(key: &str, cx: &mut VisualTestContext) {
        cx.simulate_keystrokes(key);
        let keystroke = Keystroke::parse(key).expect("test keys are valid keystrokes");
        cx.simulate_event(KeyUpEvent { keystroke });
    }

    #[gpui::test]
    fn the_create_account_switch_works_from_the_keyboard(cx: &mut TestAppContext) {
        let (dialog, cx) = open_dialog_for_open_server(cx);

        cx.simulate_keystrokes("tab tab");
        press("space", cx);
        assert_eq!(
            dialog.read_with(cx, |dialog, _| dialog.mode),
            Mode::CreateAccount
        );

        cx.simulate_keystrokes("shift-tab");
        press("enter", cx);
        assert_eq!(dialog.read_with(cx, |dialog, _| dialog.mode), Mode::SignIn);
    }

    #[gpui::test]
    fn tab_stays_inside_the_dialog(cx: &mut TestAppContext) {
        let (dialog, cx) = open_dialog_for_open_server(cx);
        let server = dialog.read_with(cx, |dialog, cx| dialog.server.focus_handle(cx));

        // Server, the two switch buttons, email, password and the submit button.
        cx.simulate_keystrokes("tab tab tab tab tab tab");
        assert!(
            cx.update(|window, _| server.is_focused(window)),
            "Tab after the last control wraps to the first, not to the window behind"
        );
        // Back across the wrap to the submit button, which checks the empty form.
        cx.simulate_keystrokes("shift-tab");
        press("enter", cx);
        assert_eq!(
            dialog.read_with(cx, |dialog, _| dialog.error.clone()),
            Some("Enter your email address.".into())
        );
    }

    #[test]
    fn a_missing_invite_asks_for_the_code_whichever_way_it_was_reported() {
        let by_password = SessionError::Account(AccountError::InviteRequired);
        let by_browser = SessionError::Account(AccountError::BrowserSignInRefused(
            BrowserSignInRefusal::InviteRequired,
        ));
        let cancelled = SessionError::Account(AccountError::BrowserSignInRefused(
            BrowserSignInRefusal::Cancelled,
        ));
        assert!(is_invite_needed(&by_password));
        assert!(is_invite_needed(&by_browser));
        assert!(!is_invite_needed(&cancelled));
    }
}
