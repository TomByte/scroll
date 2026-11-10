//! One row of the page tree: its title, actions, renaming in place, and dragging pages between rows.

use crate::page::commands::NEW_SUB_PAGE;
use crate::workspace::page_tree::{PageDrop, TreeRow, display_title};
use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder,
};
use scroll_core::id::PageId;
use scroll_ui::{
    ButtonSize, Disclosure, DragPreview, DropIndicator, IconButton, IconName, Theme, TreeItem,
};

use super::*;

impl Sidebar {
    pub(super) fn render_row(
        &self,
        row: &TreeRow,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = row.id;
        // Viewers can open pages but not add, move or rename them.
        let can_edit = !self.workspace.read(cx).is_read_only();
        let (title, icon) = self
            .workspace
            .read(cx)
            .tree()
            .page(page)
            .map(|stored| {
                (
                    display_title(&stored.title).to_string(),
                    stored.icon.clone(),
                )
            })
            .unwrap_or_default();
        let group = SharedString::from(format!("page-row-{}", page.0));
        let rename_input = self
            .renaming
            .as_ref()
            .filter(|renaming| renaming.page == page)
            .map(|renaming| renaming.input.clone());
        let is_renaming = rename_input.is_some();
        let disclosure = match (row.has_children, self.expanded.contains(&page)) {
            (false, _) => Disclosure::Leaf,
            (true, true) => Disclosure::Expanded,
            (true, false) => Disclosure::Collapsed,
        };
        let hover_actions = div()
            .flex()
            .gap(theme.spacing.xxs)
            .child(
                IconButton::new(
                    ElementId::Name(format!("more-{}", page.0).into()),
                    IconName::Ellipsis,
                )
                .size(ButtonSize::Small)
                .tooltip("More")
                .on_click(cx.listener(
                    move |sidebar, event: &gpui::ClickEvent, window, cx| {
                        sidebar.open_context_menu(page, event.position(), window, cx)
                    },
                )),
            )
            .when(can_edit, |actions| {
                actions.child(
                    IconButton::new(
                        ElementId::Name(format!("add-{}", page.0).into()),
                        IconName::Plus,
                    )
                    .size(ButtonSize::Small)
                    .tooltip("Add a page inside")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(SidebarEvent::RunCommand(NEW_SUB_PAGE, Some(page)))
                    })),
                )
            });
        let drop_here = self.drop.filter(|drop| match drop {
            PageDrop::Before(target) | PageDrop::After(target) | PageDrop::Inside(target) => {
                *target == page
            }
        });
        let indicator_inset = theme.size.tree_indent * row.depth as f32;
        let is_database = self
            .workspace
            .read(cx)
            .records()
            .database_hosted_by(page)
            .is_some();
        let dragged = DraggedPage {
            id: page,
            title: title.clone().into(),
        };

        div()
            .id(ElementId::Name(group.clone()))
            .group(group)
            .relative()
            .rounded(theme.radius.md)
            .when(matches!(drop_here, Some(PageDrop::Inside(_))), |row| {
                row.bg(theme.colors.text_selection)
            })
            .when(can_edit && !is_renaming, |row| {
                row.on_drag(dragged, |dragged, _, _, cx| {
                    cx.new(|_| DragPreview::new(dragged.title.clone()))
                })
            })
            .on_drag_move(
                cx.listener(move |sidebar, event: &DragMoveEvent<DraggedPage>, _, cx| {
                    sidebar.drag_over_row(page, event, cx)
                }),
            )
            .on_drop(
                cx.listener(|sidebar, dragged: &DraggedPage, _, cx| sidebar.drop_page(dragged, cx)),
            )
            .child(
                TreeItem::new(ElementId::Name(format!("page-{}", page.0).into()), title)
                    .depth(row.depth)
                    .icon(if is_database {
                        IconName::Table
                    } else {
                        IconName::Page
                    })
                    .when_some(icon, TreeItem::emoji)
                    .when_some(rename_input, TreeItem::editing)
                    .disclosure(disclosure)
                    .when(self.current_page == Some(page), TreeItem::selected)
                    .hover_end_slot(hover_actions)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::OpenPage(page))))
                    .on_toggle(
                        cx.listener(move |sidebar, _, _, cx| sidebar.toggle_expanded(page, cx)),
                    )
                    .on_secondary_mouse_down(cx.listener(
                        move |sidebar, event: &gpui::MouseDownEvent, window, cx| {
                            sidebar.open_context_menu(page, event.position, window, cx)
                        },
                    )),
            )
            .when_some(drop_here, |row, drop| match drop {
                PageDrop::Before(_) => row.child(DropIndicator::above().inset(indicator_inset)),
                PageDrop::After(_) => row.child(DropIndicator::below().inset(indicator_inset)),
                PageDrop::Inside(_) => row,
            })
            .into_any_element()
    }

    /// Top quarter drops before, bottom quarter after, the middle inside.
    pub(super) fn drag_over_row(
        &mut self,
        row: PageId,
        event: &DragMoveEvent<DraggedPage>,
        cx: &mut Context<Self>,
    ) {
        let bounds = event.bounds;
        if !bounds.contains(&event.event.position) {
            return;
        }
        let dragged = event.drag(cx).id;
        let fraction = (event.event.position.y - bounds.top()) / bounds.size.height;
        let drop = match fraction {
            fraction if fraction < 0.25 => PageDrop::Before(row),
            fraction if fraction > 0.75 => PageDrop::After(row),
            _ => PageDrop::Inside(row),
        };
        let is_into_itself = self
            .workspace
            .read(cx)
            .tree()
            .ancestors_and_self(row)
            .contains(&dragged);
        let drop = (!is_into_itself).then_some(drop);
        if drop != self.drop {
            self.drop = drop;
            cx.notify();
        }
    }

    pub(super) fn drop_page(&mut self, dragged: &DraggedPage, cx: &mut Context<Self>) {
        let Some(drop) = self.drop.take() else {
            return;
        };
        if let PageDrop::Inside(parent) = drop
            && self.expanded.insert(parent)
        {
            self.rows = None;
            self.load_expanded_children(cx);
            cx.emit(SidebarEvent::ExpandedChanged);
        }
        let page = dragged.id;
        self.workspace
            .update(cx, |workspace, cx| workspace.move_page(page, drop, cx));
    }
}
