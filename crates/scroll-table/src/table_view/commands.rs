//! Everything the person can ask the table to do, from keys, buttons, menus or the app, and the
//! one check that a read-only table refuses the commands that edit.

use gpui::{Context, Window};
use scroll_core::id::{PageId, PropertyId};
use scroll_core::record::PropertyType;

use super::TableView;
use crate::selection::Movement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Move(Movement),
    Extend(Movement),
    StartEditing,
    /// The person started typing on the selected cell; the text is what they typed.
    TypeToEdit(String),
    CopyCells,
    PasteCells,
    ClearCells,
    AddRow,
    DeleteRows,
    /// The row checkbox of the selected cell's row.
    ToggleSelectedRow,
    ToggleRow(PageId),
    ToggleCheckbox {
        page: PageId,
        property: PropertyId,
    },
    RenameProperty(PropertyId),
    AddProperty(PropertyType),
    ChangePropertyType {
        property: PropertyId,
        property_type: PropertyType,
    },
    DeleteProperty(PropertyId),
    OpenRow,
    Cancel,
}

impl Command {
    fn needs_edit_access(&self) -> bool {
        match self {
            Command::Move(_)
            | Command::Extend(_)
            | Command::CopyCells
            | Command::OpenRow
            | Command::Cancel => false,
            Command::StartEditing
            | Command::TypeToEdit(_)
            | Command::PasteCells
            | Command::ClearCells
            | Command::AddRow
            | Command::DeleteRows
            | Command::ToggleSelectedRow
            | Command::ToggleRow(_)
            | Command::ToggleCheckbox { .. }
            | Command::RenameProperty(_)
            | Command::AddProperty(_)
            | Command::ChangePropertyType { .. }
            | Command::DeleteProperty(_) => true,
        }
    }
}

impl TableView {
    /// Runs the command, unless it edits and the table is read-only. Writes are refused again
    /// in `write`, which keeps only a viewer's session changes to the view.
    pub(crate) fn perform(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if command.needs_edit_access() && !self.can_edit() {
            return;
        }
        match command {
            Command::Move(movement) => self.move_selection(movement, window, cx),
            Command::Extend(movement) => self.extend_selection(movement, cx),
            Command::StartEditing => self.start_editing(None, window, cx),
            Command::TypeToEdit(typed) => {
                cx.stop_propagation();
                self.start_editing(Some(typed), window, cx);
            }
            Command::CopyCells => self.copy_cells(cx),
            Command::PasteCells => self.paste_cells(cx),
            Command::ClearCells => self.clear_cells(cx),
            Command::AddRow => self.add_row_and_edit_title(window, cx),
            Command::DeleteRows => self.delete_rows(cx),
            Command::ToggleSelectedRow => {
                if let Some(page) = self.model.row_id(self.selection.row) {
                    self.toggle_row(page, cx);
                }
            }
            Command::ToggleRow(page) => self.toggle_row(page, cx),
            Command::ToggleCheckbox { page, property } => self.toggle_checkbox(page, property, cx),
            Command::RenameProperty(property) => self.start_renaming(property, window, cx),
            Command::AddProperty(property_type) => self.add_property(property_type, cx),
            Command::ChangePropertyType {
                property,
                property_type,
            } => self.change_type(property, property_type, cx),
            Command::DeleteProperty(property) => self.delete_property(property, cx),
            Command::OpenRow => self.open_row(cx),
            Command::Cancel => self.cancel(window, cx),
        }
    }
}
