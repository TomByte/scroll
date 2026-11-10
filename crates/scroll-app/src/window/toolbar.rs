//! The toolbar: history buttons, breadcrumbs, who else is here and how syncing stands.

use crate::accounts::commands::AccountRequest;
use crate::accounts::flows;
use crate::commands::hint;
use crate::sync_status::status_text;
use crate::title_bar::{self, TRAFFIC_LIGHTS_WIDTH};
use crate::window::commands::{GO_BACK, GO_FORWARD};
use crate::workspace::page_tree::labelled_title;
use crate::workspace::sync_state::SyncState;
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, Pixels, Styled, div,
    prelude::FluentBuilder,
};
use scroll_ui::{
    Avatar, Button, ButtonSize, ButtonVariant, Ellipsis, IconButton, IconName, Theme,
    TitlebarDragArea,
};
use std::collections::HashSet;

use super::*;

const PUBLISHING: &str = "Publishing — preparing pages and records…";

impl AppWindow {
    /// `sidebar_width` is how much of the sidebar shows beside the toolbar, which changes as
    /// the sidebar slides open or closed.
    pub(super) fn render_toolbar(
        &self,
        theme: &Theme,
        sidebar_width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tree = self.workspace.read(cx).tree();
        let crumbs: Vec<String> = self
            .current_page()
            .map(|page| tree.ancestors_and_self(page))
            .unwrap_or_default()
            .into_iter()
            .rev()
            .filter_map(|page| tree.page(page).map(labelled_title))
            .collect();
        let crumb_count = crumbs.len();
        let breadcrumbs = crumbs.into_iter().enumerate().flat_map(|(index, title)| {
            let is_last = index + 1 == crumb_count;
            let separator = (index > 0).then(|| {
                div()
                    .text_color(theme.colors.text_placeholder)
                    .child("/")
                    .into_any_element()
            });
            let crumb = div()
                .ellipsis()
                .text_color(if is_last {
                    theme.colors.text
                } else {
                    theme.colors.text_muted
                })
                .when(is_last, |crumb| crumb.font_weight(FontWeight::MEDIUM))
                .child(title)
                .into_any_element();
            separator.into_iter().chain(std::iter::once(crumb))
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.spacing.xs)
            .h(title_bar::height(theme))
            .px(theme.spacing.md)
            // Where the sidebar does not cover them, the traffic lights sit over the toolbar.
            .pl((TRAFFIC_LIGHTS_WIDTH - sidebar_width).max(theme.spacing.md))
            .text_size(theme.text.ui.size)
            .child(
                IconButton::new("back", IconName::ArrowLeft)
                    .size(ButtonSize::Small)
                    .tooltip(format!("Back  {}", hint(command_list::all(), GO_BACK)))
                    .when(!self.history.can_go_back(), IconButton::disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))),
            )
            .child(
                IconButton::new("forward", IconName::ArrowRight)
                    .size(ButtonSize::Small)
                    .tooltip(format!(
                        "Forward  {}",
                        hint(command_list::all(), GO_FORWARD)
                    ))
                    .when(!self.history.can_go_forward(), IconButton::disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.go_forward(window, cx))),
            )
            .child(
                TitlebarDragArea::new()
                    .flex()
                    .items_center()
                    .gap(theme.spacing.sm)
                    .pl(theme.spacing.sm)
                    .min_w_0()
                    .flex_1()
                    .h_full()
                    .children(breadcrumbs),
            )
            .children(self.render_people(theme, cx))
            .children(self.render_connection(theme, cx))
            .into_any_element()
    }

    /// Everyone else on the open page, as overlapping avatars.
    pub(super) fn render_people(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let page = self.page.as_ref()?.read(cx);
        let mut seen = HashSet::new();
        let avatars: Vec<Avatar> = page
            .people()
            .filter(|person| seen.insert(person.user))
            .map(|person| {
                Avatar::new(gpui::ElementId::Uuid(person.user.0), person.name.clone())
                    .collaborator(person.color_index)
            })
            .collect();
        if avatars.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .items_center()
                .pr(theme.spacing.sm)
                .children(
                    avatars
                        .into_iter()
                        .map(|avatar| div().ml(-avatar_overlap(theme)).child(avatar)),
                )
                .into_any_element(),
        )
    }

    /// Says nothing while all is well; otherwise how syncing stands and what to do about it.
    pub(super) fn render_connection(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if cx
            .try_global::<Accounts>()
            .is_some_and(Accounts::is_publishing)
        {
            return Some(self.status_line(PUBLISHING.into(), theme));
        }
        let state = self.workspace.read(cx).sync_state();
        if state == SyncState::SignedOut {
            return Some(self.render_signed_out(theme, cx));
        }
        status_text(&state).map(|text| self.status_line(text, theme))
    }

    pub(super) fn status_line(&self, text: String, theme: &Theme) -> AnyElement {
        div()
            .flex_none()
            .px(theme.spacing.sm)
            .text_color(theme.colors.text_muted)
            .child(text)
            .into_any_element()
    }

    pub(super) fn render_signed_out(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.spacing.sm)
            .text_color(theme.colors.danger)
            .child("Signed out")
            .child(
                Button::new("sign-in-again", "Sign in")
                    .size(ButtonSize::Small)
                    .variant(ButtonVariant::Primary)
                    .on_click(cx.listener(|this, _, window, cx| {
                        flows::run(AccountRequest::SignIn(None), &this.overlays, window, cx)
                    })),
            )
            .into_any_element()
    }
}

/// How far each avatar tucks under the one before it, so several read as a stack.
fn avatar_overlap(theme: &Theme) -> gpui::Pixels {
    theme.spacing.xs
}
