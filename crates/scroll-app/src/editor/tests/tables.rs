//! Tables driven as a person would: the slash menu, typing, Tab, Enter and the arrows, the bars
//! and menu that add and remove rows and columns, and the clipboard.

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};

use super::{kinds, open_editor, texts};
use crate::editor::document::{BlockId, BlockKind};
use crate::editor::{PageAccess, PageEditor, Selection};

/// An editor whose page holds a new table (three by three) above an empty paragraph, with the
/// caret in the table's first cell.
fn open_table(cx: &mut TestAppContext) -> (Entity<PageEditor>, &mut VisualTestContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("/table");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    (editor, cx)
}

/// The table's text, row by row.
fn cells(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    editor.read_with(cx, |editor, _| {
        let mut rows: Vec<Vec<String>> = Vec::new();
        for block in editor.page().blocks().iter() {
            match block.kind {
                BlockKind::TableRow => rows.push(Vec::new()),
                BlockKind::TableCell => {
                    let text = editor.page().block_content(block.id).unwrap().text;
                    rows.last_mut().expect("a cell is in a row").push(text);
                }
                _ => {}
            }
        }
        rows
    })
}

fn caret(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> BlockId {
    editor.read_with(cx, |editor, _| editor.selection().block)
}

/// The caret's cell as (row, column).
fn caret_cell(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> (usize, usize) {
    editor.read_with(cx, |editor, _| {
        let mut row = 0;
        let mut column = 0;
        let mut rows_seen = 0;
        for block in editor.page().blocks().iter() {
            match block.kind {
                BlockKind::TableRow => {
                    rows_seen += 1;
                    row = rows_seen - 1;
                    column = 0;
                }
                BlockKind::TableCell if block.id == editor.selection().block => {
                    return (row, column);
                }
                BlockKind::TableCell => column += 1,
                _ => {}
            }
        }
        panic!("the caret is not in a table cell");
    })
}

fn row_strings(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| row.iter().map(|cell| cell.to_string()).collect())
        .collect()
}

/// Types `text` into each cell in turn, moving on with Tab.
fn fill(texts: &[&str], cx: &mut VisualTestContext) {
    for (index, text) in texts.iter().enumerate() {
        if index > 0 {
            cx.simulate_keystrokes("tab");
        }
        cx.simulate_input(text);
    }
}

#[gpui::test]
fn the_slash_menu_makes_a_table_above_the_empty_line(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    let empty_row = ["", "", ""].as_slice();
    assert_eq!(
        cells(&editor, cx),
        row_strings(&[empty_row, empty_row, empty_row])
    );
    assert_eq!(caret_cell(&editor, cx), (0, 0));
    let page = kinds(&editor, cx);
    assert_eq!(page[0], (BlockKind::Table, 0));
    assert_eq!(page[1], (BlockKind::TableRow, 1));
    assert_eq!(page[2], (BlockKind::TableCell, 2));
    assert_eq!(
        page.last(),
        Some(&(BlockKind::Paragraph, 0)),
        "the line the table was made on stays below it"
    );
    let list_rows = editor.read_with(cx, |editor, _| editor.rows.count());
    assert_eq!(
        list_rows, 2,
        "the table is one row of the page, then the line"
    );
}

#[gpui::test]
fn a_table_made_on_a_line_with_text_goes_below_it(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["notes"], cx);
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" /table");
    cx.simulate_keystrokes("enter");
    let page = kinds(&editor, cx);
    assert_eq!(page[0], (BlockKind::Paragraph, 0));
    assert_eq!(page[1], (BlockKind::Table, 0));
    assert_eq!(texts(&editor, cx)[0], "notes ");
    assert_eq!(caret_cell(&editor, cx), (0, 0));
}

