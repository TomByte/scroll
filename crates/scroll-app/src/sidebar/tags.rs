//! The sidebar's tags: filtering pages by tag, and each tag's menu.

use crate::workspace::page_tree::display_title;
use crate::workspace::tag_colors::{TAG_COLORS, swatch_of};
use gpui::{
    AnyElement, Context, ElementId, IntoElement, Pixels, Point, Styled, Window, div,
    prelude::FluentBuilder,
};
use scroll_core::id::{PageId, TagId};
use scroll_core::operation::{RecordId, TagField, Transaction};
use scroll_core::record::TagColor;
use scroll_ui::{IconName, ListItem, Menu, MenuItem, PopupAnchor, SectionHeader, Theme};

use super::*;
use crate::overlays::prompt_dialog::PromptText;

impl Sidebar {
    pub(super) fn toggle_tag(&mut self, tag: TagId, cx: &mut Context<Self>) {
        match self
            .selected_tags
            .iter()
            .position(|selected| *selected == tag)
        {
            Some(index) => {
                self.selected_tags.remove(index);
            }
            None => self.selected_tags.push(tag),
        }
        self.load_tagged_pages(cx);
        cx.notify();
    }

    /// Reads the pages carrying every selected tag on the storage thread.
    pub(super) fn load_tagged_pages(&mut self, cx: &mut Context<Self>) {
        if self.selected_tags.is_empty() {
            self.tagged_pages.clear();
            return;
        }
        let tags = self.selected_tags.clone();
        let reading = self.workspace.read(cx).storage().read(cx, move |store| {
            Ok(store.pages_with_tags(&tags, TAGGED_LIMIT)?)
        });
        cx.spawn(async move |sidebar, cx| match reading.await {
            Ok(pages) => {
                let shown = sidebar.update(cx, |sidebar, cx| {
                    sidebar.tagged_pages = pages;
                    cx.notify();
                });
                if shown.is_err() {
                    log::debug!("sidebar closed before tagged pages arrived");
                }
            }
            Err(error) => log::error!("could not read tagged pages: {error}"),
        })
        .detach();
    }

    pub(super) fn open_tag_menu(
        &mut self,
        tag: TagId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let workspace = self.workspace.clone();
        let overlays = self.overlays.clone();
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| {
                    let rename = {
                        let workspace = workspace.clone();
                        move |window: &mut Window, cx: &mut gpui::App| {
                            rename_tag(tag, &workspace, &overlays, window, cx)
                        }
                    };
                    let mut menu = Menu::new(cx)
                        .item(MenuItem::new("Rename tag…", rename).icon(IconName::Pencil));
                    menu = menu.separator().header("Colour");
                    for (color, name) in TAG_COLORS {
                        let workspace = workspace.clone();
                        menu = menu.item(MenuItem::new(name, move |_, cx: &mut gpui::App| {
                            workspace.update(cx, |workspace, cx| {
                                workspace.commit(
                                    Transaction::set_tag_field(tag, TagField::Color(color)),
                                    cx,
                                )
                            });
                        }));
                    }
                    let workspace = workspace.clone();
                    menu.separator().item(
                        MenuItem::new("Delete tag", move |_, cx: &mut gpui::App| {
                            workspace.update(cx, |workspace, cx| {
                                workspace.commit(Transaction::delete(RecordId::Tag(tag)), cx)
                            });
                        })
                        .icon(IconName::Trash),
                    )
                },
                window,
                cx,
            );
        });
    }

    /// Tags with how many pages carry each; picking tags lists the pages carrying them all.
    pub(super) fn render_tags(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let records = self.workspace.read(cx).records();
        let tags: Vec<(TagId, String, TagColor, usize)> = records
            .tags_with_counts()
            .into_iter()
            .map(|(tag, count)| (tag.id, tag.name.clone(), tag.color, count))
            .collect();
        if tags.is_empty() {
            return Vec::new();
        }
        let tagged: Vec<(PageId, String, Option<String>)> = self
            .tagged_pages
            .iter()
            .map(|page| {
                (
                    page.id,
                    display_title(&page.title).to_string(),
                    page.icon.clone(),
                )
            })
            .collect();
        let mut rows = vec![SectionHeader::new("Tags").into_any_element()];
        for (id, name, color, count) in tags {
            let is_selected = self.selected_tags.contains(&id);
            let dot = theme.swatch(swatch_of(color));
            rows.push(
                ListItem::new(
                    ElementId::Name(format!("sidebar-tag-{}", id.0).into()),
                    format!("#{name}"),
                )
                .start_slot(div().size(theme.size.icon_xsmall).rounded_full().bg(dot))
                .end_slot(count.to_string())
                .when(is_selected, ListItem::selected)
                .on_click(cx.listener(move |sidebar, _, _, cx| sidebar.toggle_tag(id, cx)))
                .on_secondary_mouse_down(cx.listener(
                    move |sidebar, event: &gpui::MouseDownEvent, window, cx| {
                        sidebar.open_tag_menu(id, event.position, window, cx)
                    },
                ))
                .into_any_element(),
            );
        }
        if !self.selected_tags.is_empty() {
            let label = if tagged.is_empty() {
                "No page has all these tags"
            } else {
                "Tagged pages"
            };
            rows.push(SectionHeader::new(label).into_any_element());
            for (page, title, icon) in tagged {
                rows.push(
                    ListItem::new(ElementId::Name(format!("tagged-{}", page.0).into()), title)
                        .indent(theme.size.tree_indent)
                        .icon(IconName::Page)
                        .when_some(icon, ListItem::emoji)
                        .on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::OpenPage(page))),
                        )
                        .into_any_element(),
                );
            }
        }
        rows
    }
}

const RENAME_TAG: PromptText = PromptText {
    title: "Rename tag",
    explanation: "Pages keep the tag; only its name changes.",
    placeholder: "Tag name",
    action: "Rename",
};

fn rename_tag(
    tag: TagId,
    workspace: &Entity<Workspace>,
    overlays: &Overlays,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let workspace = workspace.clone();
    overlays.prompt(
        RENAME_TAG,
        move |name, cx| {
            let field = TagField::Name(name);
            workspace.update(cx, |workspace, cx| {
                workspace.commit(Transaction::set_tag_field(tag, field), cx)
            });
            Task::ready(Ok(()))
        },
        window,
        cx,
    );
}
