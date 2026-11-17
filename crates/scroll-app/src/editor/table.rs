//! Tables: a grid of text. A table is a block whose children are its rows, and a row's children
//! are its cells; each cell is a block of text like any other, so typing, marks, undo and other
//! people's carets work in a cell as they do in a paragraph. The page's list shows a table and
//! everything in it as one row (`rows`). This module is what knows a table's shape: reading it,
//! moving between cells, adding and removing rows and columns, and drawing the grid.

use std::ops::Range;

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Point, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use scroll_ui::{Icon, IconName, IconSize, Menu, MenuItem, PopupAnchor, Theme};

use super::block_element::BlockElement;
use super::{BlockSelection, PageEditor, Selection};
use crate::editor::document::{
    BlockId, BlockInfo, BlockKind, PageDocument, PageError, PageResult, Placement,
};

/// What `/table` makes.
const NEW_TABLE_ROWS: usize = 3;
const NEW_TABLE_COLUMNS: usize = 3;
/// How thick the bars that add a row or a column are.
const ADD_BAR: Pixels = px(18.0);

/// Where a cell is in its table, counting from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellPlace {
    pub(super) row: usize,
    pub(super) column: usize,
}

/// One row of a table: its own block, and its cells from left to right.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TableRow {
    block: BlockId,
    cells: Vec<BlockId>,
}

/// A table's rows and cells as they are in the page.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Table {
    block: BlockId,
    rows: Vec<TableRow>,
}

impl Table {
    /// Reads a table from `blocks`: its own block first, then everything nested under it.
    /// Blocks under it that are not its rows or their cells are left out.
    pub(super) fn read(blocks: &[BlockInfo]) -> Option<Table> {
        let (table, parts) = blocks.split_first()?;
        if table.kind != BlockKind::Table {
            return None;
        }
        let mut rows: Vec<TableRow> = Vec::new();
        for part in parts {
            let is_row = part.kind == BlockKind::TableRow && part.parent == Some(table.id);
            if is_row {
                rows.push(TableRow {
                    block: part.id,
                    cells: Vec::new(),
                });
                continue;
            }
            let row = rows
                .last_mut()
                .filter(|row| part.kind == BlockKind::TableCell && part.parent == Some(row.block));
            if let Some(row) = row {
                row.cells.push(part.id);
            }
        }
        Some(Table {
            block: table.id,
            rows,
        })
    }

    /// The most cells any row has. Rows are the same length unless two people changed the
    /// table at once; a short row is drawn padded.
    fn column_count(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0)
    }

    /// Every cell, row by row.
    fn cells(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.rows.iter().flat_map(|row| row.cells.iter().copied())
    }

    fn place_of(&self, cell: BlockId) -> Option<CellPlace> {
        self.rows.iter().enumerate().find_map(|(row, cells)| {
            let column = cells
                .cells
                .iter()
                .position(|candidate| *candidate == cell)?;
            Some(CellPlace { row, column })
        })
    }

    /// The cell at `place`, or the last cell of its row if the row is shorter.
    fn cell_near(&self, place: CellPlace) -> Option<BlockId> {
        let row = self.rows.get(place.row)?;
        row.cells.get(place.column).or(row.cells.last()).copied()
    }

    fn row_of(&self, cell: BlockId) -> Option<BlockId> {
        Some(self.rows[self.place_of(cell)?.row].block)
    }
}

/// A change to a table's rows or columns, relative to the cell holding the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableEdit {
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    DeleteRow,
    DeleteColumn,
    DeleteTable,
}

