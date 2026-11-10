//! A database row's property values, shown and edited above the row's page content.

use scroll_core::operation::Transaction;
use std::collections::HashMap;

use gpui::{
    AnyElement, AppContext, Context, ElementId, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Pixels, Point, Render, Styled, Subscription, Window, div,
    prelude::FluentBuilder,
};
use scroll_core::id::{DatabaseId, PageId, PropertyId};
use scroll_core::record::{Property, PropertyType, PropertyValue};
use scroll_table::values::{
    chosen_options, display_text, select_options, select_picker, type_icon, value_from_text,
    value_of_chosen, with_new_option,
};
use scroll_ui::{
    Checkbox, Clickable, Combobox, ComboboxEvent, Ellipsis, Icon, IconSize, InputAccess, Pill,
    PopupAnchor, PopupHost, TextInput, TextInputEvent, Theme,
};

use crate::workspace::{Workspace, WorkspaceEvent};

/// The width of the column naming each property.
const LABEL_WIDTH: Pixels = gpui::px(164.0);

pub struct RowProperties {
    page: PageId,
    database: DatabaseId,
    workspace: Entity<Workspace>,
    /// Text fields for text, number, URL and date properties, kept so typing is not interrupted.
    inputs: HashMap<PropertyId, Entity<TextInput>>,
    popups: Entity<PopupHost>,
    options_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl RowProperties {
    pub fn new(
        page: PageId,
        database: DatabaseId,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe_in(&workspace, window, Self::workspace_changed);
        let closing = cx.on_release(|properties, cx| {
            let database = properties.database;
            properties
                .workspace
                .update(cx, |workspace, _| workspace.close_database(database));
        });
        workspace.update(cx, |workspace, _| workspace.open_database(database));
        RowProperties {
            page,
            database,
            workspace,
            inputs: HashMap::new(),
            popups: cx.new(|_| PopupHost::new()),
            options_subscription: None,
            _subscriptions: vec![subscription, closing],
        }
    }

    fn properties(&self, cx: &gpui::App) -> Vec<Property> {
        self.workspace
            .read(cx)
            .records()
            .database(self.database)
            .map(|database| database.properties.clone())
            .unwrap_or_default()
    }

    fn value(&self, property: PropertyId, cx: &gpui::App) -> Option<PropertyValue> {
        self.workspace
            .read(cx)
            .records()
            .value(self.page, property)
            .cloned()
    }

    fn set_value(
        &mut self,
        property: PropertyId,
        value: Option<PropertyValue>,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let page = self.page;
        self.workspace.update(cx, |workspace, cx| {
            workspace.commit(Transaction::set_value(page, property, value), cx)
        });
    }

    /// Values changed elsewhere (the table, another person) show here unless being typed in.
    fn workspace_changed(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, WorkspaceEvent::RecordsChanged(_)) {
            return;
        }
        let properties = self.properties(cx);
        for property in properties {
            let Some(input) = self.inputs.get(&property.id).cloned() else {
                continue;
            };
            let stored = display_value(self.value(property.id, cx).as_ref(), &property);
            let is_editing = input.read(cx).focus_handle(cx).is_focused(window);
            if !is_editing && input.read(cx).text() != stored {
                input.update(cx, |input, cx| input.set_text(stored, cx));
            }
        }
        cx.notify();
    }

