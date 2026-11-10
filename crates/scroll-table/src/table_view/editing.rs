//! Editing cells in place: a text field for titles, text, URLs and numbers (and property names in
//! the header), a toggle for checkboxes, and popovers for select options and dates.

use gpui::{AppContext, Context, Entity, Focusable, Window};
use scroll_core::id::{PageId, PropertyId};
use scroll_core::operation::Transaction;
use scroll_core::record::{Date, Property, PropertyType, PropertyValue};
use scroll_ui::{Combobox, ComboboxEvent, PopupAnchor, TextInput, TextInputEvent, Theme};

use super::{EditTarget, TableView, TextEdit};
use crate::date_picker::{DatePicker, DatePickerEvent};
use crate::selection::CellPosition;
use crate::values::{
    chosen_options, select_options, select_picker, value_of_chosen, with_new_option,
};
use crate::values::{display_text, value_from_text};

/// The cell a picker popover is changing.
#[derive(Debug, Clone, Copy)]
struct PickerCell {
    page: PageId,
    property: PropertyId,
}

impl TableView {
    /// Opens the selected cell's editor. `typed` is a character the person started typing with.
    pub(super) fn start_editing(
        &mut self,
        typed: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.selection;
        let Some(row) = self.model.row(position.row) else {
            return;
        };
        let page = row.page.id;
        let Some(property) = self.property_at(position.column).cloned() else {
            let title = typed.unwrap_or_else(|| row.page.title.clone());
            self.open_text_editor(EditTarget::Title(page), title, window, cx);
            return;
        };
        let current = self.model.value(position.row, &property).cloned();
        match property.property_type {
            PropertyType::Text | PropertyType::Url | PropertyType::Number => {
                let text = typed.unwrap_or_else(|| {
                    current
                        .map(|value| display_text(&value, &property))
                        .unwrap_or_default()
                });
                let target = EditTarget::Value {
                    page,
                    property: property.id,
                    property_type: property.property_type,
                };
                self.open_text_editor(target, text, window, cx);
            }
            PropertyType::Checkbox => self.toggle_checkbox(page, property.id, cx),
            PropertyType::Select | PropertyType::MultiSelect => {
                self.open_select_picker(position, property, typed, window, cx)
            }
            PropertyType::Date => self.open_date_picker(position, typed, window, cx),
        }
    }

    /// Renames a property from its column header.
    pub(super) fn start_renaming(
        &mut self,
        property: PropertyId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .model
            .property(property)
            .map(|property| property.name.clone())
        else {
            return;
        };
        self.open_text_editor(EditTarget::PropertyName(property), name, window, cx);
    }

