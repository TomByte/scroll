//! The trash: a dialog listing the pages in it, the one deleted last first, with when each was
//! deleted and what it was under, to search and bring back. Opened from the foot of the
//! sidebar. Only the rows on screen are drawn, so a full trash opens as fast as an empty one.

use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, ScrollStrategy, SharedString, Styled, Subscription, UniformListScrollHandle, Window,
    div, prelude::FluentBuilder, px, uniform_list,
};
use scroll_core::id::PageId;
use scroll_core::store::TrashedPage;
use scroll_ui::actions::{MoveDown, MoveUp};
use scroll_ui::{
    Button, ButtonSize, Divider, Ellipsis, EmptyState, Icon, IconName, IconSize, Modal,
    ModalCloseButton, Scrollbar, TextInput, TextInputEvent, Theme,
};

use crate::workspace::Workspace;
use crate::workspace::page_tree::display_title;

/// The height of a row: a title above a line saying when the page was deleted.
const ROW_HEIGHT: Pixels = px(52.0);
/// How many rows show before the list scrolls.
const ROWS_SHOWN: usize = 8;

pub struct TrashDialog {
    workspace: Entity<Workspace>,
    search: Entity<TextInput>,
    /// Everything in the trash, read once as the dialog opens; `None` until it has been.
    trash: Option<Vec<TrashedPage>>,
    /// The places in `trash` of the pages the search finds, in the trash's order.
    found: Vec<usize>,
    /// Which of `found` Return restores.
    highlighted: usize,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for TrashDialog {}

impl Focusable for TrashDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
}

