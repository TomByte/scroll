//! The table's popups (pickers, editors, menus) open through the kit's `PopupHost`, which anchors
//! them, focuses them, closes them when they dismiss themselves and gives focus back.

use gpui::{
    AnyView, AnyWeakView, App, Context, Entity, ManagedView, Render, SharedString, Subscription,
    Window,
};
use scroll_core::id::PropertyId;
use scroll_ui::{PopupAnchor, PopupTrigger};

use super::TableView;

pub(super) const SORT_TRIGGER: &str = "sort";
pub(super) const FILTER_TRIGGER: &str = "filter";
pub(super) const PROPERTIES_TRIGGER: &str = "properties";

/// The popup last opened, and the subscription to the changes it reports.
pub(super) struct OpenPopup {
    view: AnyWeakView,
    _events: Option<Subscription>,
}

pub(super) fn column_trigger(property: PropertyId) -> SharedString {
    format!("column-{}", property.0).into()
}

impl TableView {
    /// Opens the popup `build` makes at `anchor`, replacing any open one. Focus returns to the
    /// table when it closes, unless a cell editor is open.
    pub(super) fn show_popup<V: ManagedView>(
        &mut self,
        anchor: PopupAnchor,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<V> {
        if self.text_edit.is_none() {
            window.focus(&self.focus_handle);
        }
        self.popups
            .update(cx, |popups, cx| popups.show(anchor, build, window, cx))
    }

    pub(super) fn remember_popup<V: Render>(
        &mut self,
        popup: &Entity<V>,
        events: Option<Subscription>,
    ) {
        self.popup = Some(OpenPopup {
            view: AnyView::from(popup.clone()).downgrade(),
            _events: events,
        });
    }

    pub(super) fn trigger(&self, name: impl Into<SharedString>, cx: &App) -> PopupTrigger {
        self.popups.read(cx).trigger(name)
    }

    pub(super) fn has_popup(&self, cx: &App) -> bool {
        self.popups.read(cx).is_open()
    }

    pub(super) fn is_popup_open<V: 'static>(&self, cx: &App) -> bool {
        self.has_popup(cx)
            && self
                .popup
                .as_ref()
                .and_then(|popup| popup.view.upgrade())
                .is_some_and(|view| view.downcast::<V>().is_ok())
    }

    pub(super) fn close_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popups
            .update(cx, |popups, cx| popups.close(window, cx));
        cx.notify();
    }
}
