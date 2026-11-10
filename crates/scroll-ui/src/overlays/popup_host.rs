use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, DismissEvent, Entity, FocusHandle, Focusable,
    IntoElement, ManagedView, ParentElement, Pixels, Point, Render, RenderOnce, SharedString,
    Styled, Subscription, Window, canvas, div, point, px,
};

use crate::controls::button::Button;
use crate::display::icon::IconName;
use crate::overlays::menu::{Menu, MenuItem};
use crate::overlays::popover::Popover;

#[derive(Clone, Debug, PartialEq)]
pub enum PopupAnchor {
    /// At a window position, e.g. where a right-click landed.
    Point(Point<Pixels>),
    /// Under the element holding the `PopupHost::trigger` of this name, e.g. a button that
    /// was pressed from the keyboard or a command that opens the button's popup.
    Below(SharedString),
}

type TriggerBounds = Rc<RefCell<HashMap<SharedString, Bounds<Pixels>>>>;

struct ActivePopup {
    view: AnyView,
    position: Point<Pixels>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    _dismiss_subscription: Subscription,
}

/// Holds at most one popup (a menu, picker or panel) for the view that renders it. It draws
/// the popup in a `Popover` at its anchor and focuses it, closes it on a click outside, on
/// Escape or when the popup emits `DismissEvent`, and then gives focus back to where it was.
/// A popup is a plain focusable view that knows nothing of where it is drawn. Render the host
/// as a child of its owner.
#[derive(Default)]
pub struct PopupHost {
    triggers: TriggerBounds,
    active: Option<ActivePopup>,
}

impl PopupHost {
    pub fn new() -> Self {
        PopupHost::default()
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// An invisible element that records where its parent is, so `PopupAnchor::Below(name)`
    /// can open under it. Place it inside the trigger, which must be relatively positioned.
    pub fn trigger(&self, name: impl Into<SharedString>) -> PopupTrigger {
        PopupTrigger {
            name: name.into(),
            triggers: self.triggers.clone(),
        }
    }

    /// Opens the popup `build` makes, at the window position `anchor` resolves to, replacing
    /// any open one.
    pub fn show<V: ManagedView>(
        &mut self,
        anchor: PopupAnchor,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<V> {
        let position = self.resolve(&anchor);
        let previous_focus = match self.active.take() {
            Some(active) => active.previous_focus,
            None => window.focused(cx),
        };
        let view = cx.new(|cx| build(window, cx));
        let focus = view.focus_handle(cx);
        let dismiss_subscription =
            cx.subscribe_in(&view, window, |host, _, _: &DismissEvent, window, cx| {
                host.close(window, cx)
            });
        window.focus(&focus);
        self.active = Some(ActivePopup {
            view: view.clone().into(),
            position,
            focus,
            previous_focus,
            _dismiss_subscription: dismiss_subscription,
        });
        cx.notify();
        view
    }

    /// Closes the popup. Focus goes back to where it was, unless the person has already moved
    /// it elsewhere (for example by clicking another field, which also dismisses the popup).
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active.take() else {
            return;
        };
        let is_focus_unclaimed =
            window.focused(cx).is_none() || active.focus.contains_focused(window, cx);
        if is_focus_unclaimed && let Some(previous) = active.previous_focus {
            window.focus(&previous);
        }
        cx.notify();
    }

    fn resolve(&self, anchor: &PopupAnchor) -> Point<Pixels> {
        match anchor {
            PopupAnchor::Point(position) => *position,
            PopupAnchor::Below(name) => match self.triggers.borrow().get(name) {
                Some(bounds) => bounds.bottom_left(),
                None => {
                    log::warn!(
                        "no popup trigger named {name:?} has been drawn; opening at the top left"
                    );
                    point(px(0.0), px(0.0))
                }
            },
        }
    }
}

impl Render for PopupHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let host = cx.entity().downgrade();
        div().children(self.active.as_ref().map(|active| {
            Popover::new(active.position)
                .on_dismiss(move |window, cx| {
                    if let Err(error) = host.update(cx, |host, cx| host.close(window, cx)) {
                        log::debug!("a popup was dismissed after its host closed: {error}");
                    }
                })
                .child(active.view.clone())
        }))
    }
}

/// See `PopupHost::trigger`.
#[derive(IntoElement)]
pub struct PopupTrigger {
    name: SharedString,
    triggers: TriggerBounds,
}

impl RenderOnce for PopupTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        canvas(
            move |bounds, _, _| {
                self.triggers.borrow_mut().insert(self.name, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

impl PopupHost {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let popups = cx.new(|_| PopupHost::new());
        cx.new(|_| PopupHostStory { popups }).into()
    }
}

struct PopupHostStory {
    popups: Entity<PopupHost>,
}

impl PopupHostStory {
    fn open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Below("page-menu".into()),
                |_, cx| {
                    Menu::new(cx)
                        .header("Page")
                        .item(
                            MenuItem::new("Rename", |_, _| {})
                                .icon(IconName::Text)
                                .shortcut("⌘R"),
                        )
                        .item(
                            MenuItem::new("Copy link", |_, _| {})
                                .icon(IconName::Link)
                                .shortcut("⌘L"),
                        )
                        .separator()
                        .item(MenuItem::new("Move to trash", |_, _| {}).icon(IconName::Trash))
                },
                window,
                cx,
            );
        });
    }
}

impl Render for PopupHostStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let trigger = self.popups.read(cx).trigger("page-menu");
        div()
            .flex()
            .child(
                div().relative().child(trigger).child(
                    Button::new("open-menu", "Open menu")
                        .icon(IconName::Ellipsis)
                        .on_click(cx.listener(|story, _, window, cx| story.open_menu(window, cx))),
                ),
            )
            .child(self.popups.clone())
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyUpEvent, Keystroke, TestAppContext};

    use super::*;
    use crate::foundation::story::testing::open_story;

    #[gpui::test]
    fn a_menu_opens_under_its_button_and_escape_gives_focus_back(cx: &mut TestAppContext) {
        let (story, cx) = open_story(PopupHost::story, cx);
        let popups = story
            .downcast::<PopupHostStory>()
            .expect("the popup host story")
            .read_with(cx, |story, _| story.popups.clone());
        cx.simulate_keystrokes("tab");
        let button = cx.update(|window, cx| window.focused(cx));
        cx.simulate_keystrokes("space");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").expect("a key"),
        });
        assert!(popups.read_with(cx, |popups, _| popups.is_open()));
        assert_ne!(cx.update(|window, cx| window.focused(cx)), button);

        cx.simulate_keystrokes("escape");
        assert!(!popups.read_with(cx, |popups, _| popups.is_open()));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), button);
    }

    #[gpui::test]
    fn a_click_outside_closes_the_popup(cx: &mut TestAppContext) {
        let (story, cx) = open_story(PopupHost::story, cx);
        let story = story
            .downcast::<PopupHostStory>()
            .expect("the popup host story");
        story.update_in(cx, |story, window, cx| story.open_menu(window, cx));
        let popups = story.read_with(cx, |story, _| story.popups.clone());
        assert!(popups.read_with(cx, |popups, _| popups.is_open()));

        cx.simulate_click(point(px(600.0), px(500.0)), gpui::Modifiers::none());
        assert!(!popups.read_with(cx, |popups, _| popups.is_open()));
    }
}
