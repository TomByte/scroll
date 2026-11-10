use gpui::{
    AnyView, App, AppContext, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    Render, RenderOnce, Styled, Window, div, prelude::FluentBuilder,
};

use crate::foundation::focus::Clickable;
use crate::motion::{Transition, mix};
use crate::theme::Theme;

type ToggleHandler = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// An on/off switch: a track with a knob, as on macOS. It has no label; put one beside it. The
/// caller owns the state and receives the requested new state in `on_toggle`.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    is_checked: bool,
    is_disabled: bool,
    on_toggle: Option<ToggleHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Switch {
            id: id.into(),
            is_checked: false,
            is_disabled: false,
            on_toggle: None,
        }
    }

    pub fn checked(mut self) -> Self {
        self.is_checked = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.is_disabled = true;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let colors = &theme.colors;
        let size = &theme.size;
        let inset = theme.spacing.xxs;
        let knob_size = size.switch_height - inset * 2.0;
        let on_amount =
            Transition::Toggle.fraction_on(self.id.clone(), self.is_checked, window, cx);
        let knob_travel = size.switch_width - knob_size - inset * 2.0;
        let knob_left = inset + knob_travel * on_amount;
        let track = mix(colors.border_strong, colors.accent, on_amount);

        let knob = div()
            .absolute()
            .top(inset)
            .left(knob_left)
            .size(knob_size)
            .rounded_full()
            .bg(colors.text_on_accent)
            .shadow(theme.shadow.raised.clone());

        let is_checked = self.is_checked;
        div()
            .id(self.id)
            .flex_none()
            .w(size.switch_width)
            .h(size.switch_height)
            .rounded_full()
            .bg(track)
            .child(knob)
            .map(|switch| {
                Clickable::new(size.switch_height / 2.0)
                    .when(self.is_disabled, Clickable::disabled)
                    .when_some(self.on_toggle, |clickable, handler| {
                        clickable.on_click(move |_, window, cx| handler(&!is_checked, window, cx))
                    })
                    .apply(switch, window, cx)
            })
    }
}

impl Switch {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| SwitchStory {
            checked: [false, true],
        })
        .into()
    }
}

struct SwitchStory {
    checked: [bool; 2],
}

impl Render for SwitchStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let labels = ["Sync on cellular", "Reduce motion"];
        let row = |label: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .w(theme.size.popover_min_width)
                .text_size(theme.text.ui.size)
                .line_height(theme.text.ui.line_height)
                .text_color(theme.colors.text)
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.md)
            .children(labels.into_iter().enumerate().map(|(index, label)| {
                row(label).child(
                    Switch::new(("switch", index))
                        .when(self.checked[index], Switch::checked)
                        .on_toggle(cx.listener(move |story, is_checked: &bool, _, cx| {
                            story.checked[index] = *is_checked;
                            cx.notify();
                        })),
                )
            }))
            .child(row("Disabled").child(Switch::new("switch-disabled").checked().disabled()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn space_asks_for_the_opposite_state(cx: &mut gpui::TestAppContext) {
        use gpui::{KeyUpEvent, Keystroke};
        let (story, cx) = crate::foundation::story::testing::open_story(Switch::story, cx);
        let story = story.downcast::<SwitchStory>().expect("the switch story");
        cx.simulate_keystrokes("tab space");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").expect("a key"),
        });
        assert_eq!(story.read_with(cx, |story, _| story.checked), [true, true]);
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Switch::story, cx);
    }
}
