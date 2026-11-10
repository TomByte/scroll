use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, div, prelude::FluentBuilder,
};

use crate::controls::button::Button;
use crate::motion::{self, Motion, MotionExt, MotionPreference};
use crate::overlays::toast::Toast;
use crate::theme::Theme;

const TOAST_DURATION: Duration = Duration::from_secs(5);

/// Shows one short notice at a time at the bottom of its owner, for a few seconds. Render it
/// as the last child of a full-size, relatively positioned root.
pub struct ToastHost {
    message: Option<SharedString>,
    /// The notice is fading out and goes once the fade is done.
    is_leaving: bool,
    hide: Task<()>,
}

impl ToastHost {
    pub fn new() -> Self {
        ToastHost {
            message: None,
            is_leaving: false,
            hide: Task::ready(()),
        }
    }

    /// Shows `message`, replacing any notice already up.
    pub fn show(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.message = Some(message.into());
        self.is_leaving = false;
        let fade_out =
            (motion::preference(cx) == MotionPreference::Full).then(|| Motion::FadeOut.duration());
        self.hide = cx.spawn(async move |host, cx| {
            let hidden = async {
                cx.background_executor().timer(TOAST_DURATION).await;
                if let Some(fade_out) = fade_out {
                    host.update(cx, |host, cx| {
                        host.is_leaving = true;
                        cx.notify();
                    })?;
                    cx.background_executor().timer(fade_out).await;
                }
                host.update(cx, |host, cx| {
                    host.message = None;
                    cx.notify();
                })
            };
            if let Err(error) = hidden.await {
                log::debug!("window closed before its notice expired: {error}");
            }
        });
        cx.notify();
    }

    #[cfg(test)]
    fn message(&self) -> Option<&SharedString> {
        self.message.as_ref()
    }
}

impl Default for ToastHost {
    fn default() -> Self {
        ToastHost::new()
    }
}

impl Render for ToastHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let (key, toast_motion) = if self.is_leaving {
            ("toast-leaving", Motion::FadeOut)
        } else {
            ("toast", Motion::RiseIn)
        };
        div().when_some(self.message.clone(), |host, message| {
            host.absolute()
                .bottom(theme.spacing.xl)
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .motion(key, toast_motion, window, cx)
                        .child(Toast::new(message)),
                )
        })
    }
}

impl ToastHost {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let toasts = cx.new(|_| ToastHost::new());
        cx.new(|_| ToastHostStory { toasts }).into()
    }
}

/// The notice shows at the bottom of the story here; in the app, at the bottom of the window.
struct ToastHostStory {
    toasts: Entity<ToastHost>,
}

impl Render for ToastHostStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let toasts = self.toasts.clone();
        div()
            .relative()
            .flex()
            .items_start()
            .h(theme.size.popover_min_width / 2.0)
            .child(
                Button::new("show-toast", "Copy link").on_click(move |_, _, cx| {
                    toasts.update(cx, |toasts, cx| toasts.show("Link copied", cx));
                }),
            )
            .child(self.toasts.clone())
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn a_notice_goes_away_after_a_few_seconds(cx: &mut TestAppContext) {
        let host = cx.new(|_| ToastHost::new());
        host.update(cx, |host, cx| host.show("Link copied", cx));
        cx.executor().advance_clock(TOAST_DURATION / 2);
        assert!(host.read_with(cx, |host, _| host.message().is_some()));
        host.update(cx, |host, cx| host.show("Moved to trash", cx));
        cx.executor().advance_clock(TOAST_DURATION * 3 / 4);
        assert_eq!(
            host.read_with(cx, |host, _| host.message().cloned()),
            Some("Moved to trash".into()),
            "a newer notice gets its own full time"
        );
        cx.executor().advance_clock(TOAST_DURATION);
        assert!(
            host.read_with(cx, |host, _| host.is_leaving),
            "it fades out before it goes"
        );
        cx.executor().advance_clock(Motion::FadeOut.duration());
        assert_eq!(host.read_with(cx, |host, _| host.message().cloned()), None);
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut TestAppContext) {
        crate::foundation::story::testing::open_story(ToastHost::story, cx);
    }
}
