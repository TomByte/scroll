use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, MouseDownEvent,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::display::icon::{Icon, IconName, IconSize};
use crate::lists::list_item::ListItem;
use crate::theme::Theme;
use crate::{ClickHandler, MouseDownHandler};
use gpui::{AnyView, AppContext, Context, Entity, Render};

use crate::controls::text_input::TextInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disclosure {
    /// No children: the chevron's space is kept so labels stay aligned.
    Leaf,
    Collapsed,
    Expanded,
}

/// A sidebar tree row: a list item indented by depth with a disclosure chevron.
#[derive(IntoElement)]
pub struct TreeItem {
    id: ElementId,
    label: SharedString,
    depth: usize,
    disclosure: Disclosure,
    icon: Option<IconName>,
    emoji: Option<SharedString>,
    is_selected: bool,
    on_click: Option<ClickHandler>,
    on_toggle: Option<ClickHandler>,
    on_secondary_mouse_down: Option<MouseDownHandler>,
    hover_end_slot: Option<AnyElement>,
    editor: Option<AnyElement>,
}

impl TreeItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        TreeItem {
            id: id.into(),
            label: label.into(),
            depth: 0,
            disclosure: Disclosure::Leaf,
            icon: None,
            emoji: None,
            is_selected: false,
            on_click: None,
            on_toggle: None,
            on_secondary_mouse_down: None,
            hover_end_slot: None,
            editor: None,
        }
    }

    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn disclosure(mut self, disclosure: Disclosure) -> Self {
        self.disclosure = disclosure;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// An emoji in the icon's place, such as the page's own icon.
    pub fn emoji(mut self, emoji: impl Into<SharedString>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }

    pub fn selected(mut self) -> Self {
        self.is_selected = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Buttons at the trailing edge, shown while the pointer is over the row.
    pub fn hover_end_slot(mut self, element: impl IntoElement) -> Self {
        self.hover_end_slot = Some(element.into_any_element());
        self
    }

    /// Shows `editor` in place of the label, lined up with it, while the row is renamed.
    pub fn editing(mut self, editor: impl IntoElement) -> Self {
        self.editor = Some(editor.into_any_element());
        self
    }

    pub fn on_secondary_mouse_down(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_secondary_mouse_down = Some(Box::new(handler));
        self
    }

    /// Called when the chevron is clicked; the row's own click handler does not fire.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for TreeItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let chevron_icon = match self.disclosure {
            Disclosure::Leaf => None,
            Disclosure::Collapsed => Some(IconName::ChevronRight),
            Disclosure::Expanded => Some(IconName::ChevronDown),
        };
        let chevron_id = ElementId::Name(format!("{}-disclosure", self.id).into());
        let chevron = div()
            .id(chevron_id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(theme.size.icon_large)
            .rounded(theme.radius.sm)
            .when_some(chevron_icon, |chevron, icon| {
                chevron.hover(|style| style.bg(theme.colors.hover)).child(
                    Icon::new(icon)
                        .size(IconSize::XSmall)
                        .color(theme.colors.text_placeholder),
                )
            })
            .when_some(self.on_toggle, |chevron, handler| {
                chevron.on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx)
                })
            });

        let indent = theme.size.tree_indent * self.depth as f32;
        ListItem::new(self.id, self.label)
            .indent(indent)
            .start_slot(chevron)
            .when_some(self.icon, ListItem::icon)
            .when_some(self.emoji, ListItem::emoji)
            .when(self.is_selected, ListItem::selected)
            .when_some(self.hover_end_slot, ListItem::hover_end_slot)
            .when_some(self.editor, ListItem::editing)
            .when_some(self.on_secondary_mouse_down, |row, handler| {
                row.on_secondary_mouse_down(handler)
            })
            .when_some(self.on_click, |row, handler| row.on_click(handler))
    }
}

impl TreeItem {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let rename = cx.new(|cx| TextInput::new(cx).plain().with_text("Meeting notes"));
        cx.new(|_| TreeItemStory {
            is_expanded: true,
            rename,
        })
        .into()
    }
}

struct TreeItemStory {
    is_expanded: bool,
    rename: Entity<TextInput>,
}

impl Render for TreeItemStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disclosure = if self.is_expanded {
            Disclosure::Expanded
        } else {
            Disclosure::Collapsed
        };
        div()
            .flex()
            .flex_col()
            .child(
                TreeItem::new("tree-product", "Product")
                    .icon(IconName::Page)
                    .disclosure(disclosure)
                    .on_toggle(cx.listener(|story, _, _, cx| {
                        story.is_expanded = !story.is_expanded;
                        cx.notify();
                    })),
            )
            .when(self.is_expanded, |tree| {
                tree.child(
                    TreeItem::new("tree-roadmap", "Roadmap")
                        .icon(IconName::Page)
                        .depth(1)
                        .selected(),
                )
                .child(
                    TreeItem::new("tree-principles", "Design principles")
                        .icon(IconName::Page)
                        .depth(1)
                        .disclosure(Disclosure::Collapsed)
                        .hover_end_slot(Icon::new(IconName::Plus).size(IconSize::Small))
                        .on_click(|_, _, _| {}),
                )
            })
            .child(
                TreeItem::new("tree-notes", "")
                    .icon(IconName::Page)
                    .editing(self.rename.clone()),
            )
            .child(
                TreeItem::new("tree-garden", "Garden plans")
                    .icon(IconName::Page)
                    .emoji("🌱"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(TreeItem::story, cx);
    }
}
