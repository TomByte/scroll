//! Drives a real `TableView` through GPUI's headless test platform: keystrokes go through the key
//! bindings, actions and text inputs, and every change is checked in the store. The test
//! platform does not shape text or paint, so layout and looks are not covered here.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use gpui::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext};
use scroll_core::id::{DatabaseId, PageId, PropertyId, SelectOptionId, ViewId};
use scroll_core::operation::{Operation, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{
    Condition, Database, Date, FilterCondition, Page, Property, PropertyType, PropertyValue,
    SelectOption, SortDirection, SortKey, View, ViewType,
};
use scroll_core::store::WorkspaceStore;

use super::{Command, TableView};
use crate::selection::CellPosition;

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    /// The test's own connection, for setting up and checking what the table wrote.
    store: Rc<RefCell<WorkspaceStore>>,
    view: ViewId,
    /// Visible columns, in order: notes, points, done, status, due.
    properties: Vec<PropertyId>,
    pages: Vec<PageId>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Best effort: a leftover temporary directory is harmless.
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn property(name: &str, property_type: PropertyType) -> Property {
    let options = match property_type {
        PropertyType::Select | PropertyType::MultiSelect => vec![SelectOption {
            id: SelectOptionId::generate(),
            name: "Todo".into(),
        }],
        _ => Vec::new(),
    };
    Property {
        id: PropertyId::generate(),
        name: name.into(),
        property_type,
        options,
    }
}

fn page(
    title: &str,
    position: Position,
    parent: Option<PageId>,
    row_of: Option<DatabaseId>,
) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: title.into(),
        icon: None,
        position,
        row_of,
        is_archived: false,
    }
}

fn fixture(row_count: usize) -> Fixture {
    let directory =
        std::env::temp_dir().join(format!("scroll-table-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    let mut store = WorkspaceStore::open(&path).unwrap();
    let host = page("Host", Position::between(None, None).unwrap(), None, None);
    let properties = vec![
        property("Notes", PropertyType::Text),
        property("Points", PropertyType::Number),
        property("Done", PropertyType::Checkbox),
        property("Status", PropertyType::Select),
        property("Due", PropertyType::Date),
    ];
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: properties.clone(),
    };
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: Vec::new(),
        sort: Vec::new(),
        visible_columns: properties.iter().map(|property| property.id).collect(),
        column_widths: Vec::new(),
    };
    let mut operations = vec![
        Operation::Create(Record::Page(host.clone())),
        Operation::Create(Record::Database(database.clone())),
        Operation::Create(Record::View(view.clone())),
    ];
    let mut position = Position::between(None, None).unwrap();
    let mut pages = Vec::new();
    for index in 0..row_count {
        let row = page(
            &format!("row {index}"),
            position.clone(),
            Some(host.id),
            Some(database.id),
        );
        position = Position::between(Some(&position), None).unwrap();
        pages.push(row.id);
        operations.push(Operation::Create(Record::Page(row)));
    }
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    Fixture {
        directory,
        path,
        store: Rc::new(RefCell::new(store)),
        view: view.id,
        properties: properties.iter().map(|property| property.id).collect(),
        pages,
    }
}

fn open<'a>(
    fixture: &Fixture,
    cx: &'a mut TestAppContext,
) -> (Entity<TableView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        scroll_ui::init(cx);
        crate::init(cx);
    });
    let path = fixture.path.clone();
    let view = fixture.view;
    let (table, cx) = cx.add_window_view(|window, cx| {
        let table = TableView::new(path, view, cx);
        window.focus(&table.focus_handle(cx));
        table
    });
    cx.run_until_parked();
    (table, cx)
}

/// The stored value, once the table's background writes have finished.
fn value(
    fixture: &Fixture,
    cx: &mut VisualTestContext,
    (row, column): (usize, usize),
) -> Option<PropertyValue> {
    cx.run_until_parked();
    let property = fixture.properties[column - 1];
    fixture
        .store
        .borrow()
        .property_values(fixture.pages[row])
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == property)
        .map(|(_, value)| value)
}

