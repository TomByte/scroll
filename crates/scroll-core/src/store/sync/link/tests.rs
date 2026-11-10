use super::*;
use crate::id::{DatabaseId, PageId};
use crate::id::{PropertyId, ViewId};
use crate::operation::PageField;
use crate::operation::{FieldChange, Operation, Record, Transaction};
use crate::page_doc::{BlockKind, BlockPlacement, PageDoc};
use crate::position::Position;
use crate::record::{Database, Page, Property, PropertyType, PropertyValue, View, ViewType};
use crate::store::QueuedChange;
use crate::store::encode;
use crate::store::open_in_memory;
use crate::store::sync::publish::{PUBLISH_CHUNK_BYTES, PUBLISH_CHUNK_OPERATIONS};
use crate::store::test_support;
use crate::sync::WorkspaceSnapshot;

fn page(parent: Option<PageId>, row_of: Option<DatabaseId>) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: "Page".into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of,
        is_archived: false,
    }
}

/// A page tree with a child, a database on the child, a row with a value, a view, and content.
fn populated_local_store() -> (WorkspaceStore, PageId, Vec<u8>) {
    let mut store = open_in_memory();
    let top = page(None, None);
    let host = page(Some(top.id), None);
    let property = Property {
        id: PropertyId::generate(),
        name: "Done".into(),
        property_type: PropertyType::Checkbox,
        options: vec![],
    };
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![property.clone()],
    };
    let row = page(Some(host.id), Some(database.id));
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: vec![],
        sort: vec![],
        visible_columns: vec![property.id],
        column_widths: Vec::new(),
    };
    let operations = vec![
        Operation::Create(Record::Page(top.clone())),
        Operation::Create(Record::Page(host.clone())),
        Operation::Create(Record::Database(database)),
        Operation::Create(Record::Page(row.clone())),
        Operation::Create(Record::View(view)),
        Operation::Set(FieldChange::PropertyValue {
            page: row.id,
            property: property.id,
            value: Some(PropertyValue::Checkbox(true)),
        }),
    ];
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();

    let mut doc = PageDoc::new();
    let block = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    doc.insert_text(block, 0, "published text").unwrap();
    let update = doc.take_local_update().unwrap().unwrap();
    store.record_local_page_update(top.id, &update).unwrap();
    (store, top.id, update)
}

#[test]
fn a_new_workspace_is_local_only_until_linked_once() {
    let mut store = open_in_memory();
    assert_eq!(store.link().unwrap(), WorkspaceLink::LocalOnly);
    store.set_workspace_name("Team notes").unwrap();
    assert_eq!(store.workspace_name().unwrap(), "Team notes");

    let workspace_id = WorkspaceId::generate();
    store
        .link_to_server("https://scroll.test", workspace_id)
        .unwrap();
    assert_eq!(
        store.link().unwrap(),
        WorkspaceLink::Linked {
            server_url: "https://scroll.test".into(),
            workspace_id
        }
    );
    assert!(matches!(
        store.link_to_server("https://other.test", WorkspaceId::generate()),
        Err(StoreError::AlreadyLinked)
    ));
    // An empty workspace has nothing to publish.
    assert!(test_support::queued_changes(&store).unwrap().is_empty());
}

#[test]
fn linking_queues_everything_a_server_needs_to_rebuild_the_workspace() {
    let (mut store, page_with_content, _) = populated_local_store();
    let before = store.export_workspace_snapshot().unwrap();
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();

    let queued = test_support::queued_changes(&store).unwrap();
    let [
        QueuedChange::Transaction(publish),
        QueuedChange::PageUpdate(content),
    ] = &queued[..]
    else {
        panic!("expected one transaction and one page update, got {queued:?}");
    };
    assert_eq!(content.page_id, page_with_content);

    // A server running the same validation accepts the transaction and ends up with the same records.
    let mut server = open_in_memory();
    server.apply_local_transaction(publish).unwrap();
    let mut published = server.export_workspace_snapshot().unwrap();
    let mut local = before.clone();
    for snapshot in [&mut published, &mut local] {
        snapshot
            .records
            .sort_by_key(|record| format!("{:?}", record.id()));
        snapshot.seq = 0;
    }
    assert_eq!(published, local);

    let server_doc = PageDoc::new();
    server_doc.import(&content.update).unwrap();
    assert_eq!(server_doc.plain_text().unwrap(), "published text");
}