#[gpui::test]
fn tab_and_enter_move_through_cells_and_add_rows_at_the_end(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    fill(&["a", "b", "c", "d"], cx);
    assert_eq!(caret_cell(&editor, cx), (1, 0), "tab wraps to the next row");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(caret_cell(&editor, cx), (0, 2));
    cx.simulate_keystrokes("shift-tab shift-tab shift-tab");
    assert_eq!(
        caret_cell(&editor, cx),
        (0, 0),
        "and stops at the first cell"
    );

    cx.simulate_keystrokes("enter enter");
    assert_eq!(caret_cell(&editor, cx), (2, 0), "enter goes down a row");
    cx.simulate_keystrokes("enter");
    assert_eq!(caret_cell(&editor, cx), (3, 0));
    assert_eq!(
        cells(&editor, cx).len(),
        4,
        "under the last row is a new one"
    );

    cx.simulate_keystrokes("tab tab tab");
    assert_eq!(caret_cell(&editor, cx), (4, 0));
    assert_eq!(cells(&editor, cx).len(), 5, "and so is after the last cell");
    assert_eq!(cells(&editor, cx)[0], ["a", "b", "c"]);
    assert_eq!(cells(&editor, cx)[1][0], "d");
}

#[gpui::test]
fn a_cell_holds_text_like_any_block_but_keeps_its_kind(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    cx.simulate_input("# not a **heading**");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("/quote");
    assert!(
        editor.read_with(cx, |editor, _| editor.completion.is_none()),
        "no block menu in a cell"
    );
    assert_eq!(cells(&editor, cx)[0][0], "# not a heading\n/quote");
    assert_eq!(kinds(&editor, cx)[2], (BlockKind::TableCell, 2));

    // Backspace at the start and Delete at the end leave the cell and its neighbours alone.
    cx.simulate_keystrokes("tab backspace");
    cx.simulate_keystrokes("shift-tab delete");
    assert_eq!(cells(&editor, cx)[0].len(), 3);
    assert_eq!(cells(&editor, cx)[0][0], "# not a heading\n/quote");
}

#[gpui::test]
fn the_arrows_move_between_rows_and_out_of_the_table(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["above", ""], cx);
    cx.simulate_keystrokes("down");
    cx.simulate_input("/table");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    fill(&["a", "b"], cx);
    assert_eq!(caret_cell(&editor, cx), (0, 1));

    cx.simulate_keystrokes("down");
    assert_eq!(caret_cell(&editor, cx), (1, 1), "down keeps the column");
    cx.simulate_keystrokes("down down");
    let below = editor.read_with(cx, |editor, _| *editor.page().block_ids().last().unwrap());
    assert_eq!(
        caret(&editor, cx),
        below,
        "below the last row is the next line"
    );

    cx.simulate_keystrokes("up");
    assert_eq!(
        caret_cell(&editor, cx),
        (2, 0),
        "up from below enters the last row"
    );
    cx.simulate_keystrokes("up up up");
    let above = editor.read_with(cx, |editor, _| editor.page().block_ids()[0]);
    assert_eq!(
        caret(&editor, cx),
        above,
        "and above the first row is the line before"
    );

    // Left and right step through the cells in reading order.
    cx.simulate_keystrokes("cmd-right right");
    assert_eq!(caret_cell(&editor, cx), (0, 0));
    cx.simulate_keystrokes("right right");
    assert_eq!(caret_cell(&editor, cx), (0, 1));
}

#[gpui::test]
fn rows_and_columns_are_added_and_deleted_around_the_caret(cx: &mut TestAppContext) {
    use crate::editor::table::TableEdit;

    let (editor, cx) = open_table(cx);
    fill(&["a", "b", "c", "d", "e", "f"], cx);
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(caret_cell(&editor, cx), (1, 1));
    let edit = |edit: TableEdit, cx: &mut VisualTestContext| {
        editor.update(cx, |editor, cx| editor.edit_table(edit, cx));
    };

    edit(TableEdit::InsertColumnLeft, cx);
    assert_eq!(
        caret_cell(&editor, cx),
        (1, 1),
        "the caret is in the new column"
    );
    assert_eq!(cells(&editor, cx)[0], ["a", "", "b", "c"]);
    assert_eq!(cells(&editor, cx)[1], ["d", "", "e", "f"]);

    edit(TableEdit::InsertRowBelow, cx);
    assert_eq!(caret_cell(&editor, cx), (2, 1));
    assert_eq!(cells(&editor, cx).len(), 4);
    assert_eq!(cells(&editor, cx)[2], ["", "", "", ""]);

    edit(TableEdit::DeleteRow, cx);
    assert_eq!(cells(&editor, cx).len(), 3);
    assert_eq!(caret_cell(&editor, cx), (2, 1));
    // Loro folds edits made within its merge interval into one undo step.
    std::thread::sleep(std::time::Duration::from_millis(700));
    edit(TableEdit::DeleteColumn, cx);
    assert_eq!(cells(&editor, cx)[0], ["a", "b", "c"]);
    assert_eq!(cells(&editor, cx)[1], ["d", "e", "f"]);

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        cells(&editor, cx)[0],
        ["a", "", "b", "c"],
        "deleting a column is one undo step"
    );
    assert_eq!(cells(&editor, cx)[1], ["d", "", "e", "f"]);
}

