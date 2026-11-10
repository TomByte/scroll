//! Edits the view's sort keys.

use gpui::{
    AppContext, Context, DismissEvent, ElementId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};
use scroll_core::id::PropertyId;
use scroll_core::record::{Property, SortDirection, SortKey};
use scroll_ui::{
    Button, ButtonSize, DragPreview, EmptyState, Icon, IconButton, IconName, IconSize, ListItem,
    SectionHeader, Theme,
};

use crate::sort_keys::{with_direction_flipped, with_key_added, with_key_moved, without_key};
use crate::values::type_icon;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortEditorEvent {
    Changed(Vec<SortKey>),
}

pub struct SortEditor {
    keys: Vec<SortKey>,
    properties: Vec<Property>,
    focus_handle: FocusHandle,
}

impl EventEmitter<SortEditorEvent> for SortEditor {}
impl EventEmitter<DismissEvent> for SortEditor {}

impl Focusable for SortEditor {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SortEditor {
    pub fn new(keys: Vec<SortKey>, properties: Vec<Property>, cx: &mut Context<Self>) -> Self {
        SortEditor {
            keys,
            properties,
            focus_handle: cx.focus_handle(),
        }
    }

    fn change(&mut self, keys: Vec<SortKey>, cx: &mut Context<Self>) {
        self.keys = keys;
        cx.emit(SortEditorEvent::Changed(self.keys.clone()));
        cx.notify();
    }

    fn property(&self, id: PropertyId) -> Option<&Property> {
        self.properties.iter().find(|property| property.id == id)
    }

    fn render_key(
        &self,
        index: usize,
        theme: &scroll_ui::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let key = self.keys[index];
        let name = self
            .property(key.property)
            .map(|property| property.name.clone())
            .unwrap_or_default();
        let direction = match key.direction {
            SortDirection::Ascending => "Ascending",
            SortDirection::Descending => "Descending",
        };
        let dragged = DraggedSortKey {
            index,
            name: name.clone().into(),
        };
        let drop_target = theme.colors.selected;
        div()
            .id(ElementId::Name(format!("sort-key-{index}").into()))
            .flex()
            .items_center()
            .gap(theme.spacing.sm)
            .px(theme.spacing.sm)
            .h(theme.size.row_height)
            .rounded(theme.radius.md)
            .drag_over::<DraggedSortKey>(move |style, _, _, _| style.bg(drop_target))
            .on_drop(cx.listener(move |editor, dragged: &DraggedSortKey, _, cx| {
                let keys = with_key_moved(&editor.keys, dragged.index, index);
                editor.change(keys, cx)
            }))
            .child(
                div()
                    .id(ElementId::Name(format!("sort-grip-{index}").into()))
                    .cursor_grab()
                    .child(Icon::new(IconName::Grip).size(IconSize::Small))
                    .on_drag(dragged, |dragged, _, _, cx| {
                        cx.new(|_| DragPreview::new(dragged.name.clone()))
                    }),
            )
            .child(div().flex_1().truncate().child(name))
            .child(
                Button::new(
                    ElementId::Name(format!("direction-{index}").into()),
                    direction,
                )
                .size(ButtonSize::Small)
                .on_click(cx.listener(move |editor, _, _, cx| {
                    let keys = with_direction_flipped(&editor.keys, index);
                    editor.change(keys, cx)
                })),
            )
            .child(
                IconButton::new(
                    ElementId::Name(format!("remove-sort-{index}").into()),
                    IconName::Close,
                )
                .size(ButtonSize::Small)
                .tooltip("Remove sort")
                .on_click(cx.listener(move |editor, _, _, cx| {
                    let keys = without_key(&editor.keys, index);
                    editor.change(keys, cx)
                })),
            )
    }
}

/// A sort key being dragged by its grip to a new place in the list.
#[derive(Clone)]
struct DraggedSortKey {
    index: usize,
    name: SharedString,
}

impl Render for SortEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let unsorted: Vec<Property> = self
            .properties
            .iter()
            .filter(|property| !self.keys.iter().any(|key| key.property == property.id))
            .cloned()
            .collect();
        let keys = (0..self.keys.len())
            .map(|index| self.render_key(index, &theme, cx).into_any_element())
            .collect::<Vec<_>>();
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .child(SectionHeader::new("Sort"))
            .when(self.keys.is_empty(), |panel| {
                panel.child(EmptyState::new("Rows keep their own order.").inline())
            })
            .children(keys)
            .when(!unsorted.is_empty(), |panel| {
                panel
                    .child(SectionHeader::new("Add a sort"))
                    .children(unsorted.into_iter().map(|property| {
                        let id = property.id;
                        ListItem::new(
                            ElementId::Name(format!("add-sort-{}", id.0).into()),
                            property.name.clone(),
                        )
                        .start_slot(
                            Icon::new(type_icon(property.property_type)).size(IconSize::Small),
                        )
                        .on_click(cx.listener(move |editor, _, _, cx| {
                            let keys = with_key_added(&editor.keys, id);
                            editor.change(keys, cx)
                        }))
                    }))
            })
    }
}