fn title(fixture: &Fixture, cx: &mut VisualTestContext, row: usize) -> String {
    cx.run_until_parked();
    fixture
        .store
        .borrow()
        .page(fixture.pages[row])
        .unwrap()
        .unwrap()
        .title
}

fn select(table: &Entity<TableView>, row: usize, column: usize, cx: &mut VisualTestContext) {
    table.update(cx, |table, cx| {
        table.select(CellPosition { row, column }, cx)
    });
}

fn selection(table: &Entity<TableView>, cx: &mut VisualTestContext) -> CellPosition {
    cx.run_until_parked();
    table.read_with(cx, |table, _| table.selection)
}

fn titles_shown(table: &Entity<TableView>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    table.read_with(cx, |table, _| {
        (0..table.model.row_count())
            .map(|index| match table.model.row(index) {
                Some(row) => row.page.title.clone(),
                None => "(not loaded)".into(),
            })
            .collect()
    })
}

#[gpui::test]
fn arrows_and_tab_move_the_selection(cx: &mut TestAppContext) {
    let fixture = fixture(3);
    let (table, cx) = open(&fixture, cx);
    cx.simulate_keystrokes("down right right");
    assert_eq!(selection(&table, cx), CellPosition { row: 1, column: 2 });
    select(&table, 0, 5, cx);
    cx.simulate_keystrokes("tab");
    assert_eq!(selection(&table, cx), CellPosition { row: 1, column: 0 });
    cx.simulate_keystrokes("shift-tab up");
    assert_eq!(selection(&table, cx), CellPosition { row: 0, column: 5 });
}

#[gpui::test]
fn typing_on_a_cell_edits_it_and_enter_saves(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("h i enter");
    assert_eq!(
        value(&fixture, cx, (0, 1)),
        Some(PropertyValue::Text("hi".into()))
    );
    assert!(table.read_with(cx, |table, _| table.text_edit.is_none()));

    select(&table, 1, 0, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input(" renamed");
    cx.simulate_keystrokes("enter");
    assert_eq!(title(&fixture, cx, 1), "row 1 renamed");
}

#[gpui::test]
fn escape_cancels_and_tab_saves_then_moves(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("x escape");
    assert_eq!(value(&fixture, cx, (0, 1)), None);

    cx.simulate_keystrokes("y tab");
    assert_eq!(
        value(&fixture, cx, (0, 1)),
        Some(PropertyValue::Text("y".into()))
    );
    assert_eq!(selection(&table, cx), CellPosition { row: 0, column: 2 });
}

#[gpui::test]
fn numbers_are_checked_before_saving(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 2, cx);
    cx.simulate_keystrokes("a enter");
    assert_eq!(value(&fixture, cx, (0, 2)), None);
    assert!(table.read_with(cx, |table, _| table.text_edit.is_some()
        && table.edit_error.is_some()));

    cx.simulate_keystrokes("backspace 4 2 enter");
    assert_eq!(
        value(&fixture, cx, (0, 2)),
        Some(PropertyValue::Number(42.0))
    );
}

#[gpui::test]
fn enter_toggles_a_checkbox(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 3, cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        value(&fixture, cx, (0, 3)),
        Some(PropertyValue::Checkbox(true))
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(
        value(&fixture, cx, (0, 3)),
        Some(PropertyValue::Checkbox(false))
    );
}

#[gpui::test]
fn the_select_picker_chooses_and_creates_options(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    let status = fixture.properties[3];
    let todo = options_of(&fixture, cx, status)[0];

    select(&table, 0, 4, cx);
    cx.simulate_keystrokes("enter");
    assert!(table.read_with(cx, |table, cx| {
        table.is_popup_open::<scroll_ui::Combobox>(cx)
    }));
    cx.simulate_input("todo");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        value(&fixture, cx, (0, 4)),
        Some(PropertyValue::Select(todo))
    );
    assert!(table.read_with(cx, |table, cx| !table.has_popup(cx)));

    select(&table, 1, 4, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Blocked");
    cx.simulate_keystrokes("enter");
    let options = options_of(&fixture, cx, status);
    assert_eq!(options.len(), 2);
    assert_eq!(
        value(&fixture, cx, (1, 4)),
        Some(PropertyValue::Select(options[1]))
    );
}

