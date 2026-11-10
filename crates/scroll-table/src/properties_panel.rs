//! Shows and hides the view's columns, and adds properties.

use gpui::{
    App, Context, DismissEvent, ElementId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::FluentBuilder,
};
use scroll_core::id::PropertyId;
use scroll_core::record::{Property, PropertyType};
use scroll_ui::{Checkbox, Icon, IconSize, ListItem, SectionHeader};

use crate::TableAccess;
use crate::values::{ALL_TYPES, type_icon, type_label};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertiesPanelEvent {
    /// The visible columns, in order, after showing or hiding one.
    ShowColumns(Vec<PropertyId>),
    AddProperty(PropertyType),
}

pub struct PropertiesPanel {
    properties: Vec<Property>,
    visible: Vec<PropertyId>,
    focus_handle: FocusHandle,
    /// A viewer only shows and hides columns; adding properties is left out.
    access: TableAccess,
}

impl EventEmitter<PropertiesPanelEvent> for PropertiesPanel {}
impl EventEmitter<DismissEvent> for PropertiesPanel {}

impl Focusable for PropertiesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PropertiesPanel {
    pub fn new(
        properties: Vec<Property>,
        visible: Vec<PropertyId>,
        cx: &mut Context<Self>,
    ) -> Self {
        PropertiesPanel {
            properties,
            visible,
            focus_handle: cx.focus_handle(),
            access: TableAccess::Editable,
        }
    }

    pub fn for_access(self, access: TableAccess) -> Self {
        PropertiesPanel { access, ..self }
    }

    /// After the table changed the schema or the view, so the panel shows the result.
    pub fn update(
        &mut self,
        properties: Vec<Property>,
        visible: Vec<PropertyId>,
        cx: &mut Context<Self>,
    ) {
        self.properties = properties;
        self.visible = visible;
        cx.notify();
    }

    fn toggle(&mut self, property: PropertyId, cx: &mut Context<Self>) {
        let mut visible = self.visible.clone();
        match visible.iter().position(|id| *id == property) {
            Some(index) => {
                visible.remove(index);
            }
            None => visible.push(property),
        }
        cx.emit(PropertiesPanelEvent::ShowColumns(visible));
    }
}

impl Render for PropertiesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.properties.iter().map(|property| {
            let id = property.id;
            let is_visible = self.visible.contains(&id);
            ListItem::new(
                ElementId::Name(format!("property-{}", id.0).into()),
                property.name.clone(),
            )
            .start_slot(Icon::new(type_icon(property.property_type)).size(IconSize::Small))
            .end_slot(
                Checkbox::new(ElementId::Name(format!("show-{}", id.0).into()))
                    .when(is_visible, Checkbox::checked)
                    .on_toggle(cx.listener(move |panel, _: &bool, _, cx| panel.toggle(id, cx))),
            )
        });
        let new_property_items = ALL_TYPES.map(|property_type| {
            ListItem::new(
                ElementId::Name(format!("new-{}", type_label(property_type)).into()),
                type_label(property_type),
            )
            .start_slot(Icon::new(type_icon(property_type)).size(IconSize::Small))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(PropertiesPanelEvent::AddProperty(property_type))
            }))
        });
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .child(SectionHeader::new("Shown in this view"))
            .children(rows)
            .when(self.access == TableAccess::Editable, |panel| {
                panel
                    .child(SectionHeader::new("New property"))
                    .children(new_property_items)
            })
    }
}
