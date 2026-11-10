//! A page's tags under its title, as coloured pills, each removable, with a button that asks for
//! a tag to add (an existing one, or a new name) in the palette.

use gpui::{
    Context, ElementId, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, prelude::FluentBuilder,
};
use scroll_core::id::{PageId, TagId};
use scroll_core::operation::{RecordId, Transaction};
use scroll_core::record::TagColor;
use scroll_ui::{Button, ButtonSize, ButtonVariant, IconName, Pill, Theme};

use crate::commands::RunCommand;
use crate::page::commands::ADD_TAG;
use crate::workspace::tag_colors::swatch_of;
use crate::workspace::{Workspace, WorkspaceEvent};

pub struct PageTags {
    page: PageId,
    workspace: Entity<Workspace>,
    _subscription: Subscription,
}

impl PageTags {
    pub fn new(page: PageId, workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&workspace, |_, _, event: &WorkspaceEvent, cx| {
            if matches!(event, WorkspaceEvent::RecordsChanged(_)) {
                cx.notify();
            }
        });
        PageTags {
            page,
            workspace,
            _subscription: subscription,
        }
    }

    fn remove(&mut self, tag: TagId, cx: &mut Context<Self>) {
        let page = self.page;
        self.workspace.update(cx, |workspace, cx| {
            workspace.commit(Transaction::delete(RecordId::PageTag { page, tag }), cx)
        });
    }
}

impl Render for PageTags {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let tags: Vec<(TagId, String, TagColor)> = self
            .workspace
            .read(cx)
            .records()
            .tags_of(self.page)
            .into_iter()
            .map(|tag| (tag.id, tag.name.clone(), tag.color))
            .collect();
        let page = self.page;
        let can_edit = !self.workspace.read(cx).is_read_only();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme.spacing.xs)
            .pt(theme.spacing.sm)
            .children(tags.into_iter().map(|(id, name, color)| {
                let pill = Pill::new(format!("#{name}")).swatch(swatch_of(color));
                match can_edit {
                    true => pill.on_remove(
                        ElementId::Name(format!("tag-{}", id.0).into()),
                        cx.listener(move |this, _, _, cx| this.remove(id, cx)),
                    ),
                    false => pill,
                }
            }))
            .when(can_edit, |tags| {
                tags.child(
                    Button::new("add-tag", "Tag")
                        .icon(IconName::Hash)
                        .size(ButtonSize::Small)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(RunCommand {
                                    id: ADD_TAG,
                                    page: Some(page),
                                }),
                                cx,
                            )
                        }),
                )
            })
    }
}