#[gpui::test]
fn the_date_picker_takes_typed_dates(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 5, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("9 Nov 2026");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        value(&fixture, cx, (0, 5)),
        Some(PropertyValue::Date(Date {
            year: 2026,
            month: 11,
            day: 9
        }))
    );
    assert!(table.read_with(cx, |table, cx| !table.has_popup(cx)));
}

#[gpui::test]
fn cells_copy_paste_and_clear(cx: &mut TestAppContext) {
    let fixture = fixture(3);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 2, cx);
    cx.simulate_keystrokes("7 enter cmd-c down cmd-v");
    assert_eq!(
        value(&fixture, cx, (1, 2)),
        Some(PropertyValue::Number(7.0))
    );

    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        "first\t1\nsecond\tnot a number".into(),
    ));
    select(&table, 1, 1, cx);
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(
        value(&fixture, cx, (1, 1)),
        Some(PropertyValue::Text("first".into()))
    );
    assert_eq!(
        value(&fixture, cx, (1, 2)),
        Some(PropertyValue::Number(1.0))
    );
    assert_eq!(
        value(&fixture, cx, (2, 1)),
        Some(PropertyValue::Text("second".into()))
    );
    assert_eq!(value(&fixture, cx, (2, 2)), None);
    assert!(table.read_with(cx, |table, _| table.error.is_some()));

    select(&table, 1, 1, cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(value(&fixture, cx, (1, 1)), None);
}

#[gpui::test]
fn rows_are_added_and_deleted(cx: &mut TestAppContext) {
    let fixture = fixture(3);
    let (table, cx) = open(&fixture, cx);
    table.update_in(cx, |table, window, cx| table.add_row(window, cx));
    cx.simulate_input("fresh");
    cx.simulate_keystrokes("enter");
    let rows = stored_row_titles(&fixture, cx);
    assert_eq!(rows.last().map(String::as_str), Some("fresh"));
    assert_eq!(table.read_with(cx, |table, _| table.row_count()), 4);

    select(&table, 0, 0, cx);
    cx.simulate_keystrokes("shift-space down shift-space cmd-backspace");
    assert_eq!(titles_shown(&table, cx), ["row 2", "fresh"]);
    assert_eq!(stored_row_titles(&fixture, cx), ["row 2", "fresh"]);
}

