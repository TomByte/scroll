use super::*;
use scroll_core::operation::{DatabaseField, PageField, ViewField};
use scroll_core::record::{Page, PropertyType, SortDirection, SortKey};

fn model_with_row() -> (TableModel, PageId, Property) {
    let mut model = TableModel::placeholder(ViewId::generate());
    let property = Property {
        id: PropertyId::generate(),
        name: "Points".into(),
        property_type: PropertyType::Number,
        options: vec![],
    };
    model.database.properties.push(property.clone());
    let page = Page {
        id: PageId::generate(),
        parent: None,
        title: "Row".into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: Some(model.database.id),
        is_archived: false,
    };
    let id = page.id;
    model.apply(&Transaction::new(vec![Operation::Create(Record::Page(
        page,
    ))]));
    (model, id, property)
}

#[test]
fn applied_changes_show_at_once() {
    let (mut model, page, property) = model_with_row();
    let renamed = Property {
        name: "Score".into(),
        ..property.clone()
    };
    let view = model.view.id;
    let database = model.database.id;
    model.apply(&Transaction::new(vec![
        Operation::Set(FieldChange::PropertyValue {
            page,
            property: property.id,
            value: Some(PropertyValue::Number(3.0)),
        }),
        Operation::Set(FieldChange::Page {
            id: page,
            field: PageField::Title("Renamed".into()),
        }),
        Operation::Set(FieldChange::Database {
            id: database,
            field: DatabaseField::Property(renamed),
        }),
        Operation::Set(FieldChange::View {
            id: view,
            field: ViewField::VisibleColumns(vec![property.id]),
        }),
    ]));
    assert_eq!(model.row(0).unwrap().page.title, "Renamed");
    assert_eq!(
        model.value(0, model.property(property.id).unwrap()),
        Some(&PropertyValue::Number(3.0))
    );
    assert_eq!(model.visible_properties()[0].name, "Score");

    model.apply(&Transaction::new(vec![Operation::Delete(RecordId::Page(
        page,
    ))]));
    assert_eq!(model.row_count(), 0);
}

fn row(title: &str) -> TableRow {
    TableRow {
        page: Page {
            id: PageId::generate(),
            parent: None,
            title: title.into(),
            icon: None,
            position: Position::between(None, None).unwrap(),
            row_of: None,
            is_archived: false,
        },
        values: Vec::new(),
    }
}

#[test]
fn rows_load_by_window_and_the_farthest_are_dropped_beyond_the_limit() {
    let mut model = TableModel::placeholder(ViewId::generate());
    let rows: Vec<TableRow> = (0..MAX_LOADED_ROWS + 50)
        .map(|index| row(&index.to_string()))
        .collect();
    model.order = rows.iter().map(|row| row.page.id).collect();
    assert_eq!(model.missing_rows(10..13).len(), 3);
    assert!(model.row(10).is_none());

    let loaded: Vec<(usize, TableRow)> = rows.iter().cloned().enumerate().collect();
    model.insert_rows(loaded, 0..10);
    assert_eq!(model.loaded.len(), MAX_LOADED_ROWS);
    assert_eq!(model.row(0).unwrap().page.title, "0");
    assert!(model.row(MAX_LOADED_ROWS + 49).is_none());
    assert_eq!(model.row_count(), MAX_LOADED_ROWS + 50);
    assert!(model.missing_rows(0..10).is_empty());

    // A row already loaded keeps its contents, which may hold an edit newer than the read.
    let mut stale = rows[0].clone();
    stale.page.title = "stale".into();
    model.insert_rows(vec![(0, stale)], 0..10);
    assert_eq!(model.row(0).unwrap().page.title, "0");
}

#[test]
fn deleted_rows_leave_the_order_and_new_rows_go_last() {
    let (mut model, page, _) = model_with_row();
    let second = row("Second");
    let second_page = Page {
        row_of: Some(model.database.id),
        ..second.page
    };
    let second_id = second_page.id;
    model.apply(&Transaction::new(vec![Operation::Create(Record::Page(
        second_page.clone(),
    ))]));
    assert_eq!(model.row_ids(0..10), [page, second_id]);
    assert_eq!(model.last_position(), Some(&second_page.position));
    assert_eq!(model.row_index(second_id), Some(1));

    model.apply(&Transaction::new(vec![Operation::Delete(RecordId::Page(
        page,
    ))]));
    assert_eq!(model.row_ids(0..10), [second_id]);
    let wanted: HashSet<PageId> = [page, second_id].into();
    assert_eq!(model.rows_among(&wanted), [second_id].into());
}

#[test]
#[ignore = "timing; run with --release --ignored to measure"]
fn applying_large_changes_to_a_million_row_order_is_quick() {
    let mut model = TableModel::placeholder(ViewId::generate());
    model.order = (0..1_000_000).map(|_| PageId::generate()).collect();
    let doomed: Vec<Operation> = model.order[..10_000]
        .iter()
        .map(|page| Operation::Delete(RecordId::Page(*page)))
        .collect();
    let delete = Transaction::new(doomed);
    let started = std::time::Instant::now();
    model.apply(&delete);
    println!(
        "deleting 10,000 of 1,000,000 rows on screen: {:?}",
        started.elapsed()
    );
    let last = *model.order.last().unwrap();
    let started = std::time::Instant::now();
    assert_eq!(model.row_index(last), Some(989_999));
    println!("finding the last row's place: {:?}", started.elapsed());
    assert_eq!(model.row_count(), 990_000);
}

#[test]
fn only_sorted_or_filtered_values_move_rows() {
    let (mut model, page, property) = model_with_row();
    let set_points = Transaction::new(vec![Operation::Set(FieldChange::PropertyValue {
        page,
        property: property.id,
        value: None,
    })]);
    assert!(!changes_row_placement(
        &model.view,
        &model.database,
        &set_points
    ));
    model.view.sort.push(SortKey {
        property: property.id,
        direction: SortDirection::Ascending,
    });
    assert!(changes_row_placement(
        &model.view,
        &model.database,
        &set_points
    ));
}