#[gpui::test]
fn deleting_the_last_row_or_the_table_leaves_the_rest_of_the_page(cx: &mut TestAppContext) {
    use crate::editor::table::TableEdit;

    let (editor, cx) = open_table(cx);
    for _ in 0..3 {
        editor.update(cx, |editor, cx| editor.edit_table(TableEdit::DeleteRow, cx));
    }
    assert_eq!(kinds(&editor, cx), [(BlockKind::Paragraph, 0)]);
    let only = editor.read_with(cx, |editor, _| editor.page().block_ids()[0]);
    assert_eq!(caret(&editor, cx), only);
}

#[gpui::test]
fn escape_selects_the_whole_table_which_backspace_deletes(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    cx.simulate_input("x");
    cx.simulate_keystrokes("escape");
    let selected = editor.read_with(cx, |editor, _| editor.selected_block_range());
    assert_eq!(selected, Some(0..13), "the table with its rows and cells");
    cx.simulate_keystrokes("enter");
    assert_eq!(caret_cell(&editor, cx), (0, 0), "enter goes back into it");

    cx.simulate_keystrokes("escape backspace");
    assert_eq!(kinds(&editor, cx), [(BlockKind::Paragraph, 0)]);
}

#[gpui::test]
fn backspace_under_a_table_selects_it_instead_of_joining_it(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    cx.simulate_keystrokes("cmd-down backspace");
    let selected = editor.read_with(cx, |editor, _| editor.selected_block_range());
    assert_eq!(selected, Some(0..13));
    assert_eq!(kinds(&editor, cx).len(), 14, "nothing was deleted");
}

#[gpui::test]
fn cmd_shift_arrows_move_the_carets_row(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    fill(&["a", "b", "c", "d"], cx);
    cx.simulate_keystrokes("cmd-shift-up");
    assert_eq!(cells(&editor, cx)[0][0], "d");
    assert_eq!(cells(&editor, cx)[1], ["a", "b", "c"]);
    assert_eq!(caret_cell(&editor, cx), (0, 0));
}

#[gpui::test]
fn the_bars_under_and_beside_a_table_add_a_row_and_a_column(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    let add_row = cx
        .debug_bounds("table-add-row")
        .expect("a bar under the table");
    cx.simulate_click(add_row.center(), Modifiers::none());
    assert_eq!(cells(&editor, cx).len(), 4);
    assert_eq!(caret_cell(&editor, cx), (3, 0));

    cx.run_until_parked();
    let add_column = cx
        .debug_bounds("table-add-column")
        .expect("a bar beside it");
    cx.simulate_click(add_column.center(), Modifiers::none());
    assert_eq!(cells(&editor, cx)[0].len(), 4);
    assert_eq!(caret_cell(&editor, cx), (0, 3));
}

