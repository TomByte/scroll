use gpui::{
    AnyView, App, AppContext, Context, ElementId, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, Styled, Window, div,
};

use crate::controls::button::{Button, ButtonSize, ButtonVariant};
use crate::theme::Theme;

type SelectHandler = std::rc::Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

/// A row of mutually exclusive choices, such as "Sign in" / "Create account". Each choice is a
/// button, so Tab reaches it and Space or Return picks it. The caller owns the selection.
#[derive(IntoElement)]
pub struct SegmentedControl {
    id: ElementId,
    options: Vec<SharedString>,
    selected: usize,
    size: ButtonSize,
    on_select: Option<SelectHandler>,
}

impl SegmentedControl {
    pub fn new(id: impl Into<ElementId>) -> Self {
        SegmentedControl {
            id: id.into(),
            options: Vec::new(),
            selected: 0,
            size: ButtonSize::default(),
            on_select: None,
        }
    }

    pub fn option(mut self, label: impl Into<SharedString>) -> Self {
        self.options.push(label.into());
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Called with the index of the choice picked.
    pub fn on_select(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(std::rc::Rc::new(handler));
        self
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let options = self.options.into_iter().enumerate().map(|(index, label)| {
            let handler = self.on_select.clone();
            Button::new(
                ElementId::NamedChild(Box::new(self.id.clone()), index.to_string().into()),
                label,
            )
            .size(self.size)
            .variant(if index == self.selected {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Ghost
            })
            .on_click(move |_, window, cx| {
                if let Some(handler) = handler.as_ref() {
                    handler(&index, window, cx);
                }
            })
        });
        div()
            .flex()
            .flex_wrap()
            .gap(theme.spacing.xs)
            .children(options)
    }
}

impl SegmentedControl {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| SegmentedControlStory { selected: 0 }).into()
    }
}

struct SegmentedControlStory {
    selected: usize,
}

impl Render for SegmentedControlStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SegmentedControl::new("mode")
            .option("Sign in")
            .option("Create account")
            .selected(self.selected)
            .on_select(cx.listener(|story, index: &usize, _, cx| {
                story.selected = *index;
                cx.notify();
            }))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyUpEvent, Keystroke, TestAppContext};

    use super::*;
    use crate::foundation::story::testing::open_story;

    #[gpui::test]
    fn tab_and_space_pick_the_next_choice(cx: &mut TestAppContext) {
        let (story, cx) = open_story(SegmentedControl::story, cx);
        let story = story
            .downcast::<SegmentedControlStory>()
            .expect("the segmented control story");
        cx.simulate_keystrokes("tab tab space");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").expect("a key"),
        });
        assert_eq!(story.read_with(cx, |story, _| story.selected), 1);
    }
}