#[gpui::test]
fn sort_and_filter_are_saved_and_re_run_the_query(cx: &mut TestAppContext) {
    let fixture = fixture(3);
    let (table, cx) = open(&fixture, cx);
    let points = fixture.properties[1];
    for (row, number) in [(0, "5"), (1, "20"), (2, "10")] {
        select(&table, row, 2, cx);
        cx.simulate_keystrokes(&format!(
            "{} enter",
            number
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    table.update(cx, |table, cx| {
        table.change_sort(
            vec![SortKey {
                property: points,
                direction: SortDirection::Descending,
            }],
            cx,
        )
    });
    assert_eq!(titles_shown(&table, cx), ["row 1", "row 2", "row 0"]);

    table.update(cx, |table, cx| {
        table.change_filter(
            vec![FilterCondition {
                property: points,
                condition: Condition::NumberGreaterThan(8.0),
            }],
            cx,
        )
    });
    assert_eq!(titles_shown(&table, cx), ["row 1", "row 2"]);
    let saved = fixture.store.borrow().view(fixture.view).unwrap().unwrap();
    assert_eq!(saved.sort.len(), 1);
    assert_eq!(saved.filter.len(), 1);
}

#[gpui::test]
fn properties_are_added_renamed_retyped_reordered_hidden_and_deleted(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    let columns = |table: &Entity<TableView>, cx: &mut VisualTestContext| {
        table.read_with(cx, |table, _| table.columns.order())
    };

    table.update(cx, |table, cx| table.add_property(PropertyType::Url, cx));
    let added = *columns(&table, cx).last().unwrap();
    assert_eq!(columns(&table, cx).len(), 6);

    table.update_in(cx, |table, window, cx| {
        table.start_renaming(added, window, cx)
    });
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Website");
    cx.simulate_keystrokes("enter");
    let name = |cx: &mut VisualTestContext| {
        table.read_with(cx, |table, _| {
            table.model.property(added).unwrap().name.clone()
        })
    };
    assert_eq!(name(cx), "Website");

    table.update(cx, |table, cx| {
        table.change_type(added, PropertyType::Text, cx)
    });
    assert_eq!(
        table.read_with(cx, |table, _| table
            .model
            .property(added)
            .unwrap()
            .property_type),
        PropertyType::Text
    );

    let first = fixture.properties[0];
    table.update(cx, |table, cx| table.move_column(added, first, cx));
    assert_eq!(columns(&table, cx)[0], added);

    table.update(cx, |table, cx| table.hide_column(first, cx));
    assert!(!columns(&table, cx).contains(&first));

    table.update(cx, |table, cx| table.delete_property(added, cx));
    assert!(!columns(&table, cx).contains(&added));
    cx.run_until_parked();
    let view = fixture.store.borrow().view(fixture.view).unwrap().unwrap();
    assert_eq!(view.visible_columns, columns(&table, cx));
}

#[gpui::test]
fn a_resized_column_is_saved_when_the_drag_ends(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    let first = fixture.properties[0];
    cx.update(|_, cx| {
        table.update(cx, |table, cx| {
            table.pointer_drag = Some(super::PointerDrag::ResizeColumn {
                property: first,
                start_x: gpui::px(100.0),
                start_width: crate::sizes::DEFAULT_COLUMN_WIDTH,
            });
            table.drag_to(gpui::px(130.0), cx);
            table.drag_to(gpui::px(160.0), cx);
        });
    });
    cx.run_until_parked();
    // Nothing is saved while the drag is under way.
    let saved = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        fixture
            .store
            .borrow()
            .view(fixture.view)
            .unwrap()
            .unwrap()
            .column_widths
    };
    assert!(saved(cx).is_empty());

    table.update(cx, |table, cx| table.end_drag(cx));
    let expected = crate::sizes::DEFAULT_COLUMN_WIDTH + gpui::px(60.0);
    let width = table.read_with(cx, |table, _| table.columns.columns()[0].width);
    assert_eq!(width, expected);
    assert_eq!(
        saved(cx),
        vec![scroll_core::record::ColumnWidth {
            property: first,
            width: f32::from(expected) as u32
        }]
    );
}

#[gpui::test]
fn refresh_shows_changes_made_elsewhere(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    fixture
        .store
        .borrow_mut()
        .apply_local_transaction(&scroll_core::operation::Transaction::set_title(
            fixture.pages[0],
            "from elsewhere",
        ))
        .unwrap();
    table.update(cx, |table, cx| table.refresh(cx));
    assert_eq!(titles_shown(&table, cx)[0], "from elsewhere");
}

#[gpui::test]
fn open_row_reports_the_page(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorded = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&table, move |_, event: &crate::TableEvent, _| {
            recorded.borrow_mut().push(event.clone())
        })
        .detach()
    });
    select(&table, 1, 3, cx);
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        *opened.borrow(),
        [crate::TableEvent::OpenPage(fixture.pages[1])]
    );
}

#[gpui::test]
#[ignore = "timing; run with --release --ignored to measure"]
fn a_cell_edit_at_a_hundred_thousand_rows_is_quick(cx: &mut TestAppContext) {
    let started = Instant::now();
    let fixture = fixture(100_000);
    println!("building 100,000 rows: {:?}", started.elapsed());
    let started = Instant::now();
    let (table, cx) = open(&fixture, cx);
    println!("opening the table: {:?}", started.elapsed());
    select(&table, 50_000, 2, cx);
    let mut timings = Vec::new();
    for digit in 1..=20 {
        let started = Instant::now();
        cx.simulate_keystrokes(&format!("{} enter", digit % 10));
        timings.push(started.elapsed());
    }
    timings.sort();
    println!(
        "typing a number and pressing enter: median {:?}, max {:?}",
        timings[timings.len() / 2],
        timings.last().unwrap()
    );
    assert_eq!(
        value(&fixture, cx, (50_000, 2)),
        Some(PropertyValue::Number(0.0))
    );
}