#[test]
fn after_linking_local_changes_are_queued_and_content_still_loads() {
    let (mut store, page_with_content, _) = populated_local_store();
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();
    assert_eq!(store.last_seen_workspace_seq().unwrap(), 0);
    assert_eq!(store.last_seen_page_seq(page_with_content).unwrap(), 0);
    assert_eq!(
        store
            .load_page_doc(page_with_content)
            .unwrap()
            .plain_text()
            .unwrap(),
        "published text"
    );

    let queued_before = test_support::queued_changes(&store).unwrap().len();
    let rename = FieldChange::Page {
        id: page_with_content,
        field: PageField::Title("Renamed".into()),
    };
    store
        .apply_local_transaction(&Transaction::new(vec![Operation::Set(rename)]))
        .unwrap();
    store
        .record_local_page_update(page_with_content, b"more")
        .unwrap();
    assert_eq!(
        test_support::queued_changes(&store).unwrap().len(),
        queued_before + 2
    );
}

fn snapshot_records(store: &WorkspaceStore) -> Vec<String> {
    let mut records: Vec<String> = store
        .export_workspace_snapshot()
        .unwrap()
        .records
        .iter()
        .map(|record| format!("{record:?}"))
        .collect();
    records.sort();
    records
}

#[test]
fn published_records_survive_the_new_workspace_s_empty_snapshot() {
    let (mut store, page_with_content, _) = populated_local_store();
    let before = snapshot_records(&store);
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();
    let queued = test_support::queued_changes(&store).unwrap();

    let empty = WorkspaceSnapshot {
        seq: 0,
        records: Vec::new(),
        property_values: Vec::new(),
    };
    test_support::apply_snapshot(&mut store, empty).unwrap();

    assert_eq!(snapshot_records(&store), before);
    assert_eq!(test_support::queued_changes(&store).unwrap(), queued);
    assert_eq!(
        store
            .load_page_doc(page_with_content)
            .unwrap()
            .plain_text()
            .unwrap(),
        "published text"
    );
}

/// A workspace whose publishing needs several transactions: many rows, and titles big enough
/// that the byte limit also comes into play.
fn large_local_store() -> WorkspaceStore {
    let mut store = open_in_memory();
    let host = page(None, None);
    let property = Property {
        id: PropertyId::generate(),
        name: "Points".into(),
        property_type: PropertyType::Number,
        options: vec![],
    };
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![property.clone()],
    };
    let mut operations = vec![
        Operation::Create(Record::Page(host.clone())),
        Operation::Create(Record::Database(database.clone())),
    ];
    for index in 0..1_500 {
        let row = Page {
            title: "x".repeat(if index % 100 == 0 { 300_000 } else { 10 }),
            ..page(Some(host.id), Some(database.id))
        };
        let row_id = row.id;
        operations.push(Operation::Create(Record::Page(row)));
        operations.push(Operation::Set(FieldChange::PropertyValue {
            page: row_id,
            property: property.id,
            value: Some(PropertyValue::Number(index as f64)),
        }));
    }
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    store
}

