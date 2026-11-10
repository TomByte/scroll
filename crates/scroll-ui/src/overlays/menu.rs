use std::rc::Rc;

use gpui::{
    App, Context, DismissEvent, ElementId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::display::divider::Divider;
use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::actions::{Confirm, MENU_CONTEXT, SelectNext, SelectPrevious};
use crate::lists::list_item::ListItem;
use crate::lists::section_header::SectionHeader;
use crate::theme::Theme;
use gpui::{AnyView, AppContext, Entity, MouseDownEvent};

use crate::overlays::popup_host::{PopupAnchor, PopupHost};

type SelectHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

pub struct MenuItem {
    label: SharedString,
    icon: Option<IconName>,
    shortcut: Option<SharedString>,
    is_checked: bool,
    on_select: SelectHandler,
}

impl MenuItem {
    pub fn new(
        label: impl Into<SharedString>,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        MenuItem {
            label: label.into(),
            icon: None,
            shortcut: None,
            is_checked: false,
            on_select: Rc::new(on_select),
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Text shown at the trailing edge, e.g. "⌘B".
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Marks the item as the current choice with a check at the trailing edge, e.g. the type
    /// a column already has.
    pub fn checked(mut self) -> Self {
        self.is_checked = true;
        self
    }
}

enum MenuEntry {
    Item(MenuItem),
    Separator,
    Header(SharedString),
}

/// A keyboard-navigable menu, opened by a `PopupHost`. Emits `DismissEvent` when an item is
/// chosen.
pub struct Menu {
    focus_handle: FocusHandle,
    entries: Vec<MenuEntry>,
    highlighted: Option<usize>,
}

impl EventEmitter<DismissEvent> for Menu {}

impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Menu {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Menu {
            focus_handle: cx.focus_handle(),
            entries: Vec::new(),
            highlighted: None,
        }
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.entries.push(MenuEntry::Item(item));
        self
    }

    /// Several items at once; an `Option` adds its item only when there is one.
    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.entries.extend(items.into_iter().map(MenuEntry::Item));
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(MenuEntry::Separator);
        self
    }

    pub fn header(mut self, title: impl Into<SharedString>) -> Self {
        self.entries.push(MenuEntry::Header(title.into()));
        self
    }

    fn item_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, MenuEntry::Item(_)))
            .map(|(index, _)| index)
            .collect()
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let items = self.item_indices();
        self.highlighted = match self.highlighted {
            None => items.first().copied(),
            Some(current) => items
                .iter()
                .copied()
                .find(|&index| index > current)
                .or(items.first().copied()),
        };
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        let items = self.item_indices();
        self.highlighted = match self.highlighted {
            None => items.last().copied(),
            Some(current) => items
                .iter()
                .rev()
                .copied()
                .find(|&index| index < current)
                .or(items.last().copied()),
        };
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.highlighted {
            self.choose(index, window, cx);
        }
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(MenuEntry::Item(item)) = self.entries.get(index) else {
            return;
        };
        let on_select = item.on_select.clone();
        cx.emit(DismissEvent);
        on_select(window, cx);
    }

    fn highlight(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.highlighted != Some(index) {
            self.highlighted = Some(index);
            cx.notify();
        }
    }
}

impl Render for Menu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let rows = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                MenuEntry::Separator => div()
                    .py(theme.spacing.xs)
                    .child(Divider::horizontal())
                    .into_any_element(),
                MenuEntry::Header(title) => SectionHeader::new(title.clone()).into_any_element(),
                MenuEntry::Item(item) => {
                    let is_highlighted = self.highlighted == Some(index);
                    div()
                        .id(ElementId::Integer(index as u64))
                        .on_mouse_move(cx.listener(move |menu, _, _, cx| menu.highlight(index, cx)))
                        .child(
                            ListItem::new(ElementId::Integer(index as u64), item.label.clone())
                                .emphasized()
                                .when_some(item.icon, ListItem::icon)
                                .when_some(item.shortcut.clone(), |row, shortcut| {
                                    row.end_slot(shortcut)
                                })
                                .when(item.is_checked, |row| {
                                    row.end_slot(Icon::new(IconName::Check).size(IconSize::Small))
                                })
                                .when(is_highlighted, ListItem::highlighted)
                                .on_click(cx.listener(move |menu, _, window, cx| {
                                    menu.choose(index, window, cx)
                                })),
                        )
                        .into_any_element()
                }
            });

        div()
            .track_focus(&self.focus_handle)
            .key_context(MENU_CONTEXT)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .flex()
            .flex_col()
            .children(rows)
    }
}

impl Menu {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let popups = cx.new(|_| PopupHost::new());
        cx.new(|_| MenuStory { popups }).into()
    }
}

struct MenuStory {
    popups: Entity<PopupHost>,
}

impl Render for MenuStory {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let popups = self.popups.clone();
        div()
            .child(
                ListItem::new("context-row", "Right-click this row")
                    .icon(IconName::Page)
                    .on_click(|_, _, _| {})
                    .on_secondary_mouse_down(move |event: &MouseDownEvent, window, cx| {
                        let position = event.position;
                        popups.update(cx, |popups, cx| {
                            popups.show(
                                PopupAnchor::Point(position),
                                |_, cx| {
                                    Menu::new(cx)
                                        .header("Page")
                                        .item(
                                            MenuItem::new("Duplicate", |_, _| {})
                                                .icon(IconName::Plus)
                                                .shortcut("⌘D"),
                                        )
                                        .item(
                                            MenuItem::new("Full width", |_, _| {})
                                                .icon(IconName::Text)
                                                .checked(),
                                        )
                                        .separator()
                                        .item(
                                            MenuItem::new("Move to trash", |_, _| {})
                                                .icon(IconName::Trash),
                                        )
                                },
                                window,
                                cx,
                            );
                        });
                    }),
            )
            .child(self.popups.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Menu::story, cx);
    }
}
