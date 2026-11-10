//! The table entity. Its behaviour is split by concern: `commands` (what the person can ask for,
//! and the read-only check), `navigation` (selection, clipboard, rows), `editing` (cell editors),
//! `schema` (columns, properties, sort and filter), `popups`, `storage` (background reads and
//! writes), and the drawing in `render`, `header` and `row`.

mod commands;
mod editing;
mod header;
mod navigation;
mod pointer;
mod popups;
mod render;
mod row;
mod schema;
mod storage;

pub(crate) use commands::Command;

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, Point,
    SharedString, Subscription, UniformListScrollHandle, Window, point, px,
};
use scroll_core::id::{PageId, PropertyId, ViewId};
use scroll_core::record::{Property, PropertyType, View};
use scroll_ui::{Combobox, PopupHost, TextInput};

use crate::columns::ColumnLayout;
use crate::date_picker::DatePicker;
use crate::selection::{CellPosition, CellRange, GridSize};
use crate::sizes::{GUTTER_WIDTH, ROW_HEIGHT, TITLE_COLUMN_WIDTH};
use crate::table_model::TableModel;

/// What the person may change. A viewer can still sort, filter, resize and reorder columns, but
/// only for the session: nothing is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableAccess {
    #[default]
    Editable,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableEvent {
    /// Open the row as a page.
    OpenPage(PageId),
    /// One of the table's changes was written to the store. Other views of the same records
    /// (another table, the sidebar) should refresh.
    Changed,
}

/// What an in-place text editor is changing. Rows are named by page, so a reload while editing
/// cannot redirect the edit to another row. A value edit remembers the property's type, so an
/// edit is not saved into a property whose type changed meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Title(PageId),
    Value {
        page: PageId,
        property: PropertyId,
        property_type: PropertyType,
    },
    PropertyName(PropertyId),
}

impl EditTarget {
    fn is_value_of(&self, page: PageId, property: PropertyId) -> bool {
        matches!(self, EditTarget::Value { page: edited, property: edited_property, .. }
            if *edited == page && *edited_property == property)
    }
}

struct TextEdit {
    target: EditTarget,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// A drag that follows the pointer anywhere in the window until the button is released.
enum PointerDrag {
    ResizeColumn {
        property: PropertyId,
        start_x: Pixels,
        start_width: Pixels,
    },
}

pub struct TableView {
    storage: storage::TableStorage,
    background: storage::BackgroundWork,
    model: TableModel,
    columns: ColumnLayout,
    focus_handle: FocusHandle,
    scroll: UniformListScrollHandle,
    /// How far the property columns are scrolled left; the title column stays pinned.
    horizontal_offset: Pixels,
    /// Width of the area the property columns scroll in, measured at the last render.
    scrolling_width: Pixels,
    /// The cell the keyboard moves; with `anchor`, the selected block of cells.
    selection: CellPosition,
    anchor: CellPosition,
    /// Off after a click on the table's empty space, until a cell is next selected.
    is_selection_shown: bool,
    /// Whether the pointer is dragging out a block of cells.
    is_drag_selecting: bool,
    selected_rows: HashSet<PageId>,
    text_edit: Option<TextEdit>,
    /// Why the text being edited cannot be saved, shown under the cell.
    edit_error: Option<SharedString>,
    popups: Entity<PopupHost>,
    popup: Option<popups::OpenPopup>,
    pointer_drag: Option<PointerDrag>,
    error: Option<SharedString>,
    /// True until the first load arrives, and while a newer one is running.
    is_loading: bool,
    /// The rows the list showed at the last frame.
    visible_rows: Range<usize>,
    access: TableAccess,
    /// The row just added, while its title is first being typed.
    new_row: Option<PageId>,
    /// While read-only, the view as the person changed it this session; it is never written.
    session_view: Option<View>,
    /// The table should take keyboard focus at the next render (its editor went away).
    wants_focus: bool,
}

impl EventEmitter<TableEvent> for TableView {}

impl Focusable for TableView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TableView {
    /// The table for `view` in the workspace file at `path`. The table opens its own connections
    /// to the file and uses them only on background threads, so it never runs SQLite on the UI
    /// thread: it shows the header at once, then rows when the first load finishes, and writes
    /// each change in order after showing it. The file must already be open as a workspace (its
    /// schema migrated), as it is once the app's own `WorkspaceStore::open` has run.
    pub fn new(path: PathBuf, view: ViewId, cx: &mut Context<Self>) -> Self {
        let mut table = TableView {
            storage: storage::TableStorage::new(path),
            background: storage::BackgroundWork::default(),
            model: TableModel::placeholder(view),
            columns: ColumnLayout::default(),
            focus_handle: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            horizontal_offset: px(0.0),
            scrolling_width: px(0.0),
            selection: CellPosition::default(),
            anchor: CellPosition::default(),
            is_selection_shown: true,
            is_drag_selecting: false,
            selected_rows: HashSet::new(),
            text_edit: None,
            edit_error: None,
            popups: cx.new(|_| PopupHost::new()),
            popup: None,
            pointer_drag: None,
            error: None,
            is_loading: true,
            visible_rows: 0..0,
            access: TableAccess::Editable,
            new_row: None,
            session_view: None,
            wants_focus: false,
        };
        table.load(cx);
        table
    }