    /// The text field for a typed property, made on first use.
    fn input_for(
        &mut self,
        property: &Property,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = self.inputs.get(&property.id) {
            return input.clone();
        }
        let text = display_value(self.value(property.id, cx).as_ref(), property);
        let placeholder = match property.property_type {
            PropertyType::Number => "Empty number",
            PropertyType::Url => "Empty link",
            PropertyType::Date => "Empty date",
            _ => "Empty",
        };
        let access = if self.workspace.read(cx).is_read_only() {
            InputAccess::ReadOnly
        } else {
            InputAccess::Editable
        };
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx)
                .plain()
                .placeholder(placeholder)
                .with_text(text);
            input.set_access(access, cx);
            input
        });
        let property_for_parse = property.clone();
        let subscription = cx.subscribe_in(&input, window, move |this, input, event, _, cx| {
            if *event != TextInputEvent::Changed {
                return;
            }
            let text = input.read(cx).text().to_string();
            // Text that is not a valid value yet (half a date, say) waits for more typing.
            if let Ok(value) = value_from_text(&text, &property_for_parse) {
                this.set_value(property_for_parse.id, value, cx);
            }
        });
        self._subscriptions.push(subscription);
        self.inputs.insert(property.id, input.clone());
        input
    }

    /// The property's options, to choose from, where the person clicked.
    fn open_options(
        &mut self,
        property: Property,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let value = self.value(property.id, cx);
        let picker = self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| select_picker(&property, value.as_ref(), cx),
                window,
                cx,
            )
        });
        self.listen_to_picker(property.id, &picker, cx);
    }

    fn listen_to_picker(
        &mut self,
        property: PropertyId,
        picker: &Entity<Combobox>,
        cx: &mut Context<Self>,
    ) {
        self.options_subscription = Some(cx.subscribe(
            picker,
            move |this, picker, event: &ComboboxEvent, cx| {
                this.options_changed(property, &picker, event, cx)
            },
        ));
    }

    /// Saves what was chosen, or adds the option asked for to the schema and chooses it.
    fn options_changed(
        &mut self,
        property: PropertyId,
        picker: &Entity<Combobox>,
        event: &ComboboxEvent,
        cx: &mut Context<Self>,
    ) {
        // Read afresh: an option created a moment ago is in the schema now.
        let Some(property) = self.property(property, cx) else {
            return;
        };
        match event {
            ComboboxEvent::Changed(chosen) => {
                self.set_value(property.id, value_of_chosen(&property, chosen), cx)
            }
            ComboboxEvent::CreateRequested(name) => {
                if self.workspace.read(cx).is_read_only() {
                    return;
                }
                let (updated, option) = with_new_option(&property, name);
                let database = self.database;
                let saved = Transaction::set_property(database, updated.clone());
                self.workspace
                    .update(cx, |workspace, cx| workspace.commit(saved, cx));
                let created = chosen_options(&updated, Some(&PropertyValue::Select(option)));
                picker.update(cx, |picker, cx| {
                    picker.set_options(select_options(&updated), cx);
                    for index in created {
                        picker.select_created(index, cx);
                    }
                });
            }
        }
    }

    fn property(&self, property: PropertyId, cx: &gpui::App) -> Option<Property> {
        self.properties(cx)
            .into_iter()
            .find(|candidate| candidate.id == property)
    }

    fn render_value(
        &mut self,
        property: &Property,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = self.value(property.id, cx);
        match property.property_type {
            PropertyType::Checkbox => {
                let is_checked = matches!(value, Some(PropertyValue::Checkbox(true)));
                let property_id = property.id;
                Checkbox::new(ElementId::Name(
                    format!("property-{}", property.id.0).into(),
                ))
                .when(is_checked, Checkbox::checked)
                .on_toggle(cx.listener(move |this, is_checked: &bool, _, cx| {
                    this.set_value(property_id, Some(PropertyValue::Checkbox(*is_checked)), cx)
                }))
                .into_any_element()
            }
            PropertyType::Select | PropertyType::MultiSelect => {
                let chosen = chosen_options(property, value.as_ref());
                let property = property.clone();
                let field = div()
                    .id(ElementId::Name(
                        format!("property-{}", property.id.0).into(),
                    ))
                    .flex()
                    .flex_wrap()
                    .gap(theme.spacing.xs)
                    .min_h(theme.size.row_height)
                    .items_center()
                    .px(theme.spacing.xs)
                    .rounded(theme.radius.sm)
                    .when(chosen.is_empty(), |field| {
                        field.child(
                            div()
                                .text_color(theme.colors.text_placeholder)
                                .child("Empty"),
                        )
                    })
                    .children(chosen.iter().map(|index| {
                        Pill::new(property.options[*index].name.clone()).swatch(*index)
                    }));
                Clickable::new(theme.radius.sm)
                    .hover(theme.colors.hover)
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            this.open_options(property.clone(), event.position(), window, cx)
                        }),
                    )
                    .apply(field, window, cx)
                    .into_any_element()
            }
            PropertyType::Text | PropertyType::Number | PropertyType::Url | PropertyType::Date => {
                let input = self.input_for(property, window, cx);
                div()
                    .flex_1()
                    .px(theme.spacing.xs)
                    .child(input)
                    .into_any_element()
            }
        }
    }
}

