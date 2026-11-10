//! Keyboard focus: Tab between controls, when focus rings show, and how they are drawn.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, FocusHandle, Global, Hsla, InteractiveElement, Keystroke,
    ParentElement, Pixels, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::ClickHandler;
use crate::foundation::actions::{FocusNext, FocusPrevious};
use crate::theme::Theme;

/// Whether focus last moved by keyboard. Controls draw their focus ring only then, as on
/// macOS, so clicking a button does not leave a ring behind.
#[derive(Default)]
struct KeyboardFocus {
    is_active: bool,
}

impl Global for KeyboardFocus {}

/// Lets Tab and Shift+Tab move focus between controls inside `root` (normally a window's
/// root element), and tracks whether focus rings should show.
pub fn tab_navigation<E: InteractiveElement>(root: E) -> E {
    root.on_action(|_: &FocusNext, window, cx| {
        cx.set_global(KeyboardFocus { is_active: true });
        window.focus_next();
    })
    .on_action(|_: &FocusPrevious, window, cx| {
        cx.set_global(KeyboardFocus { is_active: true });
        window.focus_prev();
    })
    .capture_any_mouse_down(|_, _, cx| {
        if is_keyboard_focus_visible(cx) {
            cx.set_global(KeyboardFocus { is_active: false });
        }
    })
}

/// Keeps Tab and Shift+Tab inside the element that tracks `container` (a modal): from its last
/// control focus wraps to its first, instead of reaching the window behind.
pub(crate) fn contained_tab_navigation<E: InteractiveElement>(
    element: E,
    container: FocusHandle,
) -> E {
    let backward_container = container.clone();
    element
        .on_action(move |_: &FocusNext, window, cx| {
            move_focus_within(&container, Window::focus_next, window, cx);
        })
        .on_action(move |_: &FocusPrevious, window, cx| {
            move_focus_within(&backward_container, Window::focus_prev, window, cx);
        })
}

/// Steps focus with `step` until it lands inside `container`, skipping the window's other
/// controls. If a whole round finds nothing inside, focus goes back to where it was.
fn move_focus_within(
    container: &FocusHandle,
    step: fn(&mut Window),
    window: &mut Window,
    cx: &mut App,
) {
    cx.set_global(KeyboardFocus { is_active: true });
    let start = window.focused(cx);
    let mut first_outside: Option<FocusHandle> = None;
    loop {
        step(window);
        if container.contains_focused(window, cx) {
            return;
        }
        let Some(focused) = window.focused(cx) else {
            return;
        };
        match &first_outside {
            None => first_outside = Some(focused),
            Some(first) if *first == focused => {
                if let Some(start) = &start {
                    window.focus(start);
                }
                return;
            }
            Some(_) => {}
        }
    }
}

pub(crate) fn is_keyboard_focus_visible(cx: &App) -> bool {
    cx.try_global::<KeyboardFocus>()
        .is_some_and(|focus| focus.is_active)
}

/// What a stateless control keeps across frames under its id.
#[derive(Clone)]
struct ControlState {
    focus_handle: FocusHandle,
    /// Set while Return or Space is held down on the control. GPUI clicks whatever has focus
    /// when either key is released, so without this a key that closed a popup would, on its way
    /// up, click the control focus went back to, and open the popup again.
    is_key_held: Rc<Cell<bool>>,
}

/// The control's state, and whether it should draw its focus ring (it has focus, and focus came
/// from the keyboard).
fn control_state(id: &ElementId, window: &mut Window, cx: &mut App) -> (ControlState, bool) {
    let state = window
        .use_keyed_state(id.clone(), cx, |_, cx| ControlState {
            focus_handle: cx.focus_handle().tab_stop(true),
            is_key_held: Rc::default(),
        })
        .read(cx)
        .clone();
    let shows_ring = is_keyboard_focus_visible(cx) && state.focus_handle.is_focused(window);
    (state, shows_ring)
}

fn is_click_key(keystroke: &Keystroke) -> bool {
    matches!(keystroke.key.as_str(), "enter" | "space") && !keystroke.modifiers.modified()
}

/// A ring drawn just outside a focused control, like a CSS outline: it does not change the
/// control's size or fill its background. The control must be relatively positioned.
pub(crate) fn focus_outline(theme: &Theme, control_radius: Pixels) -> Div {
    let width = theme.size.focus_ring_width;
    div()
        .absolute()
        .top(-width)
        .left(-width)
        .right(-width)
        .bottom(-width)
        .rounded(control_radius + width)
        .border(width)
        .border_color(theme.colors.focus_ring)
}

