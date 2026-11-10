//! Switching pages in one step. The page being opened loads off screen while the one before
//! stays up; once it is ready (`PageViewEvent::Ready`) it replaces it and the page fade plays.
//! A page that takes longer than `PAGE_WAIT` goes on screen anyway, still loading, so a click
//! never seems ignored. Opening another page meanwhile drops the one loading.
//!
//! Which page is current changes at once: the history, sidebar, breadcrumbs, window title, the
//! saved last page and commands that name a page follow it from the click. What shows or acts
//! on the page's view follows the page on screen (the toolbar's people and width toggle), and
//! a command for the view first puts the page being opened on screen. While both pages exist
//! both count as open in the workspace and on the server: the new one from the click, so its
//! content and presence start arriving, the old one until it leaves the screen.

use std::time::Duration;

use gpui::Task;

use super::*;

/// The longest the page before stays up while the next one loads. Measured in a release
/// build: an ordinary page is ready 10–30 ms after the click, one with diagrams once they are
/// drawn (30–65 ms; about 135 ms for a 30-step flowchart).
pub(super) const PAGE_WAIT: Duration = Duration::from_millis(150);

/// A page loading off screen, to replace the one on screen.
pub(super) struct IncomingPage {
    view: Entity<PageView>,
    events: Subscription,
    /// Puts the page on screen after `PAGE_WAIT`, ready or not.
    _deadline: Task<()>,
}

impl AppWindow {
    pub(super) fn show_page(&mut self, page: PageId, window: &mut Window, cx: &mut Context<Self>) {
        let is_on_screen = self.page_on_screen(cx) == Some(page);
        if is_on_screen && self.incoming.is_none() {
            return;
        }
        self.follow_current_page(page, window, cx);
        if is_on_screen {
            // Back to the page on screen before the next one was ready.
            self.incoming = None;
            return;
        }
        let is_loading = self
            .incoming
            .as_ref()
            .is_some_and(|incoming| incoming.view.read(cx).page() == page);
        if !is_loading {
            self.incoming = Some(self.load_off_screen(page, window, cx));
        }
    }

    /// Puts the page being opened on screen in place of the one before, which is let go.
    pub(super) fn finish_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(incoming) = self.incoming.take() else {
            return;
        };
        let view = incoming.view;
        self.page_subscriptions = vec![
            incoming.events,
            // Who else is on the page shows in the toolbar.
            cx.observe(&view, |_, _, cx| cx.notify()),
        ];
        view.update(cx, |view, cx| view.arrive_on_screen(window, cx));
        self.page = Some(view);
        cx.notify();
    }

    /// The view of the current page: the one being opened, else the one on screen.
    pub(super) fn current_view(&self) -> Option<&Entity<PageView>> {
        self.incoming
            .as_ref()
            .map(|incoming| &incoming.view)
            .or(self.page.as_ref())
    }

    pub(super) fn is_switching(&self) -> bool {
        self.incoming.is_some()
    }

    pub(super) fn page_on_screen(&self, cx: &gpui::App) -> Option<PageId> {
        self.page.as_ref().map(|view| view.read(cx).page())
    }

    /// The parts of the window that name the current page follow it at once.
    fn follow_current_page(&mut self, page: PageId, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_current_page(Some(page), cx));
        self.workspace_settings.last_page = Some(page);
        self.update_window_title(window, cx);
        self.save_workspace_settings(cx);
        cx.notify();
    }

    fn load_off_screen(
        &mut self,
        page: PageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> IncomingPage {
        let workspace = self.workspace.clone();
        let my_name = cx.try_global::<Accounts>().and_then(|accounts| {
            let link = accounts.registry().current().server.as_ref()?;
            Some(
                accounts
                    .registry()
                    .account(&link.server_url)?
                    .display_name
                    .clone(),
            )
        });
        let view = cx.new(|cx| PageView::new(page, workspace, my_name, window, cx));
        let events = cx.subscribe_in(&view, window, Self::page_event);
        let deadline = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(PAGE_WAIT).await;
            let shown = this.update_in(cx, |this, window, cx| this.finish_switch(window, cx));
            if let Err(error) = shown {
                log::debug!("the window closed while a page was loading: {error}");
            }
        });
        IncomingPage {
            view,
            events,
            _deadline: deadline,
        }
    }

    fn page_event(
        &mut self,
        view: &Entity<PageView>,
        event: &PageViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PageViewEvent::OpenPage(page) => self.open_page(*page, window, cx),
            PageViewEvent::Missing => {
                let missing = view.read(cx).page();
                self.leave_missing_page(missing, window, cx);
            }
            PageViewEvent::Ready => {
                let is_incoming = self
                    .incoming
                    .as_ref()
                    .is_some_and(|incoming| incoming.view == *view);
                if is_incoming {
                    self.finish_switch(window, cx);
                }
            }
        }
    }
}