impl TableEdit {
    const MENU: [(&'static str, TableEdit); 7] = [
        ("Insert row above", TableEdit::InsertRowAbove),
        ("Insert row below", TableEdit::InsertRowBelow),
        ("Insert column left", TableEdit::InsertColumnLeft),
        ("Insert column right", TableEdit::InsertColumnRight),
        ("Delete row", TableEdit::DeleteRow),
        ("Delete column", TableEdit::DeleteColumn),
        ("Delete table", TableEdit::DeleteTable),
    ];
}

/// Makes an empty table at `placement` and returns its first cell.
fn insert_table(page: &PageDocument, placement: Placement) -> PageResult<BlockId> {
    let table = page.insert_block(placement, BlockKind::Table)?;
    let mut cells = Vec::new();
    for _ in 0..NEW_TABLE_ROWS {
        let row = Placement::LastChildOf(Some(table));
        cells.extend(insert_row(page, row, NEW_TABLE_COLUMNS)?);
    }
    cells
        .first()
        .copied()
        .ok_or(PageError::BlockNotFound(table))
}

/// Makes a row of `columns` empty cells at `placement` and returns the cells.
fn insert_row(
    page: &PageDocument,
    placement: Placement,
    columns: usize,
) -> PageResult<Vec<BlockId>> {
    let row = page.insert_block(placement, BlockKind::TableRow)?;
    (0..columns)
        .map(|_| page.insert_block(Placement::LastChildOf(Some(row)), BlockKind::TableCell))
        .collect()
}

impl PageEditor {
    /// The table that the block at `index` is a part of (one of its rows or cells), as the
    /// index of the table's block.
    pub(super) fn table_index_of(&self, index: usize) -> Option<usize> {
        let table = self.row_span(index).start;
        (table != index && self.blocks[table].info.kind == BlockKind::Table).then_some(table)
    }

    pub(super) fn is_in_table(&self, block: BlockId) -> bool {
        self.block_index(block)
            .is_some_and(|index| self.table_index_of(index).is_some())
    }

    /// The block that is selected, dragged and deleted as one for `block`: the table for a
    /// part of a table, else the block itself.
    pub(super) fn whole_block(&self, block: BlockId) -> BlockId {
        self.block_index(block)
            .and_then(|index| self.table_index_of(index))
            .map_or(block, |table| self.blocks[table].info.id)
    }

    fn table_at(&self, table_index: usize) -> Option<Table> {
        let infos: Vec<BlockInfo> = self.blocks[self.row_span(table_index)]
            .iter()
            .map(|state| state.info)
            .collect();
        Table::read(&infos)
    }

    /// The table holding the caret, and the place of the caret's cell in it.
    fn caret_in_table(&self) -> Option<(Table, CellPlace)> {
        let index = self.block_index(self.selection.block)?;
        let table = self.table_at(self.table_index_of(index)?)?;
        let place = table.place_of(self.selection.block)?;
        Some((table, place))
    }

    /// The table row holding the caret, which the block-moving keys move.
    pub(super) fn caret_table_row(&self) -> Option<BlockId> {
        let (table, _) = self.caret_in_table()?;
        table.row_of(self.selection.block)
    }

    /// Puts the caret at the end of `cell`'s text.
    fn move_caret_to_cell(&mut self, cell: BlockId, cx: &mut Context<Self>) {
        let end = self.content(cell).map_or(0, |content| content.text.len());
        self.move_caret(cell, end, cx);
    }

    /// Tab in a table: the next cell, row by row; after the last one, a new row. False when
    /// the caret is not in a table.
    pub(super) fn move_to_next_cell(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((table, _)) = self.caret_in_table() else {
            return false;
        };
        let caret = self.selection.block;
        let next = table.cells().skip_while(|cell| *cell != caret).nth(1);
        match next {
            Some(cell) => self.move_caret_to_cell(cell, cx),
            None => {
                let new_row = CellPlace {
                    row: table.rows.len(),
                    column: 0,
                };
                self.insert_table_row(&table, new_row, cx);
            }
        }
        true
    }

    /// Shift+Tab in a table: the cell before, staying put in the first one.
    pub(super) fn move_to_previous_cell(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((table, _)) = self.caret_in_table() else {
            return false;
        };
        let caret = self.selection.block;
        let previous = table.cells().take_while(|cell| *cell != caret).last();
        if let Some(cell) = previous {
            self.move_caret_to_cell(cell, cx);
        }
        true
    }

