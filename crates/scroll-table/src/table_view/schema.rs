//! Columns and properties (show, hide, add, rename, retype, delete, resize, reorder), and the
//! view's sort and filter. Changes that alter which rows show, or their order, re-run the query.

use gpui::{App, Context, WeakEntity, Window};
use scroll_core::id::PropertyId;
use scroll_core::record::{FilterCondition, Property, PropertyType, SortDirection, SortKey};
use scroll_ui::{IconName, Menu, MenuItem, PopupAnchor};

use super::popups::{FILTER_TRIGGER, PROPERTIES_TRIGGER, SORT_TRIGGER, column_trigger};
use super::{Command, TableAccess, TableView};
use crate::filter_editor::{FilterEditor, FilterEditorEvent};
use crate::properties_panel::{PropertiesPanel, PropertiesPanelEvent};
use crate::sort_editor::{SortEditor, SortEditorEvent};
use crate::sort_keys::only_by;
use crate::transactions::{
    add_property, change_property_type, delete_property, new_property, set_filter, set_sort,
    set_visible_columns,
};
use crate::values::{ALL_TYPES, type_label};

impl TableView {
    /// Opens the sort editor under the toolbar's Sort button.
    pub fn open_sort_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys = self.model.view().sort.clone();
        let properties = self.model.database().properties.clone();
        let editor = self.show_popup(
            PopupAnchor::Below(SORT_TRIGGER.into()),
            |_, cx| SortEditor::new(keys, properties, cx),
            window,
            cx,
        );
        let events = cx.subscribe(&editor, |table, _, event, cx| {
            let SortEditorEvent::Changed(keys) = event;
            table.change_sort(keys.clone(), cx)
        });
        self.remember_popup(&editor, Some(events));
    }

    /// Opens the filter editor under the toolbar's Filter button.
    pub fn open_filter_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.model.view().filter.clone();
        let properties = self.model.database().properties.clone();
        let editor = self.show_popup(
            PopupAnchor::Below(FILTER_TRIGGER.into()),
            |_, cx| FilterEditor::new(&filter, properties, cx),
            window,
            cx,
        );
        let events = cx.subscribe(&editor, |table, _, event, cx| {
            let FilterEditorEvent::Changed(conditions) = event;
            table.change_filter(conditions.clone(), cx)
        });
        self.remember_popup(&editor, Some(events));
    }

    /// Opens the properties panel under the toolbar's Properties button.
    pub fn open_properties_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let properties = self.model.database().properties.clone();
        let visible = self.columns.order();
        let access = self.access;
        let panel = self.show_popup(
            PopupAnchor::Below(PROPERTIES_TRIGGER.into()),
            |_, cx| PropertiesPanel::new(properties, visible, cx).for_access(access),
            window,
            cx,
        );
        let events = cx.subscribe_in(&panel, window, |table, panel, event, window, cx| {
            match event {
                PropertiesPanelEvent::ShowColumns(columns) => {
                    table.show_columns(columns.clone(), cx)
                }
                PropertiesPanelEvent::AddProperty(property_type) => {
                    table.perform(Command::AddProperty(*property_type), window, cx)
                }
            }
            let properties = table.model.database().properties.clone();
            let visible = table.columns.order();
            panel.update(cx, |panel, cx| panel.update(properties, visible, cx));
        });
        self.remember_popup(&panel, Some(events));
    }

    /// Opens a column's menu (rename, sort, hide, type, delete) under its header.
    pub fn open_column_menu(
        &mut self,
        property: PropertyId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.model.property(property).cloned() else {
            return;
        };
        let table = cx.entity().downgrade();
        let access = self.access;
        let menu = self.show_popup(
            PopupAnchor::Below(column_trigger(property)),
            |_, cx| {
                let menu = column_menu(&table, &current, cx);
                match access {
                    TableAccess::Editable => with_schema_items(menu, &table, &current),
                    TableAccess::ReadOnly => menu,
                }
            },
            window,
            cx,
        );
        self.remember_popup(&menu, None);
    }

    /// Saves the view's sort, as the sort editor does, and re-runs the query in the background.
    pub fn change_sort(&mut self, keys: Vec<SortKey>, cx: &mut Context<Self>) {
        self.write(set_sort(self.model.view(), keys), cx);
        self.load(cx);
        cx.notify();
    }

    /// Saves the view's filter, as the filter editor does, and re-runs the query in the background.
    pub fn change_filter(&mut self, conditions: Vec<FilterCondition>, cx: &mut Context<Self>) {
        self.write(set_filter(self.model.view(), conditions), cx);
        self.load(cx);
        cx.notify();
    }

    fn show_columns(&mut self, columns: Vec<PropertyId>, cx: &mut Context<Self>) {
        self.write(set_visible_columns(self.model.view(), columns), cx);
        cx.notify();
    }

    pub(super) fn hide_column(&mut self, property: PropertyId, cx: &mut Context<Self>) {
        let columns = self
            .columns
            .order()
            .into_iter()
            .filter(|column| *column != property)
            .collect();
        self.show_columns(columns, cx);
    }

    /// Adds a property named after its type, as the last column; rename it from its header.
    pub(super) fn add_property(&mut self, property_type: PropertyType, cx: &mut Context<Self>) {
        let property = new_property(type_label(property_type), property_type);
        self.write(add_property(self.model.view(), property), cx);
        self.horizontal_offset = self.columns.max_offset(self.scrolling_width);
        cx.notify();
    }

    pub(super) fn change_type(
        &mut self,
        property: PropertyId,
        property_type: PropertyType,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.model.property(property).cloned() else {
            return;
        };
        let had_filter = self
            .model
            .view()
            .filter
            .iter()
            .any(|condition| condition.property == property);
        let transaction = change_property_type(self.model.view(), &current, property_type);
        self.write(transaction, cx);
        if had_filter {
            self.load(cx);
        }
    }

    pub(super) fn delete_property(&mut self, property: PropertyId, cx: &mut Context<Self>) {
        self.write(delete_property(self.model.view(), property), cx);
        self.load(cx);
        cx.notify();
    }

    fn sort_by_column(
        &mut self,
        property: PropertyId,
        direction: SortDirection,
        cx: &mut Context<Self>,
    ) {
        self.change_sort(only_by(property, direction), cx);
    }

    /// Drops `property`'s column where `target`'s is, and saves the new order to the view.
    pub(super) fn move_column(
        &mut self,
        property: PropertyId,
        target: PropertyId,
        cx: &mut Context<Self>,
    ) {
        let order = self.columns.order_after_move(property, target);
        self.show_columns(order, cx);
    }
}