impl TrashDialog {
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx).placeholder("Search the trash"));
        let subscription =
            cx.subscribe_in(&search, window, |dialog, _, event, _, cx| match event {
                TextInputEvent::Changed => dialog.search_changed(cx),
                TextInputEvent::Submitted => dialog.restore_highlighted(cx),
                TextInputEvent::Cancelled => {}
            });
        let dialog = TrashDialog {
            workspace,
            search,
            trash: None,
            found: Vec::new(),
            highlighted: 0,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        };
        dialog.read_trash(cx);
        dialog
    }

    fn read_trash(&self, cx: &mut Context<Self>) {
        let storage = self.workspace.read(cx).storage();
        let reading = storage.read(cx, |store| Ok(store.trashed_pages()?));
        cx.spawn(async move |dialog, cx| match reading.await {
            Ok(trash) => {
                let shown = dialog.update(cx, |dialog, cx| {
                    dialog.trash = Some(trash);
                    dialog.find(cx);
                });
                if shown.is_err() {
                    log::debug!("the trash was closed before it was read");
                }
            }
            Err(error) => log::error!("could not read the trash: {error}"),
        })
        .detach();
    }

    /// Lists the pages whose titles hold what is typed in the search, whatever its case.
    fn find(&mut self, cx: &mut Context<Self>) {
        let wanted = self.search.read(cx).text().trim().to_lowercase();
        let trash = self.trash.iter().flatten().enumerate();
        self.found = trash
            .filter(|(_, trashed)| {
                let title = display_title(&trashed.page.title);
                title.to_lowercase().contains(&wanted)
            })
            .map(|(place, _)| place)
            .collect();
        self.highlighted = self.highlighted.min(self.found.len().saturating_sub(1));
        cx.notify();
    }

    fn search_changed(&mut self, cx: &mut Context<Self>) {
        self.highlighted = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.find(cx);
    }

    fn move_highlight(&mut self, step: isize, cx: &mut Context<Self>) {
        let last = self.found.len().saturating_sub(1) as isize;
        self.highlighted = (self.highlighted as isize + step).clamp(0, last) as usize;
        self.scroll
            .scroll_to_item(self.highlighted, ScrollStrategy::Top);
        cx.notify();
    }

    fn can_restore(&self, cx: &App) -> bool {
        !self.workspace.read(cx).is_read_only()
    }

    fn restore_highlighted(&mut self, cx: &mut Context<Self>) {
        let highlighted = self.found.get(self.highlighted);
        let page = highlighted.and_then(|place| Some(self.trash.as_ref()?.get(*place)?.page.id));
        if let Some(page) = page {
            self.restore(page, cx);
        }
    }

    /// Brings `page` back to where it was. The dialog stays open, for the next one.
    fn restore(&mut self, page: PageId, cx: &mut Context<Self>) {
        if !self.can_restore(cx) {
            return;
        }
        let Some(trash) = self.trash.as_mut() else {
            return;
        };
        trash.retain(|trashed| trashed.page.id != page);
        self.workspace
            .update(cx, |workspace, cx| workspace.restore(page, cx));
        self.find(cx);
    }

    /// The pages found, as their titles, for tests.
    #[cfg(test)]
    fn found_titles(&self) -> Vec<String> {
        let trash = self.trash.as_deref().unwrap_or_default();
        let found = self.found.iter().map(|place| &trash[*place]);
        found.map(|trashed| trashed.page.title.clone()).collect()
    }

    fn render_rows(
        &self,
        rows: Range<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let now = seconds_since_1970(SystemTime::now());
        let trash = self.trash.as_deref().unwrap_or_default();
        let found = rows.filter_map(|row| Some((row, trash.get(*self.found.get(row)?)?)));
        found
            .map(|(row, trashed)| self.render_row(row, trashed, now, theme, cx))
            .collect()
    }

    fn render_row(
        &self,
        row: usize,
        trashed: &TrashedPage,
        now: i64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = trashed.page.id;
        let hover = theme.colors.hover;
        let is_highlighted = row == self.highlighted;
        let group = SharedString::from(format!("trashed-{row}"));
        let icon = match (&trashed.page.icon, trashed.page.is_folder) {
            (Some(emoji), _) => div().child(emoji.clone()).into_any_element(),
            (None, true) => page_icon(IconName::Folder, theme),
            (None, false) => page_icon(IconName::Page, theme),
        };
        let restore = Button::new(
            ElementId::NamedInteger("restore".into(), row as u64),
            "Restore",
        )
        .size(ButtonSize::Small)
        .icon(IconName::Restore)
        .on_click(cx.listener(move |dialog, _, _, cx| dialog.restore(page, cx)));
        div()
            .id(ElementId::NamedInteger("trashed".into(), row as u64))
            .debug_selector(|| format!("trashed-{row}"))
            .group(group.clone())
            .w_full()
            .h(ROW_HEIGHT)
            .flex()
            .items_center()
            .gap(theme.spacing.md)
            .pl(theme.spacing.md)
            // Room for the scrollbar, which is drawn over the list's trailing edge.
            .pr(theme.spacing.xl)
            .rounded(theme.radius.md)
            .when(is_highlighted, |row| row.bg(hover))
            .hover(move |row| row.bg(hover))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .w(theme.size.icon_medium)
                    .child(icon),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .ellipsis()
                            .child(display_title(&trashed.page.title).to_string()),
                    )
                    .child(
                        div()
                            .ellipsis()
                            .text_size(theme.text.caption.size)
                            .line_height(theme.text.caption.line_height)
                            .text_color(theme.colors.text_muted)
                            .child(deletion_note(trashed, now)),
                    ),
            )
            // One button, on the row the keys or the pointer are at, not a column of them.
            .when(self.can_restore(cx), |row| {
                row.child(
                    div()
                        .flex_none()
                        .when(!is_highlighted, |button| {
                            button
                                .invisible()
                                .group_hover(group, |button| button.visible())
                        })
                        .child(restore),
                )
            })
            .into_any_element()
    }

    /// What the foot of the dialog says: how much is in the trash, and how to use the keys.
    fn summary(&self, cx: &App) -> String {
        let Some(trash) = self.trash.as_ref() else {
            return String::new();
        };
        let count = match (self.found.len(), trash.len()) {
            (_, 1) => "1 page".to_string(),
            (found, all) if found == all => format!("{all} pages"),
            (found, all) => format!("{found} of {all} pages"),
        };
        if self.found.is_empty() || !self.can_restore(cx) {
            return count;
        }
        format!("{count}  ·  ↑↓ choose  ·  ↵ restore  ·  esc close")
    }
}

fn page_icon(icon: IconName, theme: &Theme) -> AnyElement {
    Icon::new(icon)
        .size(IconSize::Medium)
        .color(theme.colors.icon)
        .into_any_element()
}

fn seconds_since_1970(time: SystemTime) -> i64 {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    since.as_secs() as i64
}

/// The line under a page's title: when it was deleted, and the page it was under.
fn deletion_note(trashed: &TrashedPage, now: i64) -> String {
    let deleted = match trashed.archived_at {
        Some(archived_at) => format!("Deleted {}", time_ago(now - archived_at)),
        // This device never saw it go, so all it knows is that it went before now.
        None => "Deleted earlier".to_string(),
    };
    match trashed.parent_title.as_deref() {
        Some(parent) => format!("{deleted}  ·  in {}", display_title(parent)),
        None => deleted,
    }
}