#[test]
fn pages_are_published_linked_in_tree_order_and_values_in_page_order() {
    let mut store = open_in_memory();
    let positions = Position::many_between(None, None, 3).unwrap();
    // A row filed under a page that comes before its database's host page, and a page
    // under a row: both have to wait for what they depend on.
    let early = Page {
        position: positions[0].clone(),
        ..page(None, None)
    };
    let host = Page {
        position: positions[2].clone(),
        ..page(None, None)
    };
    let points = Property {
        id: PropertyId::generate(),
        name: "Points".into(),
        property_type: PropertyType::Number,
        options: vec![],
    };
    let notes = Property {
        id: PropertyId::generate(),
        name: "Notes".into(),
        property_type: PropertyType::Text,
        options: vec![],
    };
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![points.clone(), notes.clone()],
    };
    let early_row = page(Some(early.id), Some(database.id));
    let host_row = page(Some(host.id), Some(database.id));
    let under_row = page(Some(early_row.id), None);
    let mut operations: Vec<Operation> = [
        Record::Page(early.clone()),
        Record::Page(host.clone()),
        Record::Database(database.clone()),
        Record::Page(early_row.clone()),
        Record::Page(host_row.clone()),
        Record::Page(under_row.clone()),
    ]
    .into_iter()
    .map(Operation::Create)
    .collect();
    for row in [&early_row, &host_row] {
        for (property, value) in [
            (notes.id, PropertyValue::Text("n".into())),
            (points.id, PropertyValue::Number(1.0)),
        ] {
            operations.push(Operation::Set(FieldChange::PropertyValue {
                page: row.id,
                property,
                value: Some(value),
            }));
        }
    }
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    let before = snapshot_records(&store);
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();

    let published: Vec<Operation> = test_support::queued_changes(&store)
        .unwrap()
        .into_iter()
        .filter_map(|change| match change {
            QueuedChange::Transaction(transaction) => Some(transaction.operations),
            QueuedChange::PageUpdate(_) => None,
        })
        .flatten()
        .collect();
    assert!(
        published
            .iter()
            .all(|operation| !matches!(operation, Operation::Set(FieldChange::Page { .. })))
    );
    let value_keys: Vec<(PageId, PropertyId)> = published
        .iter()
        .filter_map(|operation| match operation {
            Operation::Set(FieldChange::PropertyValue { page, property, .. }) => {
                Some((*page, *property))
            }
            _ => None,
        })
        .collect();
    let mut sorted = value_keys.clone();
    sorted.sort();
    assert_eq!(value_keys, sorted);

    let mut server = open_in_memory();
    server
        .apply_local_transaction(&Transaction::new(published))
        .unwrap();
    assert_eq!(snapshot_records(&server), before);
}

#[test]
fn publishing_is_split_into_bounded_transactions_that_each_validate_in_turn() {
    let mut store = large_local_store();
    let before = snapshot_records(&store);
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();
    let chunks: Vec<Transaction> = test_support::queued_changes(&store)
        .unwrap()
        .into_iter()
        .filter_map(|change| match change {
            QueuedChange::Transaction(transaction) => Some(transaction),
            QueuedChange::PageUpdate(_) => None,
        })
        .collect();
    assert!(
        chunks.len() > 2,
        "expected several chunks, got {}",
        chunks.len()
    );

    let mut server = open_in_memory();
    for chunk in &chunks {
        assert!(chunk.operations.len() <= PUBLISH_CHUNK_OPERATIONS);
        let largest_operation = chunk
            .operations
            .iter()
            .map(|operation| encode(operation).unwrap().len())
            .max()
            .unwrap_or(0);
        let chunk_bytes = encode(chunk).unwrap().len();
        assert!(
            chunk_bytes <= PUBLISH_CHUNK_BYTES.max(largest_operation) + 64,
            "chunk of {chunk_bytes} bytes"
        );
        server.apply_local_transaction(chunk).unwrap();
    }
    assert_eq!(snapshot_records(&server), before);
}

#[test]
fn acknowledged_publishing_makes_the_records_confirmed() {
    let (mut store, _, _) = populated_local_store();
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();
    let mut seq = 0;
    for change in test_support::queued_changes(&store).unwrap() {
        if let QueuedChange::Transaction(transaction) = change {
            seq += 1;
            store
                .acknowledge_transactions(&[(transaction.id, seq)])
                .unwrap();
        }
    }
    // Confirmed now: a snapshot from the server is the truth, even an empty one.
    let empty = WorkspaceSnapshot {
        seq,
        records: Vec::new(),
        property_values: Vec::new(),
    };
    test_support::apply_snapshot(&mut store, empty).unwrap();
    assert!(snapshot_records(&store).is_empty());
}
