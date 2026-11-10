//! The trash at the bottom of the sidebar: its pages, and bringing one back.

use crate::workspace::page_tree::display_title;
use gpui::{
    AnyElement, Context, ElementId, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder,
};
use scroll_core::id::PageId;
use scroll_ui::{ButtonSize, IconButton, IconName, ListItem, Theme};

use super::*;

impl Sidebar {
    pub(super) fn toggle_trash(&mut self, cx: &mut Context<Self>) {
        self.is_trash_open = !self.is_trash_open;
        if self.is_trash_open {
            self.load_trash(cx);
        }
        cx.notify();
    }

    pub(super) fn load_trash(&mut self, cx: &mut Context<Self>) {
        let reading = self
            .workspace
            .read(cx)
            .storage()
            .read(cx, |store| Ok(store.archived_pages()?));
        cx.spawn(async move |sidebar, cx| match reading.await {
            Ok(pages) => {
                let shown = sidebar.update(cx, |sidebar, cx| {
                    sidebar.trash = pages;
                    cx.notify();
                });
                if shown.is_err() {
                    log::debug!("sidebar closed before the trash was read");
                }
            }
            Err(error) => log::error!("could not read the trash: {error}"),
        })
        .detach();
    }

    pub(super) fn render_trash(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let can_edit = !self.workspace.read(cx).is_read_only();
        let trashed: Vec<(PageId, String)> = self
            .trash
            .iter()
            .map(|page| (page.id, display_title(&page.title).to_string()))
            .collect();
        let count = if self.is_trash_open { trashed.len() } else { 0 };
        div()
            .flex()
            .flex_col()
            .child(
                ListItem::new("trash", "Trash")
                    .icon(IconName::Trash)
                    .end_slot(if count == 0 {
                        String::new()
                    } else {
                        count.to_string()
                    })
                    .on_click(cx.listener(|sidebar, _, _, cx| sidebar.toggle_trash(cx))),
            )
            .when(self.is_trash_open, |section| {
                section.children(trashed.into_iter().map(|(page, title)| {
                    ListItem::new(ElementId::Name(format!("trashed-{}", page.0).into()), title)
                        .indent(theme.size.tree_indent)
                        .icon(IconName::Page)
                        .when(can_edit, |item| {
                            item.end_slot(
                                IconButton::new(
                                    ElementId::Name(format!("restore-{}", page.0).into()),
                                    IconName::Restore,
                                )
                                .size(ButtonSize::Small)
                                .tooltip("Restore")
                                .on_click(
                                    cx.listener(move |sidebar, _, _, cx| sidebar.restore(page, cx)),
                                ),
                            )
                        })
                        .into_any_element()
                }))
            })
            .into_any_element()
    }

    pub(super) fn restore(&mut self, page: PageId, cx: &mut Context<Self>) {
        self.trash.retain(|trashed| trashed.id != page);
        self.workspace
            .update(cx, |workspace, cx| workspace.restore(page, cx));
        cx.emit(SidebarEvent::OpenPage(page));
    }
}