fn options_of(
    fixture: &Fixture,
    cx: &mut VisualTestContext,
    property: PropertyId,
) -> Vec<SelectOptionId> {
    cx.run_until_parked();
    let store = fixture.store.borrow();
    let database = store.view(fixture.view).unwrap().unwrap().database;
    let schema = store.database(database).unwrap().unwrap();
    let property = schema
        .properties
        .iter()
        .find(|candidate| candidate.id == property)
        .unwrap();
    property.options.iter().map(|option| option.id).collect()
}

fn stored_row_titles(fixture: &Fixture, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    let store = fixture.store.borrow();
    let every_row_in_position_order = View {
        filter: Vec::new(),
        sort: Vec::new(),
        ..store.view(fixture.view).unwrap().unwrap()
    };
    let ids = store.rows_for_view(every_row_in_position_order).unwrap();
    store
        .table_rows(&ids.rows)
        .unwrap()
        .into_iter()
        .map(|row| row.page.title)
        .collect()
}

fn open_second_view(fixture: &Fixture, cx: &mut VisualTestContext) -> Entity<TableView> {
    let path = fixture.path.clone();
    let view = fixture.view;
    let table = cx.new(|cx| TableView::new(path, view, cx));
    cx.run_until_parked();
    table
}

#[gpui::test]
fn the_first_load_does_not_block_and_rows_follow(cx: &mut TestAppContext) {
    let fixture = fixture(3);
    let (_, cx) = open(&fixture, cx);
    let path = fixture.path.clone();
    let view = fixture.view;
    let table = cx.new(|cx| TableView::new(path, view, cx));
    let before = table.read_with(cx, |table, _| (table.is_loading(), table.row_count()));
    assert_eq!(before, (true, 0));
    cx.run_until_parked();
    let after = table.read_with(cx, |table, _| (table.is_loading(), table.row_count()));
    assert_eq!(after, (false, 3));
}

#[gpui::test]
fn two_views_of_one_database_see_each_other_after_refresh(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (first, cx) = open(&fixture, cx);
    let second = open_second_view(&fixture, cx);
    let changes = Rc::new(RefCell::new(0));
    let counted = changes.clone();
    cx.update(|_, cx| {
        cx.subscribe(&first, move |_, event: &crate::TableEvent, _| {
            if *event == crate::TableEvent::Changed {
                *counted.borrow_mut() += 1;
            }
        })
        .detach()
    });

    select(&first, 0, 0, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input(" edited");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*changes.borrow(), 1);
    assert_eq!(titles_shown(&second, cx)[0], "row 0");

    second.update(cx, |table, cx| table.refresh(cx));
    assert_eq!(titles_shown(&second, cx)[0], "row 0 edited");
}

#[gpui::test]
fn an_edit_is_not_saved_into_a_property_deleted_meanwhile(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    let notes = fixture.properties[0];
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("h i");

    // Another device deletes the property; the app hears and refreshes the table.
    let view = fixture.store.borrow().view(fixture.view).unwrap().unwrap();
    fixture
        .store
        .borrow_mut()
        .apply_local_transaction(&crate::transactions::delete_property(&view, notes))
        .unwrap();
    table.update(cx, |table, cx| table.refresh(cx));
    cx.run_until_parked();

    // The edited column is gone, so the edit ends without saving and says so.
    let state = table.read_with(cx, |table, _| {
        (table.text_edit.is_none(), table.error.is_some())
    });
    assert_eq!(state, (true, true));
    cx.simulate_keystrokes("enter");
    assert!(
        fixture
            .store
            .borrow()
            .property_values(fixture.pages[0])
            .unwrap()
            .is_empty()
    );
}