    fn open_text_editor(
        &mut self,
        target: EditTarget,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = Theme::current(window, cx);
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx).plain().text_size(theme.text.ui);
            input.set_text(text, cx);
            input
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |table, _, event: &TextInputEvent, window, cx| {
                // Escape arrives as the `Cancel` action, which the table also handles.
                if *event == TextInputEvent::Submitted {
                    table.commit_text_edit(window, cx);
                }
            },
        );
        window.focus(&input.focus_handle(cx));
        self.text_edit = Some(TextEdit {
            target,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Saves the edit and closes the editor. When the text does not fit the property the editor
    /// stays open with the reason shown, and this returns `false`.
    pub(super) fn commit_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_ref() else {
            return true;
        };
        let text = edit.input.read(cx).text().to_string();
        let target = edit.target;
        self.new_row = None;
        let transaction = match target {
            EditTarget::Title(page) => Some(Transaction::set_title(page, &text)),
            EditTarget::Value {
                page,
                property,
                property_type,
            } => match self.model.property(property) {
                Some(current) if current.property_type == property_type => {
                    match value_from_text(&text, current) {
                        Ok(value) => Some(Transaction::set_value(page, property, value)),
                        Err(error) => {
                            self.edit_error = Some(error.to_string().into());
                            cx.notify();
                            return false;
                        }
                    }
                }
                // Someone removed or retyped the property while this edit was open.
                _ => {
                    self.error = Some(
                        "The property changed while you were editing; the edit was not saved."
                            .into(),
                    );
                    None
                }
            },
            EditTarget::PropertyName(property) => self
                .model
                .property(property)
                .filter(|_| !text.trim().is_empty())
                .map(|property| {
                    let renamed = Property {
                        name: text.trim().to_string(),
                        ..property.clone()
                    };
                    Transaction::set_property(self.model.database().id, renamed)
                }),
        };
        if let Some(transaction) = transaction {
            self.write(transaction, cx);
        }
        self.text_edit = None;
        self.edit_error = None;
        window.focus(&self.focus_handle);
        cx.notify();
        true
    }

    /// Escape. Discards the edit, except a just-added row's title: the row already exists, so
    /// throwing away its title would leave an untitled row behind.
    pub(super) fn cancel_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_new_row_title = matches!(
            self.text_edit.as_ref().map(|edit| edit.target),
            Some(EditTarget::Title(page)) if self.new_row == Some(page)
        );
        if is_new_row_title {
            self.commit_text_edit(window, cx);
            return;
        }
        self.text_edit = None;
        self.edit_error = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    pub(super) fn toggle_checkbox(
        &mut self,
        page: PageId,
        property: PropertyId,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.model.row_index(page) else {
            return;
        };
        let is_checked = self
            .model
            .property(property)
            .and_then(|property| self.model.value(row, property))
            .is_some_and(|value| *value == PropertyValue::Checkbox(true));
        let transaction =
            Transaction::set_value(page, property, Some(PropertyValue::Checkbox(!is_checked)));
        self.write(transaction, cx);
        cx.notify();
    }

    fn open_select_picker(
        &mut self,
        position: CellPosition,
        property: Property,
        typed: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.model.row(position.row) else {
            return;
        };
        let cell = PickerCell {
            page: row.page.id,
            property: property.id,
        };
        let value = self.model.value(position.row, &property).cloned();
        let anchor = PopupAnchor::Point(self.cell_anchor(position));
        let picker = self.show_popup(
            anchor,
            |_, cx| select_picker(&property, value.as_ref(), cx),
            window,
            cx,
        );
        if let Some(typed) = typed {
            picker.update(cx, |picker, cx| picker.search_for(&typed, cx));
        }
        let events = cx.subscribe(&picker, move |table, picker, event, cx| match event {
            ComboboxEvent::Changed(chosen) => table.set_options(cell, chosen, cx),
            ComboboxEvent::CreateRequested(name) => table.create_option(cell, &picker, name, cx),
        });
        self.remember_popup(&picker, Some(events));
    }

    /// Sets the cell to the options at `chosen` (indexes into the property's options).
    fn set_options(&mut self, cell: PickerCell, chosen: &[usize], cx: &mut Context<Self>) {
        let Some(property) = self.model.property(cell.property) else {
            return;
        };
        let value = value_of_chosen(property, chosen);
        let transaction = Transaction::set_value(cell.page, cell.property, value);
        self.write(transaction, cx);
        cx.notify();
    }

    fn create_option(
        &mut self,
        cell: PickerCell,
        picker: &Entity<Combobox>,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(property) = self.model.property(cell.property) else {
            return;
        };
        let (updated, option) = with_new_option(property, name);
        self.write(
            Transaction::set_property(self.model.database().id, updated.clone()),
            cx,
        );
        let created = chosen_options(&updated, Some(&PropertyValue::Select(option)));
        picker.update(cx, |picker, cx| {
            picker.set_options(select_options(&updated), cx);
            for index in created {
                picker.select_created(index, cx);
            }
        });
        cx.notify();
    }

    fn open_date_picker(
        &mut self,
        position: CellPosition,
        typed: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.model.row(position.row) else {
            return;
        };
        let Some(property) = self.property_at(position.column) else {
            return;
        };
        let cell = PickerCell {
            page: row.page.id,
            property: property.id,
        };
        let selected = match self.model.value(position.row, property) {
            Some(PropertyValue::Date(date)) => Some(*date),
            _ => None,
        };
        let anchor = PopupAnchor::Point(self.cell_anchor(position));
        let picker = self.show_popup(anchor, |_, cx| DatePicker::new(selected, cx), window, cx);
        if let Some(typed) = typed {
            picker.update(cx, |picker, cx| picker.set_typed_text(&typed, cx));
        }
        let events = cx.subscribe_in(&picker, window, move |table, _, event, window, cx| {
            let DatePickerEvent::Picked(date) = event;
            table.set_date(cell, *date, window, cx)
        });
        self.remember_popup(&picker, Some(events));
    }

    fn set_date(
        &mut self,
        cell: PickerCell,
        date: Option<Date>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let transaction =
            Transaction::set_value(cell.page, cell.property, date.map(PropertyValue::Date));
        self.write(transaction, cx);
        self.close_popup(window, cx);
    }
}