    /// Enter in a table: the cell below; under the last row, a new row.
    pub(super) fn move_to_cell_below(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((table, place)) = self.caret_in_table() else {
            return false;
        };
        let below = CellPlace {
            row: place.row + 1,
            ..place
        };
        match table.cell_near(below) {
            Some(cell) => self.move_caret_to_cell(cell, cx),
            None => self.insert_table_row(&table, below, cx),
        }
        true
    }

    /// The cell straight above or below the caret's, for the arrow keys; `None` at the table's
    /// top or bottom edge, or when the caret is not in a table.
    pub(super) fn cell_beside_caret(&self, rows_down: isize) -> Option<BlockId> {
        let (table, place) = self.caret_in_table()?;
        let row = place.row.checked_add_signed(rows_down)?;
        table.cell_near(CellPlace { row, ..place })
    }

    /// The blocks of the table the caret is in, for stepping out of it.
    pub(super) fn caret_table_span(&self) -> Option<Range<usize>> {
        let index = self.block_index(self.selection.block)?;
        Some(self.row_span(self.table_index_of(index)?))
    }

    /// The first cell of the table row holding `cell`, where the caret lands coming up into a
    /// table from below it.
    pub(super) fn first_cell_in_row_of(&self, cell: BlockId) -> Option<BlockId> {
        let index = self.block_index(cell)?;
        let table = self.table_at(self.table_index_of(index)?)?;
        let row = table.place_of(cell)?.row;
        table.rows[row].cells.first().copied()
    }

    /// Where the caret goes when it lands on the table at `table_index` without a cell in
    /// mind: its first cell. `None` if the block there is not a table.
    pub(super) fn first_cell_of(&self, table_index: usize) -> Option<BlockId> {
        self.table_at(table_index)?.cells().next()
    }

