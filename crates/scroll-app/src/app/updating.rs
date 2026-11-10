//! A small window of its own saying something before the main window can open: that the
//! workspace file is being updated after an app update (seconds on a large file), or that the
//! app cannot start.

use gpui::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use scroll_ui::{EmptyState, Theme};

use super::AppState;

const NOTICE_WINDOW_SIZE: (f32, f32) = (420.0, 160.0);
const FAILURE_WINDOW_SIZE: (f32, f32) = (630.0, 160.0);

/// Says the workspace is being updated: in the open window, or in a small one of its own.
pub(super) fn show(cx: &mut App) {
    let window = AppState::window(cx);
    if let Some(window) = window {
        return window.update(cx, |window, cx| {
            window.show_toast("Updating workspace…", cx)
        });
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(NOTICE_WINDOW_SIZE.0), px(NOTICE_WINDOW_SIZE.1)),
            cx,
        ))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |_, cx| {
        cx.new(|_| Notice {
            title: "Updating workspace…",
            message: "This happens once after Scroll is updated.".into(),
        })
    });
    match opened {
        Ok(handle) => AppState::set_updating_window(handle.into(), cx),
        Err(error) => log::error!("could not show that the workspace is updating: {error:#}"),
    }
}

pub(super) fn close(cx: &mut App) {
    if let Some(handle) = AppState::take_updating_window(cx) {
        let closed = handle.update(cx, |_, window, _| window.remove_window());
        if let Err(error) = closed {
            log::debug!("the updating window was already gone: {error}");
        }
    }
}

/// Shows why the app cannot start, in place of the main window.
pub(super) fn show_startup_failure(message: SharedString, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(FAILURE_WINDOW_SIZE.0), px(FAILURE_WINDOW_SIZE.1)),
            cx,
        ))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |_, cx| {
        cx.new(|_| Notice {
            title: "Scroll can't open",
            message,
        })
    });
    if let Err(error) = opened {
        log::error!("could not say why Scroll can't open: {error:#}");
        cx.quit();
    }
}

struct Notice {
    title: &'static str,
    message: SharedString,
}

impl Render for Notice {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .size_full()
            .p(theme.spacing.xl)
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.font.ui.clone())
            .child(EmptyState::new(self.message.clone()).title(self.title))
    }
}
