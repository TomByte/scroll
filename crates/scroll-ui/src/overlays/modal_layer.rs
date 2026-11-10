use gpui::{
    AnyView, AppContext, Context, DismissEvent, ElementId, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ManagedView, MouseButton, ParentElement, Render, Styled,
    Subscription, Window, deferred, div,
};

use crate::foundation::actions::{Cancel, MODAL_CONTEXT};
use crate::foundation::focus::contained_tab_navigation;
use crate::motion::{Motion, MotionExt};
use crate::theme::Theme;
use gpui::{App, EventEmitter, prelude::FluentBuilder};

use crate::controls::button::Button;
use crate::controls::text_input::TextInput;
use crate::display::divider::Divider;
use crate::display::icon::IconName;
use crate::lists::list_item::ListItem;
use crate::overlays::modal::Modal;

struct ActiveModal {
    view: AnyView,
    /// Tracked by the modal's container, so Tab can tell what is inside it.
    container: FocusHandle,
    previous_focus: Option<FocusHandle>,
    _dismiss_subscription: Subscription,
}

/// Hosts at most one modal (such as a command palette) above the window's content.
/// Place it as the last child of a full-window, relatively positioned root. The modal is
/// dismissed on Escape, on a click outside it, or when its view emits `DismissEvent`, and
/// focus returns to wherever it was before. Tab moves only between the modal's controls.
#[derive(Default)]
pub struct ModalLayer {
    active: Option<ActiveModal>,
}

impl ModalLayer {
    pub fn new() -> Self {
        ModalLayer::default()
    }

    /// Shows the view `build` makes, replacing any open modal, and returns it so the caller
    /// can subscribe to its events.
    pub fn show<V: ManagedView>(
        &mut self,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<V> {
        let previous_focus = window.focused(cx);
        let view = cx.new(|cx| build(window, cx));
        let dismiss_subscription =
            cx.subscribe_in(&view, window, |layer, _, _: &DismissEvent, window, cx| {
                layer.dismiss(window, cx);
            });
        window.focus(&view.focus_handle(cx));
        self.active = Some(ActiveModal {
            view: view.clone().into(),
            container: cx.focus_handle(),
            previous_focus,
            _dismiss_subscription: dismiss_subscription,
        });
        cx.notify();
        view
    }

    /// The open modal if it is a `V`, so a command can leave its modal alone when it is already up.
    pub fn showing<V: 'static>(&self) -> Option<Entity<V>> {
        self.active.as_ref()?.view.clone().downcast::<V>().ok()
    }

    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active.take() else {
            return;
        };
        if let Some(previous_focus) = active.previous_focus {
            window.focus(&previous_focus);
        }
        cx.notify();
    }
}

impl Render for ModalLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = self.active.as_ref() else {
            return div();
        };
        let theme = Theme::current(window, cx);
        // Deferred above popovers (priority 1) so an open menu never covers the modal.
        let backdrop = div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .pt(theme.size.modal_top_offset)
            .bg(theme.colors.backdrop)
            .motion("modal-backdrop", Motion::FadeIn, window, cx)
            .child(
                contained_tab_navigation(div(), active.container.clone())
                    .key_context(MODAL_CONTEXT)
                    .track_focus(&active.container)
                    // A click on the modal's blank space would otherwise move focus to the
                    // container, away from the field being typed in.
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_action(
                        cx.listener(|layer, _: &Cancel, window, cx| layer.dismiss(window, cx)),
                    )
                    .on_mouse_down_out(
                        cx.listener(|layer, _, window, cx| layer.dismiss(window, cx)),
                    )
                    .child(active.view.clone())
                    // Keyed by the modal, so one replacing another rises in afresh.
                    .motion(
                        ElementId::View(active.view.entity_id()),
                        Motion::RiseIn,
                        window,
                        cx,
                    ),
            );
        div()
            .absolute()
            .inset_0()
            .child(deferred(backdrop).with_priority(2))
    }
}

impl ModalLayer {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let layer = cx.new(|_| ModalLayer::new());
        cx.new(|_| ModalLayerStory { layer }).into()
    }
}

/// The layer covers the story's own area here; in the app it covers the window.
struct ModalLayerStory {
    layer: Entity<ModalLayer>,
}

impl Render for ModalLayerStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let layer = self.layer.clone();
        div()
            .relative()
            .flex()
            .items_start()
            .h(theme.size.modal_width / 2.0)
            .child(
                Button::new("open-palette", "Command palette")
                    .icon(IconName::Search)
                    .on_click(move |_, window, cx| {
                        layer.update(cx, |layer, cx| {
                            layer.show(DemoPalette::new, window, cx);
                        });
                    }),
            )
            .child(self.layer.clone())
    }
}

const DEMO_COMMANDS: [(&str, IconName); 4] = [
    ("New page", IconName::Plus),
    ("Search pages", IconName::Search),
    ("Copy link to page", IconName::Link),
    ("Move page to trash", IconName::Trash),
];

/// A command palette built from kit pieces: a modal, a text input and list items.
struct DemoPalette {
    input: Entity<TextInput>,
}

impl EventEmitter<DismissEvent> for DemoPalette {}

impl Focusable for DemoPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl DemoPalette {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        DemoPalette {
            input: cx.new(|cx| TextInput::new(cx).placeholder("Type a command…")),
        }
    }
}

impl Render for DemoPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let query = self.input.read(cx).text().to_lowercase();
        let rows = DEMO_COMMANDS
            .into_iter()
            .filter(|(label, _)| label.to_lowercase().contains(&query))
            .enumerate()
            .map(|(index, (label, icon))| {
                ListItem::new(("command", index), label)
                    .icon(icon)
                    .emphasized()
                    .when(index == 0, ListItem::highlighted)
            });
        Modal::new()
            .child(div().p(theme.spacing.md).child(self.input.clone()))
            .child(Divider::horizontal())
            .child(div().flex().flex_col().p(theme.spacing.xs).children(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(ModalLayer::story, cx);
    }
}