    /// Adds a row at `new.row` and puts the caret in its cell in column `new.column`.
    fn insert_table_row(&mut self, table: &Table, new: CellPlace, cx: &mut Context<Self>) {
        let placement = match table.rows.get(new.row) {
            Some(row) => Placement::Before(row.block),
            None => Placement::LastChildOf(Some(table.block)),
        };
        let columns = table.column_count().max(1);
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            let cells = insert_row(page, placement, columns)?;
            let caret = cells.get(new.column).or(cells.last());
            Ok(caret.map_or(selection, |cell| Selection::caret(*cell, 0)))
        });
    }

    /// Adds a column at `new.column` and puts the caret in its cell in row `new.row`.
    fn insert_table_column(&mut self, table: &Table, new: CellPlace, cx: &mut Context<Self>) {
        let slots: Vec<Placement> = table
            .rows
            .iter()
            .map(|row| match row.cells.get(new.column) {
                Some(cell) => Placement::Before(*cell),
                None => Placement::LastChildOf(Some(row.block)),
            })
            .collect();
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            let mut cells = Vec::with_capacity(slots.len());
            for slot in slots {
                cells.push(page.insert_block(slot, BlockKind::TableCell)?);
            }
            let caret = cells.get(new.row).or(cells.last());
            Ok(caret.map_or(selection, |cell| Selection::caret(*cell, 0)))
        });
    }

    /// Deletes the row at `place`, leaving the caret in the same column of the row that takes
    /// its place. Deleting the only row deletes the table.
    fn delete_table_row(&mut self, table: &Table, place: CellPlace, cx: &mut Context<Self>) {
        let Some(row) = table.rows.get(place.row).map(|row| row.block) else {
            return;
        };
        if table.rows.len() == 1 {
            return self.delete_table(table, cx);
        }
        let is_last = place.row + 1 == table.rows.len();
        let neighbour = CellPlace {
            row: if is_last {
                place.row - 1
            } else {
                place.row + 1
            },
            ..place
        };
        let caret = table.cell_near(neighbour);
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            page.delete_block(row)?;
            Ok(caret.map_or(selection, |cell| Selection::caret(cell, 0)))
        });
    }

    /// Deletes the column at `place`, leaving the caret in the same row of the column that
    /// takes its place. Deleting the only column deletes the table.
    fn delete_table_column(&mut self, table: &Table, place: CellPlace, cx: &mut Context<Self>) {
        let columns = table.column_count();
        if columns <= 1 {
            return self.delete_table(table, cx);
        }
        let doomed: Vec<BlockId> = table
            .rows
            .iter()
            .filter_map(|row| row.cells.get(place.column).copied())
            .collect();
        let is_last = place.column + 1 == columns;
        let neighbour = CellPlace {
            column: if is_last {
                place.column - 1
            } else {
                place.column + 1
            },
            ..place
        };
        let caret = table.cell_near(neighbour);
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            for cell in &doomed {
                page.delete_block(*cell)?;
            }
            Ok(caret.map_or(selection, |cell| Selection::caret(cell, 0)))
        });
    }

    /// Adds the `lacking` cells a short row is missing and puts the caret in the last of them.
    fn fill_table_row(&mut self, row: BlockId, lacking: usize, cx: &mut Context<Self>) {
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            let mut caret = selection;
            for _ in 0..lacking {
                let end_of_row = Placement::LastChildOf(Some(row));
                caret = Selection::caret(page.insert_block(end_of_row, BlockKind::TableCell)?, 0);
            }
            Ok(caret)
        });
    }

    fn delete_table(&mut self, table: &Table, cx: &mut Context<Self>) {
        self.block_selection = Some(BlockSelection {
            anchor: table.block,
            head: table.block,
        });
        self.delete_selected_blocks(cx);
    }

    pub(super) fn edit_table(&mut self, edit: TableEdit, cx: &mut Context<Self>) {
        let Some((table, place)) = self.caret_in_table() else {
            return;
        };
        let below = CellPlace {
            row: place.row + 1,
            ..place
        };
        let right = CellPlace {
            column: place.column + 1,
            ..place
        };
        match edit {
            TableEdit::InsertRowAbove => self.insert_table_row(&table, place, cx),
            TableEdit::InsertRowBelow => self.insert_table_row(&table, below, cx),
            TableEdit::InsertColumnLeft => self.insert_table_column(&table, place, cx),
            TableEdit::InsertColumnRight => self.insert_table_column(&table, right, cx),
            TableEdit::DeleteRow => self.delete_table_row(&table, place, cx),
            TableEdit::DeleteColumn => self.delete_table_column(&table, place, cx),
            TableEdit::DeleteTable => self.delete_table(&table, cx),
        }
    }

    /// Adds a row under the last one, or a column after the last one, of the table at `index`.
    fn append_to_table(&mut self, table_index: usize, edit: TableEdit, cx: &mut Context<Self>) {
        let Some(table) = self.table_at(table_index) else {
            return;
        };
        let end = CellPlace {
            row: table.rows.len(),
            column: table.column_count(),
        };
        match edit {
            TableEdit::InsertColumnRight if !table.rows.is_empty() => {
                self.insert_table_column(&table, CellPlace { row: 0, ..end }, cx)
            }
            _ => self.insert_table_row(&table, CellPlace { column: 0, ..end }, cx),
        }
    }

    /// Turns `block` into a table, removing `markup` (the slash command typed): an empty line
    /// stays as the line under the new table, to carry on writing there; a line with text
    /// keeps it and gets the table below.
    pub(super) fn convert_to_table(
        &mut self,
        block: BlockId,
        markup: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.completion = None;
        self.apply_edit(cx, |page| {
            page.delete_text(block, markup.clone())?;
            let placement = if page.block_content(block)?.text.is_empty() {
                Placement::Before(block)
            } else {
                Placement::After(block)
            };
            Ok(Selection::caret(insert_table(page, placement)?, 0))
        });
    }

    /// The menu of row and column changes for `cell`, opened by a secondary click on it.
    fn open_table_menu(
        &mut self,
        cell: BlockId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.move_caret_to_cell(cell, cx);
        let editor = cx.entity().downgrade();
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| {
                    Menu::new(cx).items(TableEdit::MENU.into_iter().map(|(label, edit)| {
                        let editor = editor.clone();
                        MenuItem::new(label, move |_, cx| {
                            let edited =
                                editor.update(cx, |editor, cx| editor.edit_table(edit, cx));
                            if let Err(error) = edited {
                                log::debug!("page closed before its table was changed: {error}");
                            }
                        })
                    }))
                },
                window,
                cx,
            );
        });
    }

    /// The grid of the table whose block is at `index`, with the bars that add a row and a
    /// column showing while the pointer is over it.
    pub(super) fn render_table(
        &self,
        index: usize,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(table) = self.table_at(index) else {
            return div().into_any_element();
        };
        let name = self.blocks[index].info.id.to_string();
        let group = SharedString::from(format!("table-{name}"));
        let columns = table.column_count();
        let is_focused = self.focus_handle.is_focused(window);
        let rows = table.rows.iter().enumerate().map(|(row_index, row)| {
            let cells = (0..columns)
                .map(|column| self.render_table_cell(row, column, theme, is_focused, cx));
            div()
                .flex()
                .when(row_index > 0, |row| {
                    row.border_t(theme.size.border_width)
                        .border_color(theme.colors.border)
                })
                .children(cells)
        });
        let grid = div()
            .flex_1()
            .min_w_0()
            .rounded(theme.radius.md)
            .border(theme.size.border_width)
            .border_color(theme.colors.border)
            .overflow_hidden()
            .when(self.is_block_selected(index), |grid| {
                grid.bg(theme.colors.text_selection)
            })
            .children(rows);
        if self.is_read_only() {
            return div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(grid)
                .into_any_element();
        }
        let add_column =
            self.render_add_bar(index, TableEdit::InsertColumnRight, &group, theme, cx);
        let add_row = self.render_add_bar(index, TableEdit::InsertRowBelow, &group, theme, cx);
        div()
            .group(group)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(theme.spacing.xxs)
            .child(
                div()
                    .flex()
                    .gap(theme.spacing.xxs)
                    .child(grid)
                    .child(add_column.w(ADD_BAR)),
            )
            // As wide as the grid above it.
            .child(add_row.h(ADD_BAR).mr(ADD_BAR + theme.spacing.xxs))
            .into_any_element()
    }

    /// The cell of `row` in `column`. A row short of the table's width is drawn padded, and
    /// a press on the padding makes the cells it lacks.
    fn render_table_cell(
        &self,
        row: &TableRow,
        column: usize,
        theme: &Theme,
        is_focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let frame = div()
            .flex_1()
            .min_w_0()
            .px(theme.spacing.sm)
            .py(theme.spacing.xs)
            .min_h(theme.text.body.line_height + theme.spacing.xs * 2.0)
            .when(column > 0, |cell| {
                cell.border_l(theme.size.border_width)
                    .border_color(theme.colors.border)
            });
        let cell = row.cells.get(column).copied();
        let Some((cell, index)) = cell.and_then(|cell| Some((cell, self.block_index(cell)?)))
        else {
            let (row, lacking) = (row.block, column + 1 - row.cells.len().min(column + 1));
            return frame
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |editor, _, window, cx| {
                        cx.stop_propagation();
                        window.focus(&editor.focus_handle);
                        editor.fill_table_row(row, lacking, cx);
                    }),
                )
                .into_any_element();
        };
        let text = BlockElement::new(
            cx.entity(),
            cell,
            self.block_paint(index, theme, is_focused),
        );
        frame
            .id(ElementId::Name(format!("table-cell-{cell}").into()))
            .debug_selector(|| format!("table-cell-{index}"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |editor, event, window, cx| {
                    editor.mouse_down(cell, event, window, cx)
                }),
            )
            .when(!self.is_read_only(), |frame| {
                frame.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |editor, event: &gpui::MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        editor.open_table_menu(cell, event.position, window, cx)
                    }),
                )
            })
            .child(text)
            .into_any_element()
    }

    /// A bar along the table's bottom or right edge that adds a row or a column, shown while
    /// the pointer is over the table.
    fn render_add_bar(
        &self,
        table_index: usize,
        edit: TableEdit,
        group: &SharedString,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let table = self.blocks[table_index].info.id;
        let what = match edit {
            TableEdit::InsertColumnRight => "column",
            _ => "row",
        };
        let hover = theme.colors.hover;
        div()
            .id(ElementId::Name(format!("table-add-{what}-{table}").into()))
            .debug_selector(|| format!("table-add-{what}"))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme.radius.sm)
            .cursor_pointer()
            .opacity(0.)
            .group_hover(group.clone(), |bar| bar.opacity(1.))
            .hover(move |bar| bar.opacity(1.).bg(hover))
            .child(
                Icon::new(IconName::Plus)
                    .size(IconSize::Small)
                    .color(theme.colors.text_placeholder),
            )
            // The press is the bar's own: it must not also reach the table's row.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |editor, _, window, cx| {
                window.focus(&editor.focus_handle);
                if let Some(table_index) = editor.block_index(table) {
                    editor.append_to_table(table_index, edit, cx);
                }
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page holding one table of `rows` by `columns`, as its blocks.
    fn table_blocks(rows: usize, columns: usize) -> Vec<BlockInfo> {
        let mut page = PageDocument::create(1, &[]).unwrap();
        let table = page
            .insert_block(Placement::LastChildOf(None), BlockKind::Table)
            .unwrap();
        for _ in 0..rows {
            insert_row(&page, Placement::LastChildOf(Some(table)), columns).unwrap();
        }
        page.commit();
        page.blocks().to_vec()
    }

    #[test]
    fn a_table_is_read_as_rows_of_cells() {
        let blocks = table_blocks(2, 3);
        let table = Table::read(&blocks).unwrap();
        assert_eq!((table.rows.len(), table.column_count()), (2, 3));
        assert_eq!(table.cells().count(), 6);
        // Blocks: table, row, cell, cell, cell, row, cell, cell, cell.
        let last = blocks[8].id;
        assert_eq!(table.place_of(last), Some(CellPlace { row: 1, column: 2 }));
        assert_eq!(table.row_of(last), Some(blocks[5].id));
        assert_eq!(table.place_of(blocks[1].id), None, "a row is not a cell");
        assert!(
            Table::read(&blocks[1..]).is_none(),
            "not starting at a table"
        );
    }

    #[test]
    fn a_short_row_gives_its_last_cell_for_a_column_it_lacks() {
        let mut blocks = table_blocks(2, 3);
        // The second row loses its last cell, as when a column and a row are added at once.
        blocks.pop();
        let table = Table::read(&blocks).unwrap();
        assert_eq!(table.column_count(), 3);
        let below = table.cell_near(CellPlace { row: 1, column: 2 });
        assert_eq!(below, Some(blocks[7].id));
        assert_eq!(table.cell_near(CellPlace { row: 2, column: 0 }), None);
    }

    #[test]
    fn a_new_table_has_its_rows_and_columns_and_starts_in_its_first_cell() {
        let mut page = PageDocument::create(1, &["above"]).unwrap();
        let above = page.block_ids()[0];
        let first_cell = insert_table(&page, Placement::After(above)).unwrap();
        page.commit();
        let blocks = page.blocks();
        let table = Table::read(&blocks[1..]).unwrap();
        assert_eq!(
            (table.rows.len(), table.column_count()),
            (NEW_TABLE_ROWS, NEW_TABLE_COLUMNS)
        );
        assert_eq!(table.cells().next(), Some(first_cell));
    }
}