#[gpui::test]
fn an_edit_is_not_saved_into_a_property_retyped_meanwhile(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    let notes = fixture.properties[0];
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("4 2");

    let view = fixture.store.borrow().view(fixture.view).unwrap().unwrap();
    let database = fixture
        .store
        .borrow()
        .database(view.database)
        .unwrap()
        .unwrap();
    let property = database
        .properties
        .iter()
        .find(|property| property.id == notes)
        .unwrap()
        .clone();
    fixture
        .store
        .borrow_mut()
        .apply_local_transaction(&crate::transactions::change_property_type(
            &view,
            &property,
            PropertyType::Number,
        ))
        .unwrap();
    table.update(cx, |table, cx| table.refresh(cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("enter");
    assert_eq!(value(&fixture, cx, (0, 1)), None);
    assert!(table.read_with(cx, |table, _| table.error.is_some()));
}

#[gpui::test]
fn an_edited_row_moves_to_its_sorted_place_shortly_after_and_the_selection_follows(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(3);
    let (table, cx) = open(&fixture, cx);
    let points = fixture.properties[1];
    for (row, digit) in [(0, "1"), (1, "2"), (2, "3")] {
        select(&table, row, 2, cx);
        cx.simulate_keystrokes(&format!("{digit} enter"));
    }
    table.update(cx, |table, cx| {
        table.change_sort(
            vec![SortKey {
                property: points,
                direction: SortDirection::Ascending,
            }],
            cx,
        )
    });
    assert_eq!(titles_shown(&table, cx), ["row 0", "row 1", "row 2"]);

    select(&table, 0, 2, cx);
    cx.simulate_keystrokes("9 enter");
    // Right after the edit the row stays where it is.
    assert_eq!(titles_shown(&table, cx), ["row 0", "row 1", "row 2"]);

    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    assert_eq!(titles_shown(&table, cx), ["row 1", "row 2", "row 0"]);
    assert_eq!(selection(&table, cx), CellPosition { row: 2, column: 2 });
}

#[gpui::test]
fn a_block_of_cells_copies_as_tab_separated_text_and_clears_together(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    for (row, text) in [(0, "a"), (1, "b")] {
        select(&table, row, 1, cx);
        cx.simulate_keystrokes(&format!("{text} tab 5 enter"));
    }
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("shift-down shift-right cmd-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("a\t5\nb\t5"));

    cx.simulate_keystrokes("backspace");
    for cell in [(0, 1), (0, 2), (1, 1), (1, 2)] {
        assert_eq!(value(&fixture, cx, cell), None, "{cell:?}");
    }
    assert_eq!(title(&fixture, cx, 0), "row 0");
}

#[gpui::test]
fn tables_dropped_while_loading_leave_nothing_behind(cx: &mut TestAppContext) {
    let fixture = fixture(50);
    let (_, cx) = open(&fixture, cx);
    for _ in 0..20 {
        let path = fixture.path.clone();
        let view = fixture.view;
        let table = cx.new(|cx| TableView::new(path, view, cx));
        drop(table);
    }
    cx.run_until_parked();
}

#[gpui::test]
fn a_narrower_window_keeps_the_horizontal_scroll_in_range(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    cx.simulate_resize(gpui::size(gpui::px(600.0), gpui::px(400.0)));
    cx.run_until_parked();
    table.update(cx, |table, cx| {
        table.horizontal_offset = gpui::px(10_000.0);
        cx.notify();
    });
    cx.simulate_resize(gpui::size(gpui::px(2_000.0), gpui::px(400.0)));
    cx.run_until_parked();
    let offset = table.read_with(cx, |table, _| table.horizontal_offset);
    assert_eq!(offset, gpui::px(0.0));
}

fn loaded_title(
    table: &Entity<TableView>,
    row: usize,
    cx: &mut VisualTestContext,
) -> Option<String> {
    cx.run_until_parked();
    table.read_with(cx, |table, _| {
        table.model.row(row).map(|row| row.page.title.clone())
    })
}

#[gpui::test]
fn rows_far_down_load_when_revealed_and_only_a_bounded_number_stay_loaded(cx: &mut TestAppContext) {
    let fixture = fixture(3_000);
    let (table, cx) = open(&fixture, cx);
    assert_eq!(table.read_with(cx, |table, _| table.row_count()), 3_000);
    assert_eq!(loaded_title(&table, 0, cx).as_deref(), Some("row 0"));
    assert_eq!(loaded_title(&table, 2_500, cx), None);

    for row in [2_500, 1_200, 2_999, 40] {
        table.update(cx, |table, cx| table.reveal_row(row, cx));
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(600.0)));
        assert_eq!(loaded_title(&table, row, cx), Some(format!("row {row}")));
        let loaded = table.read_with(cx, |table, _| {
            (0..table.row_count())
                .filter(|index| table.model.row(*index).is_some())
                .count()
        });
        assert!(
            loaded <= crate::table_model::MAX_LOADED_ROWS,
            "{loaded} rows loaded"
        );
    }
}

#[gpui::test]
fn a_block_copied_across_rows_not_loaded_copies_every_row(cx: &mut TestAppContext) {
    let fixture = fixture(1_500);
    let (table, cx) = open(&fixture, cx);
    table.update(cx, |table, cx| {
        table.select(CellPosition { row: 0, column: 0 }, cx);
        table.extend_to(
            CellPosition {
                row: 1_499,
                column: 0,
            },
            cx,
        );
    });
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!(lines.len(), 1_500);
    assert_eq!(lines[1_234], "row 1234");
}

#[gpui::test]
fn emoji_are_edited_copied_and_named_as_single_characters(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    let family = "👨‍👩‍👧";

    // One left arrow steps over the whole family emoji, not one of its code points.
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input(&format!("a{family}"));
    cx.simulate_keystrokes("left");
    cx.simulate_input("b");
    cx.simulate_keystrokes("enter");
    let text = format!("ab{family}");
    assert_eq!(
        value(&fixture, cx, (0, 1)),
        Some(PropertyValue::Text(text.clone()))
    );

    select(&table, 0, 4, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("🚀 Launch");
    cx.simulate_keystrokes("enter");
    let status = fixture.properties[3];
    let options = options_of(&fixture, cx, status);
    assert_eq!(options.len(), 2);

    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("shift-right shift-right shift-right cmd-c");
    cx.run_until_parked();
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, Some(format!("{text}\t\t\t🚀 Launch")));

    select(&table, 1, 1, cx);
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(value(&fixture, cx, (1, 1)), Some(PropertyValue::Text(text)));
    assert_eq!(
        value(&fixture, cx, (1, 4)),
        Some(PropertyValue::Select(options[1]))
    );
}

#[gpui::test]
fn a_read_only_table_writes_nothing_but_sorts_for_the_session(cx: &mut TestAppContext) {
    let fixture = fixture(3);
    let (table, cx) = open(&fixture, cx);
    table.update_in(cx, |table, window, cx| {
        table.set_access(crate::TableAccess::ReadOnly, window, cx)
    });

    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("h i enter");
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    cx.simulate_keystrokes("cmd-v backspace");
    select(&table, 0, 0, cx);
    cx.simulate_keystrokes("shift-space cmd-backspace");
    table.update_in(cx, |table, window, cx| table.add_row(window, cx));
    cx.run_until_parked();
    assert_eq!(value(&fixture, cx, (0, 1)), None);
    assert_eq!(stored_row_titles(&fixture, cx), ["row 0", "row 1", "row 2"]);
    assert!(table.read_with(cx, |table, _| table.text_edit.is_none()));

    // Sorting shows the rows sorted, but the stored view keeps no sort.
    table.update(cx, |table, cx| {
        let title_sorted = vec![SortKey {
            property: fixture.properties[0],
            direction: SortDirection::Descending,
        }];
        table.change_sort(title_sorted, cx)
    });
    cx.run_until_parked();
    let stored = fixture.store.borrow().view(fixture.view).unwrap().unwrap();
    assert!(stored.sort.is_empty());
    assert_eq!(
        table.read_with(cx, |table, _| table.model.view().sort.len()),
        1
    );
    table.update(cx, |table, cx| table.refresh(cx));
    assert_eq!(
        table.read_with(cx, |table, _| table.model.view().sort.len()),
        1
    );

    // Becoming editable again shows the stored view.
    table.update_in(cx, |table, window, cx| {
        table.set_access(crate::TableAccess::Editable, window, cx)
    });
    cx.run_until_parked();
    assert!(table.read_with(cx, |table, _| table.model.view().sort.is_empty()));
}

#[gpui::test]
fn a_read_only_table_refuses_every_schema_edit(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    table.update_in(cx, |table, window, cx| {
        table.set_access(crate::TableAccess::ReadOnly, window, cx)
    });
    let stored_properties = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        let store = fixture.store.borrow();
        let view = store.view(fixture.view).unwrap().unwrap();
        store.database(view.database).unwrap().unwrap().properties
    };
    let shown =
        |cx: &mut VisualTestContext| table.read_with(cx, |table, _| table.model.view().clone());
    let properties_before = stored_properties(cx);
    let view_before = shown(cx);

    let first = fixture.properties[0];
    let schema_edits = [
        Command::AddProperty(PropertyType::Url),
        Command::ChangePropertyType {
            property: first,
            property_type: PropertyType::Number,
        },
        Command::DeleteProperty(first),
        Command::RenameProperty(first),
    ];
    for command in schema_edits {
        table.update_in(cx, |table, window, cx| table.perform(command, window, cx));
    }

    assert_eq!(stored_properties(cx), properties_before);
    assert_eq!(shown(cx), view_before);
    assert!(table.read_with(cx, |table, _| table.text_edit.is_none()));
}

