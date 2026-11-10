use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px,
};

use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::ellipsis::Ellipsis;
use crate::foundation::focus::Clickable;
use crate::motion::Transition;
use crate::theme::Theme;
use crate::{ClickHandler, MouseDownHandler};
use gpui::{AnyView, AppContext, Context, Entity, Render};

use crate::controls::text_input::TextInput;

/// A single-line row for lists, menus and sidebars, with hover and selected states.
#[derive(IntoElement)]
pub struct ListItem {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    emoji: Option<SharedString>,
    indent: Pixels,
    start_slot: Option<AnyElement>,
    end_slot: Option<AnyElement>,
    hover_end_slot: Option<AnyElement>,
    editor: Option<AnyElement>,
    is_selected: bool,
    is_highlighted: bool,
    is_emphasized: bool,
    on_click: Option<ClickHandler>,
    on_secondary_mouse_down: Option<MouseDownHandler>,
}

impl ListItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        ListItem {
            id: id.into(),
            label: label.into(),
            icon: None,
            emoji: None,
            indent: px(0.0),
            start_slot: None,
            end_slot: None,
            hover_end_slot: None,
            editor: None,
            is_selected: false,
            is_highlighted: false,
            is_emphasized: false,
            on_click: None,
            on_secondary_mouse_down: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// An emoji in the icon's place; it wins over `icon`.
    pub fn emoji(mut self, emoji: impl Into<SharedString>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }

    /// Extra leading space, used for nesting.
    pub fn indent(mut self, indent: Pixels) -> Self {
        self.indent = indent;
        self
    }

    /// Content placed before the icon, such as a disclosure chevron.
    pub fn start_slot(mut self, element: impl IntoElement) -> Self {
        self.start_slot = Some(element.into_any_element());
        self
    }

    /// Content aligned to the trailing edge, such as a shortcut or a count.
    pub fn end_slot(mut self, element: impl IntoElement) -> Self {
        self.end_slot = Some(element.into_any_element());
        self
    }

    /// Content at the trailing edge that shows only while the pointer is over the row, such
    /// as the row's own buttons.
    pub fn hover_end_slot(mut self, element: impl IntoElement) -> Self {
        self.hover_end_slot = Some(element.into_any_element());
        self
    }

    /// Shows `editor` (normally a plain `TextInput`) where the label is, lined up with it,
    /// e.g. while the row is renamed in place.
    pub fn editing(mut self, editor: impl IntoElement) -> Self {
        self.editor = Some(editor.into_any_element());
        self
    }

    /// The row is the current item, e.g. the open page.
    pub fn selected(mut self) -> Self {
        self.is_selected = true;
        self
    }

    /// The row has keyboard focus within a menu or list, without being chosen.
    pub fn highlighted(mut self) -> Self {
        self.is_highlighted = true;
        self
    }

    /// Full-contrast text even when not selected, as in menus. Sidebar rows stay muted.
    pub fn emphasized(mut self) -> Self {
        self.is_emphasized = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Right click (or Control-click), typically to open a context menu at the pointer.
    pub fn on_secondary_mouse_down(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_secondary_mouse_down = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ListItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let colors = &theme.colors;
        let text_color = if self.is_selected || self.is_emphasized {
            colors.text
        } else {
            colors.text_muted
        };
        let icon_color = if self.is_selected {
            colors.text
        } else {
            colors.icon
        };
        let hover_background = if self.is_selected {
            colors.selected
        } else {
            colors.hover
        };
        let is_editing = self.editor.is_some();
        let id = self.id;
        let selected_amount =
            Transition::Select.fraction_on(id.clone(), self.is_selected, window, cx);
        let background = if is_editing {
            Some(colors.selected)
        } else if selected_amount > 0.0 {
            Some(colors.selected.opacity(selected_amount))
        } else if self.is_highlighted {
            Some(colors.hover)
        } else {
            None
        };

        let group = SharedString::from(format!("list-item-{id}"));
        div()
            .id(id.clone())
            .group(group.clone())
            .flex()
            .items_center()
            .gap(theme.spacing.sm)
            .h(theme.size.row_height)
            .pl(theme.spacing.sm + self.indent)
            .pr(theme.spacing.sm)
            .rounded(theme.radius.md)
            .text_size(theme.text.ui.size)
            .line_height(theme.text.ui.line_height)
            .text_color(text_color)
            .when_some(background, |row, background| row.bg(background))
            .when(!is_editing, |row| {
                row.hover(|style| style.bg(hover_background).text_color(colors.text))
                    .active(|style| style.bg(colors.pressed))
            })
            .when_some(self.start_slot, |row, slot| row.child(slot))
            .map(|row| match (self.emoji, self.icon) {
                (Some(emoji), _) => row.child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .w(theme.size.icon_medium)
                        .text_size(theme.text.ui.size)
                        .child(emoji),
                ),
                (None, Some(icon)) => {
                    row.child(Icon::new(icon).size(IconSize::Medium).color(icon_color))
                }
                (None, None) => row,
            })
            .map(|row| match self.editor {
                Some(editor) => row.child(div().flex_1().min_w_0().child(editor)),
                None => row.child(div().flex_1().min_w_0().ellipsis().child(self.label)),
            })
            .when_some(self.end_slot, |row, slot| {
                row.child(
                    div()
                        .flex_none()
                        .text_color(colors.text_placeholder)
                        .child(slot),
                )
            })
            .when_some(self.hover_end_slot, |row, slot| {
                row.child(
                    div()
                        .flex()
                        .flex_none()
                        .invisible()
                        .group_hover(group, |slot| slot.visible())
                        .child(slot),
                )
            })
            .when_some(self.on_secondary_mouse_down, |row, handler| {
                row.on_mouse_down(MouseButton::Right, handler)
            })
            .when_some(self.on_click.filter(|_| !is_editing), |row, handler| {
                Clickable::new(theme.radius.md)
                    .on_click(handler)
                    .apply(row, window, cx)
            })
    }
}

impl ListItem {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let rename = cx.new(|cx| TextInput::new(cx).plain().with_text("Rename me in place"));
        cx.new(|_| ListItemStory {
            selected: 1,
            rename,
        })
        .into()
    }
}

struct ListItemStory {
    selected: usize,
    rename: Entity<TextInput>,
}

impl Render for ListItemStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let rows = [
            ("Inbox", IconName::Page, "12"),
            ("Search", IconName::Search, "⌘P"),
            ("Trash", IconName::Trash, ""),
        ];
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xxs)
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(index, (label, icon, hint))| {
                        ListItem::new(("row", index), label)
                            .icon(icon)
                            .end_slot(hint)
                            .when(self.selected == index, ListItem::selected)
                            .on_click(cx.listener(move |story, _, _, cx| {
                                story.selected = index;
                                cx.notify();
                            }))
                    }),
            )
            .child(
                ListItem::new("highlighted", "Highlighted (keyboard)")
                    .icon(IconName::Text)
                    .highlighted(),
            )
            .child(ListItem::new("emoji-row", "Page with an emoji icon").emoji("🚀"))
            .child(
                ListItem::new("hover-row", "Hover for actions")
                    .icon(IconName::Page)
                    .hover_end_slot(Icon::new(IconName::Ellipsis).size(IconSize::Small))
                    .on_click(|_, _, _| {}),
            )
            .child(
                ListItem::new("editing-row", "")
                    .icon(IconName::Page)
                    .editing(self.rename.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(ListItem::story, cx);
    }
}