/// What makes an element a clickable control, applied in one call: a Tab stop whose focus
/// survives re-renders, the focus ring when focus came from the keyboard, the pointing hand,
/// the hover and pressed fills, and the click (which Space and Return also trigger). A
/// disabled control is only dimmed. Use it for anything clickable that no kit component
/// covers, such as a table header cell or a page's icon:
/// `Clickable::new(radius).hover(fill).on_click(…).apply(div().id(id)…, window, cx)`.
pub struct Clickable {
    radius: Pixels,
    hover: Option<Hsla>,
    pressed: Option<Hsla>,
    on_click: Option<ClickHandler>,
    is_disabled: bool,
}

impl FluentBuilder for Clickable {}

impl Clickable {
    /// `radius` is the control's corner radius, which its focus ring follows.
    pub fn new(radius: Pixels) -> Self {
        Clickable {
            radius,
            hover: None,
            pressed: None,
            on_click: None,
            is_disabled: false,
        }
    }

    pub fn hover(mut self, fill: Hsla) -> Self {
        self.hover = Some(fill);
        self
    }

    pub fn pressed(mut self, fill: Hsla) -> Self {
        self.pressed = Some(fill);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn disabled(mut self) -> Self {
        self.is_disabled = true;
        self
    }

    /// Makes `element`, which carries the control's id, behave as this control.
    pub fn apply(
        self,
        mut element: Stateful<Div>,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let theme = Theme::current(window, cx);
        if self.is_disabled {
            return element.opacity(theme.disabled_opacity);
        }
        let Some(id) = element.interactivity().element_id.clone() else {
            return element;
        };
        let (state, shows_ring) = control_state(&id, window, cx);
        let key_down = state.is_key_held.clone();
        let key_up = state.is_key_held;
        element
            .track_focus(&state.focus_handle)
            .on_key_down(move |event, _, _| {
                if is_click_key(&event.keystroke) {
                    key_down.set(true);
                }
            })
            .relative()
            .when(shows_ring, |element| {
                element.child(focus_outline(&theme, self.radius))
            })
            .cursor_pointer()
            .when_some(self.hover, |element, fill| {
                element.hover(move |style| style.bg(fill))
            })
            .when_some(self.pressed, |element, fill| {
                element.active(move |style| style.bg(fill))
            })
            .when_some(self.on_click, |element, handler| {
                element.on_click(move |event, window, cx| {
                    let was_key_held = key_up.replace(false);
                    if matches!(event, ClickEvent::Keyboard(_)) && !was_key_held {
                        return;
                    }
                    handler(event, window, cx)
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, IntoElement, KeyUpEvent, Render, TestAppContext};

    use super::*;

    struct Counter {
        focus_handle: FocusHandle,
        clicks: usize,
    }

    impl Render for Counter {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let control = div().id("control").size_8();
            let count = cx.listener(|counter, _: &ClickEvent, _, _| counter.clicks += 1);
            tab_navigation(div().track_focus(&self.focus_handle)).child(
                Clickable::new(Pixels::ZERO)
                    .on_click(count)
                    .apply(control, window, cx),
            )
        }
    }

    fn release(key: &str, cx: &mut gpui::VisualTestContext) {
        let keystroke = Keystroke::parse(key).expect("test keys are valid keystrokes");
        cx.simulate_event(KeyUpEvent { keystroke });
    }

    #[gpui::test]
    fn a_key_released_on_a_control_clicks_it_only_if_it_was_pressed_there(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (counter, cx) = cx.add_window_view(|_, cx| Counter {
            focus_handle: cx.focus_handle(),
            clicks: 0,
        });
        counter.update_in(cx, |counter, window, _| window.focus(&counter.focus_handle));
        cx.simulate_keystrokes("tab");
        assert!(
            !counter.update_in(cx, |counter, window, _| counter
                .focus_handle
                .is_focused(window)),
            "Tab moves focus to the control"
        );

        // As when Return closes a popup and focus comes back before the key is let go.
        release("enter", cx);
        assert_eq!(counter.read_with(cx, |counter, _| counter.clicks), 0);

        cx.simulate_keystrokes("enter");
        release("enter", cx);
        assert_eq!(counter.read_with(cx, |counter, _| counter.clicks), 1);
    }
}