#[gpui::test]
fn a_click_puts_the_caret_in_a_cell_and_a_secondary_click_opens_its_menu(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    // Blocks: table, row, three cells, row, three cells...; the fifth cell is block 7.
    let middle = cx.debug_bounds("table-cell-7").expect("the middle cell");
    cx.simulate_click(middle.center(), Modifiers::none());
    assert_eq!(caret_cell(&editor, cx), (1, 1));

    let corner = cx.debug_bounds("table-cell-12").expect("the last cell");
    cx.simulate_mouse_down(corner.center(), MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(caret_cell(&editor, cx), (2, 2));
    assert!(editor.read_with(cx, |editor, cx| editor.popups.read(cx).is_open()));
}

#[gpui::test]
fn a_viewer_reads_a_table_but_cannot_change_its_shape(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("/table");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("kept");
    editor.update(cx, |editor, cx| editor.set_access(PageAccess::ReadOnly, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("tab tab tab tab tab tab tab tab tab enter");
    assert_eq!(cells(&editor, cx).len(), 3);
    assert_eq!(cells(&editor, cx)[0][0], "kept");
}

#[gpui::test]
fn a_table_is_copied_as_markdown_and_pasted_back_as_a_table(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    fill(&["Name", "Role", "", "Ada", "Engineer"], cx);
    cx.simulate_keystrokes("escape cmd-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        copied.as_deref(),
        Some("| Name | Role |  |\n| --- | --- | --- |\n| Ada | Engineer |  |\n|  |  |  |")
    );

    // Pasted on the empty line below, it becomes a second table.
    cx.simulate_keystrokes("cmd-down cmd-v");
    let tables = kinds(&editor, cx)
        .iter()
        .filter(|(kind, _)| *kind == BlockKind::Table)
        .count();
    assert_eq!(tables, 2);
    let all = cells(&editor, cx);
    assert_eq!(all.len(), 6);
    assert_eq!(all[3], ["Name", "Role", ""]);
    assert_eq!(all[4], ["Ada", "Engineer", ""]);
}

#[gpui::test]
fn a_markdown_table_from_elsewhere_pastes_as_a_table_below_a_line_with_text(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_editor(&["Team"], cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        "| Name | Role |\n| --- | --- |\n| Ada | Engineer |".into(),
    ));
    cx.simulate_keystrokes("cmd-down cmd-v");
    let page = kinds(&editor, cx);
    assert_eq!(page[0], (BlockKind::Paragraph, 0));
    assert_eq!(page[1], (BlockKind::Table, 0));
    assert_eq!(texts(&editor, cx)[0], "Team");
    assert_eq!(
        cells(&editor, cx),
        row_strings(&[&["Name", "Role"], &["Ada", "Engineer"]])
    );
}

#[gpui::test]
fn several_lines_pasted_into_a_cell_stay_in_the_cell(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("# one\n- two".into()));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(cells(&editor, cx)[0][0], "# one\n- two");
    assert_eq!(cells(&editor, cx).len(), 3);
    assert_eq!(cells(&editor, cx)[0].len(), 3);
}

#[gpui::test]
fn a_click_below_a_page_that_ends_in_a_table_starts_a_new_line(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    // Delete the line under the table, so the page ends with it.
    cx.simulate_keystrokes("cmd-down escape backspace");
    assert_eq!(kinds(&editor, cx).last(), Some(&(BlockKind::TableCell, 2)));
    cx.run_until_parked();

    let below = cx.update(|window, _| {
        let size = window.viewport_size();
        gpui::point(size.width / 2.0, size.height - gpui::px(20.0))
    });
    cx.simulate_click(below, Modifiers::none());
    assert_eq!(kinds(&editor, cx).last(), Some(&(BlockKind::Paragraph, 0)));
    let last = editor.read_with(cx, |editor, _| *editor.page().block_ids().last().unwrap());
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection()),
        Selection::caret(last, 0)
    );
}

#[gpui::test]
fn a_click_on_the_gap_in_a_short_row_fills_it_in(cx: &mut TestAppContext) {
    let (editor, cx) = open_table(cx);
    // Another device adds a row while this one adds a column: the row arrives a cell short.
    editor.update(cx, |editor, cx| {
        let last_cell = editor.page().block_ids()[12];
        editor.apply_edit(cx, |page| {
            page.delete_block(last_cell)?;
            Ok(Selection::caret(page.block_ids()[2], 0))
        });
    });
    cx.run_until_parked();
    assert_eq!(cells(&editor, cx)[2].len(), 2);

    // The gap is where the last cell was: at the right of the bottom row.
    let beside = cx
        .debug_bounds("table-cell-11")
        .expect("the cell before the gap");
    let gap = gpui::point(beside.right() + beside.size.width / 2.0, beside.center().y);
    cx.simulate_click(gap, Modifiers::none());
    assert_eq!(cells(&editor, cx)[2].len(), 3);
    assert_eq!(caret_cell(&editor, cx), (2, 2));
}