/// What anyone may do with a column: sort by it and hide it.
fn column_menu(table: &WeakEntity<TableView>, property: &Property, cx: &mut Context<Menu>) -> Menu {
    let id = property.id;
    Menu::new(cx)
        .item(
            MenuItem::new(
                "Sort ascending",
                on_table(table, move |table, _, cx| {
                    table.sort_by_column(id, SortDirection::Ascending, cx)
                }),
            )
            .icon(IconName::ArrowRight),
        )
        .item(
            MenuItem::new(
                "Sort descending",
                on_table(table, move |table, _, cx| {
                    table.sort_by_column(id, SortDirection::Descending, cx)
                }),
            )
            .icon(IconName::ArrowLeft),
        )
        .item(
            MenuItem::new(
                "Hide",
                on_table(table, move |table, _, cx| table.hide_column(id, cx)),
            )
            .icon(IconName::Minus),
        )
}

/// What only an editor may do with a column: rename it, change its type, delete it.
fn with_schema_items(menu: Menu, table: &WeakEntity<TableView>, property: &Property) -> Menu {
    let id = property.id;
    let mut menu = menu
        .item(
            MenuItem::new(
                "Rename",
                on_table(table, move |table, window, cx| {
                    table.perform(Command::RenameProperty(id), window, cx)
                }),
            )
            .icon(IconName::Pencil),
        )
        .separator()
        .header("Type");
    for property_type in ALL_TYPES {
        let item = MenuItem::new(
            type_label(property_type),
            on_table(table, move |table, window, cx| {
                let command = Command::ChangePropertyType {
                    property: id,
                    property_type,
                };
                table.perform(command, window, cx)
            }),
        );
        menu = menu.item(match property_type == property.property_type {
            true => item.checked(),
            false => item,
        });
    }
    menu.separator().item(
        MenuItem::new(
            "Delete property",
            on_table(table, move |table, window, cx| {
                table.perform(Command::DeleteProperty(id), window, cx)
            }),
        )
        .icon(IconName::Trash),
    )
}

/// A menu callback that runs `action` on the table, if the table is still open.
fn on_table(
    table: &WeakEntity<TableView>,
    action: impl Fn(&mut TableView, &mut Window, &mut Context<TableView>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let table = table.clone();
    move |window, cx| {
        if let Some(table) = table.upgrade() {
            table.update(cx, |table, cx| action(table, window, cx));
        }
    }
}