#[gpui::test]
fn becoming_read_only_ends_an_edit_without_saving_it(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 1, cx);
    cx.simulate_keystrokes("h i");
    table.update_in(cx, |table, window, cx| {
        table.set_access(crate::TableAccess::ReadOnly, window, cx)
    });
    cx.simulate_keystrokes("enter");
    assert!(table.read_with(cx, |table, _| table.text_edit.is_none()));
    assert_eq!(value(&fixture, cx, (0, 1)), None);
}

#[gpui::test]
fn add_row_scrolls_to_the_new_row_and_edits_its_title(cx: &mut TestAppContext) {
    let fixture = fixture(500);
    let (table, cx) = open(&fixture, cx);
    table.update_in(cx, |table, window, cx| table.add_row(window, cx));
    cx.simulate_input("last");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        selection(&table, cx),
        CellPosition {
            row: 500,
            column: 0
        }
    );
    assert_eq!(
        stored_row_titles(&fixture, cx).last().map(String::as_str),
        Some("last")
    );
}

#[gpui::test]
fn escape_keeps_a_new_row_s_title_but_discards_other_edits(cx: &mut TestAppContext) {
    let fixture = fixture(1);
    let (table, cx) = open(&fixture, cx);
    table.update_in(cx, |table, window, cx| table.add_row(window, cx));
    cx.simulate_input("kept");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        stored_row_titles(&fixture, cx).last().map(String::as_str),
        Some("kept")
    );
    assert!(table.read_with(cx, |table, _| table.text_edit.is_none()));

    // Editing the same title again is an ordinary edit: Escape discards it.
    select(&table, 1, 0, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input(" more");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        stored_row_titles(&fixture, cx).last().map(String::as_str),
        Some("kept")
    );
}

#[gpui::test]
fn a_click_on_empty_space_lets_go_of_the_selected_cell(cx: &mut TestAppContext) {
    let fixture = fixture(2);
    let (table, cx) = open(&fixture, cx);
    select(&table, 0, 1, cx);
    cx.run_until_parked();

    // Well below the two rows.
    cx.simulate_click(
        gpui::point(gpui::px(400.0), gpui::px(500.0)),
        gpui::Modifiers::none(),
    );
    assert!(!table.read_with(cx, |table, _| table.is_selection_shown));
    cx.simulate_keystrokes("h i");
    assert!(
        table.read_with(cx, |table, _| table.text_edit.is_none()),
        "typing no longer edits the cell that was selected"
    );

    select(&table, 1, 1, cx);
    assert!(table.read_with(cx, |table, _| table.is_selection_shown));
}