/// How long ago `seconds` back was, in the largest unit that fits: "3 days ago".
fn time_ago(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let (count, unit) = match seconds {
        // A clock set back since then reads as the future; that is no reason to say so.
        ..MINUTE => return "just now".to_string(),
        MINUTE..HOUR => (seconds / MINUTE, "minute"),
        HOUR..DAY => (seconds / HOUR, "hour"),
        _ if seconds < 2 * DAY => return "yesterday".to_string(),
        _ if seconds < 30 * DAY => (seconds / DAY, "day"),
        _ if seconds < 365 * DAY => (seconds / (30 * DAY), "month"),
        _ => (seconds / (365 * DAY), "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

impl Render for TrashDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let row_theme = theme.clone();
        let row_count = self.found.len();
        let is_trash_empty = self.trash.as_ref().is_some_and(Vec::is_empty);
        let nothing_to_show = match (is_trash_empty, row_count) {
            (true, _) => Some("The trash is empty"),
            (false, 0) if self.trash.is_some() => Some("Nothing in the trash matches"),
            _ => None,
        };
        let rows = uniform_list(
            "trashed-pages",
            row_count,
            cx.processor(move |dialog, rows: Range<usize>, _, cx| {
                dialog.render_rows(rows, &row_theme, cx)
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full();
        div()
            .on_action(cx.listener(|dialog, _: &MoveUp, _, cx| dialog.move_highlight(-1, cx)))
            .on_action(cx.listener(|dialog, _: &MoveDown, _, cx| dialog.move_highlight(1, cx)))
            .child(
                Modal::new()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(theme.spacing.xl)
                            .pt(theme.spacing.lg)
                            .child(
                                div()
                                    .text_size(theme.text.heading_3.size)
                                    .line_height(theme.text.heading_3.line_height)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Trash"),
                            )
                            .child(ModalCloseButton),
                    )
                    .child(div().p(theme.spacing.md).child(self.search.clone()))
                    .child(Divider::horizontal())
                    .child(
                        div()
                            .relative()
                            .h(ROW_HEIGHT * row_count.min(ROWS_SHOWN) as f32)
                            .mx(theme.spacing.xs)
                            .my(theme.spacing.xs)
                            .text_size(theme.text.ui.size)
                            .line_height(theme.text.ui.line_height)
                            .child(rows)
                            .child(Scrollbar::new("trash-scrollbar", self.scroll.clone())),
                    )
                    .when_some(nothing_to_show, |modal, message| {
                        modal.child(
                            div()
                                .px(theme.spacing.md)
                                .pb(theme.spacing.sm)
                                .child(EmptyState::new(message).inline()),
                        )
                    })
                    .child(Divider::horizontal())
                    .child(
                        div()
                            .px(theme.spacing.lg)
                            .py(theme.spacing.sm)
                            .text_size(theme.text.caption.size)
                            .text_color(theme.colors.text_placeholder)
                            .child(self.summary(cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    use super::*;
    use crate::app::first_run;
    use crate::workspace::storage::Storage;

    /// The welcome workspace with every page but the first moved to the trash, and the trash
    /// open.
    fn open_trash(cx: &mut TestAppContext) -> (Entity<TrashDialog>, &mut VisualTestContext) {
        cx.update(scroll_ui::init);
        let path =
            std::env::temp_dir().join(format!("scroll-trash-{}.sqlite", PageId::generate().0));
        let storage = Storage::open(&path).unwrap();
        let records = first_run::seed(&storage);
        let workspace = cx.new(|_| Workspace::new(path, storage, records));
        cx.update(|cx| {
            workspace.update(cx, |workspace, cx| {
                let top_level = workspace.tree().children(None);
                let pages: Vec<PageId> = top_level.iter().map(|page| page.id).collect();
                for page in pages.into_iter().skip(1) {
                    workspace.archive(page, cx);
                }
            })
        });
        cx.run_until_parked();
        let (dialog, cx) = cx.add_window_view(|window, cx| TrashDialog::new(workspace, window, cx));
        dialog.update_in(cx, |dialog, window, cx| {
            window.focus(&dialog.focus_handle(cx));
        });
        cx.run_until_parked();
        (dialog, cx)
    }

    fn found(dialog: &Entity<TrashDialog>, cx: &mut VisualTestContext) -> Vec<String> {
        dialog.read_with(cx, |dialog, _| dialog.found_titles())
    }

    fn is_in_tree(title: &str, dialog: &Entity<TrashDialog>, cx: &mut VisualTestContext) -> bool {
        dialog.read_with(cx, |dialog, cx| {
            let top_level = dialog.workspace.read(cx).tree().children(None);
            top_level.iter().any(|page| page.title == title)
        })
    }

    #[gpui::test]
    fn the_trash_lists_what_was_deleted_and_the_search_narrows_it(cx: &mut TestAppContext) {
        let (dialog, cx) = open_trash(cx);
        let everything = found(&dialog, cx);
        assert!(everything.len() >= 2, "{everything:?}");
        assert!(cx.debug_bounds("trashed-0").is_some(), "its rows are drawn");
        let note = dialog.read_with(cx, |dialog, _| {
            let now = seconds_since_1970(SystemTime::now());
            deletion_note(&dialog.trash.as_ref().unwrap()[0], now)
        });
        assert_eq!(note, "Deleted just now");

        let wanted = everything[1].clone();
        cx.simulate_input(&wanted.to_uppercase());
        assert_eq!(found(&dialog, cx), [wanted], "whatever the case typed");
        cx.simulate_input("no page is called this");
        assert!(found(&dialog, cx).is_empty());
    }

    #[gpui::test]
    fn return_restores_the_highlighted_page_and_leaves_the_rest(cx: &mut TestAppContext) {
        let (dialog, cx) = open_trash(cx);
        let everything = found(&dialog, cx);
        assert!(!is_in_tree(&everything[1], &dialog, cx));

        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        assert!(
            is_in_tree(&everything[1], &dialog, cx),
            "back in the sidebar"
        );
        let mut rest = everything.clone();
        rest.remove(1);
        assert_eq!(
            found(&dialog, cx),
            rest,
            "and out of the trash, which stays open"
        );
    }

    #[gpui::test]
    fn a_page_s_restore_button_restores_that_page(cx: &mut TestAppContext) {
        let (dialog, cx) = open_trash(cx);
        let everything = found(&dialog, cx);
        let row = cx.debug_bounds("trashed-0").unwrap();
        // The button is at the trailing end of the row.
        let button = gpui::point(row.right() - px(40.0), row.center().y);
        cx.simulate_click(button, Modifiers::none());
        cx.run_until_parked();
        assert!(is_in_tree(&everything[0], &dialog, cx));
        assert_eq!(found(&dialog, cx), everything[1..]);
    }

    #[gpui::test]
    fn the_button_on_the_row_under_the_pointer_restores_that_row_s_page(cx: &mut TestAppContext) {
        let (dialog, cx) = open_trash(cx);
        let everything = found(&dialog, cx);
        let row = cx.debug_bounds("trashed-1").unwrap();
        let button = gpui::point(row.right() - px(40.0), row.center().y);
        cx.simulate_mouse_move(button, None, Modifiers::none());
        cx.simulate_click(button, Modifiers::none());
        cx.run_until_parked();
        assert!(is_in_tree(&everything[1], &dialog, cx));
        assert_eq!(found(&dialog, cx).len(), everything.len() - 1);
    }

    #[test]
    fn how_long_ago_is_said_in_the_largest_unit_that_fits() {
        let (minute, hour, day) = (60, 3600, 86_400);
        assert_eq!(time_ago(5), "just now");
        assert_eq!(time_ago(-30), "just now", "the clock went back");
        assert_eq!(time_ago(minute), "1 minute ago");
        assert_eq!(time_ago(59 * minute), "59 minutes ago");
        assert_eq!(time_ago(hour), "1 hour ago");
        assert_eq!(time_ago(23 * hour), "23 hours ago");
        assert_eq!(time_ago(day), "yesterday");
        assert_eq!(time_ago(2 * day), "2 days ago");
        assert_eq!(time_ago(29 * day), "29 days ago");
        assert_eq!(time_ago(30 * day), "1 month ago");
        assert_eq!(time_ago(364 * day), "12 months ago");
        assert_eq!(time_ago(800 * day), "2 years ago");
    }

    #[test]
    fn the_note_says_what_is_known_of_when_and_where() {
        let page = scroll_core::record::Page {
            id: PageId::generate(),
            parent: None,
            title: "Plan".into(),
            icon: None,
            position: scroll_core::position::Position::between(None, None).unwrap(),
            row_of: None,
            is_archived: true,
            is_folder: false,
        };
        let mut trashed = TrashedPage {
            page,
            archived_at: Some(1_000),
            parent_title: Some("Projects".into()),
        };
        assert_eq!(
            deletion_note(&trashed, 1_000 + 3 * 86_400),
            "Deleted 3 days ago  ·  in Projects"
        );
        trashed.archived_at = None;
        trashed.parent_title = None;
        assert_eq!(deletion_note(&trashed, 5_000), "Deleted earlier");
    }
}
