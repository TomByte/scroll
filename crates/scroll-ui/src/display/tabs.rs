use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, AppContext, ClickEvent, Context, ElementId, Entity, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    RenderOnce, SharedString, Styled, Window, div, prelude::FluentBuilder,
};

use crate::controls::button::ButtonSize;
use crate::controls::icon_button::IconButton;
use crate::controls::text_input::TextInput;
use crate::display::icon::IconName;
use crate::foundation::focus::Clickable;
use crate::theme::Theme;

type TabHandler = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;
type TabMouseHandler = Rc<dyn Fn(&(usize, MouseDownEvent), &mut Window, &mut App) + 'static>;

/// A row of tabs, such as a database's views. The caller owns which tab is current. A tab can
/// be renamed in place (`editing`), and double-clicking or right-clicking one is reported.
#[derive(IntoElement)]
pub struct Tabs {
    id: ElementId,
    labels: Vec<SharedString>,
    selected: Option<usize>,
    editing: Option<(usize, AnyElement)>,
    end_slot: Option<AnyElement>,
    on_select: Option<TabHandler>,
    on_double_click: Option<TabHandler>,
    on_secondary_mouse_down: Option<TabMouseHandler>,
}

impl Tabs {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Tabs {
            id: id.into(),
            labels: Vec::new(),
            selected: None,
            editing: None,
            end_slot: None,
            on_select: None,
            on_double_click: None,
            on_secondary_mouse_down: None,
        }
    }

    pub fn tab(mut self, label: impl Into<SharedString>) -> Self {
        self.labels.push(label.into());
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// Shows `editor` (normally a `TextInput`) in place of tab `index`, while it is renamed.
    pub fn editing(mut self, index: usize, editor: impl IntoElement) -> Self {
        self.editing = Some((index, editor.into_any_element()));
        self
    }

    /// Content after the last tab, such as an add button.
    pub fn end_slot(mut self, element: impl IntoElement) -> Self {
        self.end_slot = Some(element.into_any_element());
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn on_double_click(
        mut self,
        handler: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_double_click = Some(Rc::new(handler));
        self
    }

    /// Right click (or Control-click) on a tab, typically to open its menu at the pointer.
    pub fn on_secondary_mouse_down(
        mut self,
        handler: impl Fn(&(usize, MouseDownEvent), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_secondary_mouse_down = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Tabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let mut editing = self.editing;
        let tabs: Vec<AnyElement> = self
            .labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                if let Some((_, editor)) = editing.take_if(|(editing, _)| *editing == index) {
                    return div()
                        .min_w(theme.size.tab_editor_min_width)
                        .px(theme.spacing.sm)
                        .child(editor)
                        .into_any_element();
                }
                let is_current = self.selected == Some(index);
                let id = ElementId::NamedChild(Box::new(self.id.clone()), index.to_string().into());
                let on_select = self.on_select.clone();
                let on_double_click = self.on_double_click.clone();
                let on_secondary = self.on_secondary_mouse_down.clone();
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .h(theme.size.control_medium)
                    .px(theme.spacing.md)
                    .rounded(theme.radius.md)
                    .text_size(theme.text.ui.size)
                    .font_weight(if is_current {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if is_current {
                        theme.colors.text
                    } else {
                        theme.colors.text_muted
                    })
                    .when(is_current, |tab| tab.bg(theme.colors.selected))
                    .child(label)
                    .map(|tab| {
                        Clickable::new(theme.radius.md)
                            .hover(theme.colors.hover)
                            .on_click(move |event: &ClickEvent, window, cx| {
                                let handler = if event.click_count() == 2 {
                                    on_double_click.as_ref()
                                } else {
                                    on_select.as_ref()
                                };
                                if let Some(handler) = handler {
                                    handler(&index, window, cx);
                                }
                            })
                            .apply(tab, window, cx)
                    })
                    .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        if let Some(handler) = on_secondary.as_ref() {
                            handler(&(index, event.clone()), window, cx);
                        }
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .items_center()
            .gap(theme.spacing.xxs)
            .children(tabs)
            .children(self.end_slot)
    }
}

impl Tabs {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let rename = cx.new(|cx| TextInput::new(cx).with_text("By owner"));
        cx.new(|_| TabsStory {
            selected: 0,
            rename,
        })
        .into()
    }
}

struct TabsStory {
    selected: usize,
    rename: Entity<TextInput>,
}

impl Render for TabsStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Tabs::new("views")
            .tab("All tasks")
            .tab("This week")
            .tab("By owner")
            .selected(self.selected)
            .editing(2, self.rename.clone())
            .on_select(cx.listener(|story, index: &usize, _, cx| {
                story.selected = *index;
                cx.notify();
            }))
            .end_slot(
                IconButton::new("add-view", IconName::Plus)
                    .size(ButtonSize::Small)
                    .tooltip("Add a view"),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyUpEvent, Keystroke, TestAppContext};

    use super::*;
    use crate::foundation::story::testing::open_story;

    #[gpui::test]
    fn tab_and_space_switch_to_the_next_tab(cx: &mut TestAppContext) {
        let (story, cx) = open_story(Tabs::story, cx);
        let story = story.downcast::<TabsStory>().expect("the tabs story");
        cx.simulate_keystrokes("tab tab space");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").expect("a key"),
        });
        assert_eq!(story.read_with(cx, |story, _| story.selected), 1);
    }
}