    /// Re-reads the view, schema and rows in the background, for when changes made elsewhere
    /// (other people, another view) arrive. Rows move into their filtered and sorted places when
    /// the read finishes; an edit in progress continues, and is not saved if its property is gone
    /// or changed type by then.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    /// Changes what the person may do, for when their role changes while the table is open.
    /// Becoming read-only ends any edit; becoming editable drops session-only view changes and
    /// shows the stored view again.
    pub fn set_access(&mut self, access: TableAccess, window: &mut Window, cx: &mut Context<Self>) {
        if access == self.access {
            return;
        }
        self.access = access;
        match access {
            TableAccess::ReadOnly => {
                self.text_edit = None;
                self.edit_error = None;
                self.selected_rows.clear();
                self.close_popup(window, cx);
                window.focus(&self.focus_handle);
            }
            TableAccess::Editable => {
                self.session_view = None;
                self.load(cx);
            }
        }
        cx.notify();
    }

    pub fn access(&self) -> TableAccess {
        self.access
    }

    fn can_edit(&self) -> bool {
        self.access == TableAccess::Editable
    }

    /// Whether rows are still being loaded or re-queried.
    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    /// Scrolls the row into view and selects its title cell.
    pub fn reveal_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let position = CellPosition { row, column: 0 }.clamped(self.grid_size());
        self.select(position, cx);
    }

    /// Selects a cell and opens its editor; column 0 is the title, then the visible properties.
    pub fn edit_cell(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = CellPosition { row, column }.clamped(self.grid_size());
        self.select(position, cx);
        self.perform(Command::StartEditing, window, cx);
    }

    pub fn row_count(&self) -> usize {
        self.model.row_count()
    }

    fn show_error(&mut self, error: &dyn std::error::Error) {
        log::error!("table: {error}");
        self.error = Some(error.to_string().into());
    }

    /// Keeps column widths, selection and scroll sensible after the schema or rows changed.
    fn after_schema_change(&mut self) {
        self.columns = self
            .columns
            .arranged(&visible_column_ids(&self.model))
            .with_saved_widths(&self.model.view().column_widths);
        self.selection = self.selection.clamped(self.grid_size());
        self.anchor = self.anchor.clamped(self.grid_size());
        self.selected_rows = self.model.rows_among(&self.selected_rows);
        self.end_orphaned_edit();
        self.clamp_horizontal_offset();
    }

    /// Ends an edit whose row or property is no longer shown, as when someone else deleted it.
    fn end_orphaned_edit(&mut self) {
        let Some(edit) = &self.text_edit else {
            return;
        };
        let is_shown = |property| self.columns.order().contains(&property);
        let still_there = match edit.target {
            EditTarget::Title(page) => self.model.row_index(page).is_some(),
            EditTarget::Value { page, property, .. } => {
                self.model.row_index(page).is_some() && is_shown(property)
            }
            EditTarget::PropertyName(property) => is_shown(property),
        };
        if still_there {
            return;
        }
        self.text_edit = None;
        self.edit_error = None;
        self.error =
            Some("What you were editing was changed elsewhere; the edit was not saved.".into());
        self.wants_focus = true;
    }

    /// Called at layout with the width the rows have; scroll limits depend on it.
    fn set_body_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let scrolling_width = (width - GUTTER_WIDTH - TITLE_COLUMN_WIDTH).max(px(0.0));
        if scrolling_width == self.scrolling_width {
            return;
        }
        self.scrolling_width = scrolling_width;
        self.clamp_horizontal_offset();
        cx.notify();
    }

    fn selected_range(&self) -> CellRange {
        CellRange {
            anchor: self.anchor,
            head: self.selection,
        }
    }

    fn is_picking_a_value(&self, cx: &App) -> bool {
        self.is_popup_open::<Combobox>(cx) || self.is_popup_open::<DatePicker>(cx)
    }

    fn grid_size(&self) -> GridSize {
        GridSize {
            rows: self.model.row_count(),
            columns: self.columns.columns().len() + 1,
        }
    }

    /// The property shown in grid column `column`; `None` for the title column.
    fn property_at(&self, column: usize) -> Option<&Property> {
        let column = self.columns.columns().get(column.checked_sub(1)?)?;
        self.model.property(column.property)
    }

    fn clamp_horizontal_offset(&mut self) {
        let max = self.columns.max_offset(self.scrolling_width);
        self.horizontal_offset = self.horizontal_offset.clamp(px(0.0), max);
    }

    /// The window position of a cell's bottom-left corner, for anchoring its editor popover.
    fn cell_anchor(&self, position: CellPosition) -> Point<Pixels> {
        let list = self.scroll.0.borrow();
        let bounds = list.base_handle.bounds();
        let scroll_y = list.base_handle.offset().y;
        let top = bounds.origin.y + scroll_y + ROW_HEIGHT * position.row as f32;
        let left = match position.column {
            0 => bounds.origin.x + GUTTER_WIDTH,
            column => {
                bounds.origin.x
                    + GUTTER_WIDTH
                    + TITLE_COLUMN_WIDTH
                    + self.columns.left_of(column - 1)
                    - self.horizontal_offset
            }
        };
        point(left, top + ROW_HEIGHT)
    }
}

fn visible_column_ids(model: &TableModel) -> Vec<PropertyId> {
    model
        .visible_properties()
        .iter()
        .map(|property| property.id)
        .collect()
}

#[cfg(test)]
mod tests;