impl Render for RowProperties {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let properties = self.properties(cx);
        let rows: Vec<AnyElement> = properties
            .iter()
            .map(|property| {
                let value = self.render_value(property, &theme, window, cx);
                div()
                    .flex()
                    .items_center()
                    .min_h(theme.size.row_height)
                    .text_size(theme.text.ui.size)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme.spacing.sm)
                            .flex_none()
                            .w(LABEL_WIDTH)
                            .text_color(theme.colors.text_muted)
                            .child(
                                Icon::new(type_icon(property.property_type)).size(IconSize::Small),
                            )
                            .child(div().ellipsis().child(property.name.clone())),
                    )
                    .child(div().flex_1().min_w_0().child(value))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xxs)
            .pb(theme.spacing.lg)
            .children(rows)
            .child(self.popups.clone())
    }
}

/// How a typed value reads in its text field; the same as in the table.
fn display_value(value: Option<&PropertyValue>, property: &Property) -> String {
    value
        .map(|value| display_text(value, property))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::workspace::records::WorkspaceRecords;
    use crate::workspace::storage::Storage;

    #[gpui::test]
    fn an_option_created_here_joins_the_schema_and_becomes_the_value(cx: &mut TestAppContext) {
        cx.update(scroll_ui::init);
        let path = std::env::temp_dir().join(format!(
            "scroll-row-properties-{}.sqlite",
            PageId::generate().0
        ));
        let storage = Storage::open(&path).unwrap();
        let mut records = WorkspaceRecords::default();
        let (host, creating) = records.create_database(None).unwrap();
        records.apply(&creating);
        let database = records.database_hosted_by(host).unwrap().id;
        let status = records
            .database(database)
            .unwrap()
            .properties
            .iter()
            .find(|property| property.property_type == PropertyType::Select)
            .unwrap()
            .clone();
        let workspace = cx.new(|_| Workspace::new(path.clone(), storage, records));
        let row = PageId::generate();
        let panel_workspace = workspace.clone();
        let (panel, cx) = cx.add_window_view(|window, cx| {
            RowProperties::new(row, database, panel_workspace, window, cx)
        });
        let picker = cx.new(|cx| select_picker(&status, None, cx));
        panel.update(cx, |panel, cx| {
            panel.listen_to_picker(status.id, &picker, cx);
            panel.options_changed(
                status.id,
                &picker,
                &ComboboxEvent::CreateRequested("Blocked".into()),
                cx,
            );
        });
        cx.run_until_parked();

        workspace.read_with(cx, |workspace, _| {
            let records = workspace.records();
            let schema = &records.database(database).unwrap().properties;
            let updated = schema
                .iter()
                .find(|property| property.id == status.id)
                .unwrap();
            let blocked = updated
                .options
                .iter()
                .find(|option| option.name == "Blocked")
                .expect("the new option is in the schema");
            assert_eq!(
                records.value(row, status.id),
                Some(&PropertyValue::Select(blocked.id))
            );
        });
        workspace.update(cx, |workspace, _| workspace.storage().shut_down());
        std::fs::remove_file(path).ok();
    }
}
