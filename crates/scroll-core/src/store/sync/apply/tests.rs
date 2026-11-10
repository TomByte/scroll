use crate::id::{
    DatabaseId, PageId, PropertyId, SelectOptionId, TransactionId, ViewId, WorkspaceId,
};
use crate::operation::{
    DatabaseField, FieldChange, Operation, PageField, Record, RecordId, Transaction, ViewField,
};
use crate::page_doc::{BlockKind, BlockPlacement, PageDoc};
use crate::position::Position;
use crate::record::{
    Database, Page, Property, PropertyType, PropertyValue, SelectOption, View, ViewType,
};
use crate::store::test_support;
use crate::store::{
    QueuedChange, SequencedBytes, StoreError, WorkspaceLink, WorkspaceStore, open_in_memory,
    open_linked_in_memory,
};
use crate::validation::ValidationError;

fn new_page(parent: Option<PageId>) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: "Untitled".into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    }
}

fn create(store: &mut WorkspaceStore, record: Record) {
    store
        .apply_local_transaction(&Transaction::new(vec![Operation::Create(record)]))
        .unwrap();
}

fn create_page(store: &mut WorkspaceStore, parent: Option<PageId>) -> PageId {
    let page = new_page(parent);
    let id = page.id;
    create(store, Record::Page(page));
    id
}

fn set_title(page: PageId, title: &str) -> Operation {
    Operation::Set(FieldChange::Page {
        id: page,
        field: PageField::Title(title.into()),
    })
}

fn set_parent(page: PageId, parent: PageId) -> Operation {
    Operation::Set(FieldChange::Page {
        id: page,
        field: PageField::Parent(Some(parent)),
    })
}

fn title_of(store: &WorkspaceStore, page: PageId) -> String {
    store.page(page).unwrap().unwrap().title
}

fn validation_error<T: std::fmt::Debug>(result: Result<T, StoreError>) -> ValidationError {
    match result {
        Err(StoreError::Validation(error)) => error,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// A database with one select property, and one row page in it.
struct RowFixture {
    database: DatabaseId,
    property: PropertyId,
    option: SelectOptionId,
    row: PageId,
}

fn row_fixture(store: &mut WorkspaceStore) -> RowFixture {
    let (fixture, operations) = row_fixture_operations();
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    fixture
}

fn row_fixture_operations() -> (RowFixture, Vec<Operation>) {
    let option = SelectOptionId::generate();
    let property = Property {
        id: PropertyId::generate(),
        name: "Status".into(),
        property_type: PropertyType::Select,
        options: vec![SelectOption {
            id: option,
            name: "Done".into(),
        }],
    };
    let host = new_page(None);
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![property.clone()],
    };
    let row = Page {
        row_of: Some(database.id),
        ..new_page(None)
    };
    let fixture = RowFixture {
        database: database.id,
        property: property.id,
        option,
        row: row.id,
    };
    let operations = vec![
        Operation::Create(Record::Page(host)),
        Operation::Create(Record::Database(database)),
        Operation::Create(Record::Page(row)),
    ];
    (fixture, operations)
}

// Validation through the store.

#[test]
fn a_failing_operation_rolls_back_the_whole_transaction() {
    let mut store = open_in_memory();
    let page = new_page(None);
    let page_id = page.id;
    let missing = PageId::generate();
    let transaction = Transaction::new(vec![
        Operation::Create(Record::Page(page)),
        set_title(missing, "never written"),
    ]);

    assert_eq!(
        validation_error(store.apply_local_transaction(&transaction)),
        ValidationError::PageNotFound(missing)
    );
    assert_eq!(store.page(page_id).unwrap(), None);
}

#[test]
fn later_operations_see_earlier_ones_in_the_same_transaction() {
    let mut store = open_in_memory();
    let parent = new_page(None);
    let child = new_page(Some(parent.id));
    let (parent_id, child_id) = (parent.id, child.id);
    store
        .apply_local_transaction(&Transaction::new(vec![
            Operation::Create(Record::Page(parent)),
            Operation::Create(Record::Page(child)),
            set_title(child_id, "Child"),
        ]))
        .unwrap();
    assert_eq!(
        test_support::child_pages(&store, Some(parent_id)).unwrap()[0].title,
        "Child"
    );

    // Deleting the parent first fails; deleting the child first, in the same transaction, succeeds.
    let wrong_order = Transaction::new(vec![Operation::Delete(RecordId::Page(parent_id))]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&wrong_order)),
        ValidationError::PageHasChildren(parent_id)
    );
    store
        .apply_local_transaction(&Transaction::new(vec![
            Operation::Delete(RecordId::Page(child_id)),
            Operation::Delete(RecordId::Page(parent_id)),
        ]))
        .unwrap();
    assert_eq!(store.page(parent_id).unwrap(), None);
}

#[test]
fn a_page_cannot_become_its_own_ancestor() {
    let mut store = open_in_memory();
    let top = create_page(&mut store, None);
    let middle = create_page(&mut store, Some(top));
    let bottom = create_page(&mut store, Some(middle));

    let error = validation_error(
        store.apply_local_transaction(&Transaction::new(vec![set_parent(top, bottom)])),
    );
    assert_eq!(
        error,
        ValidationError::PageWouldBeItsOwnAncestor {
            page: top,
            parent: bottom
        }
    );
    let error = validation_error(
        store.apply_local_transaction(&Transaction::new(vec![set_parent(top, top)])),
    );
    assert_eq!(
        error,
        ValidationError::PageWouldBeItsOwnAncestor {
            page: top,
            parent: top
        }
    );

    store
        .apply_local_transaction(&Transaction::new(vec![set_parent(bottom, top)]))
        .unwrap();
    assert_eq!(store.page(bottom).unwrap().unwrap().parent, Some(top));
}

#[test]
fn referenced_records_must_exist() {
    let mut store = open_in_memory();
    let missing_page = PageId::generate();
    let missing_database = DatabaseId::generate();

    let orphan = Page {
        parent: Some(missing_page),
        ..new_page(None)
    };
    let result = store.apply_local_transaction(&Transaction::new(vec![Operation::Create(
        Record::Page(orphan),
    )]));
    assert_eq!(
        validation_error(result),
        ValidationError::PageNotFound(missing_page)
    );

    let row = Page {
        row_of: Some(missing_database),
        ..new_page(None)
    };
    let result = store.apply_local_transaction(&Transaction::new(vec![Operation::Create(
        Record::Page(row),
    )]));
    assert_eq!(
        validation_error(result),
        ValidationError::DatabaseNotFound(missing_database)
    );

    let view = View {
        id: ViewId::generate(),
        name: "All".into(),
        database: missing_database,
        view_type: ViewType::Table,
        filter: vec![],
        sort: vec![],
        visible_columns: vec![],
        column_widths: Vec::new(),
    };
    let result = store.apply_local_transaction(&Transaction::new(vec![Operation::Create(
        Record::View(view),
    )]));
    assert_eq!(
        validation_error(result),
        ValidationError::DatabaseNotFound(missing_database)
    );
}

#[test]
fn creating_an_existing_record_is_rejected() {
    let mut store = open_in_memory();
    let page = new_page(None);
    create(&mut store, Record::Page(page.clone()));
    let result = store.apply_local_transaction(&Transaction::new(vec![Operation::Create(
        Record::Page(page.clone()),
    )]));
    assert_eq!(
        validation_error(result),
        ValidationError::RecordAlreadyExists(RecordId::Page(page.id))
    );
}

#[test]
fn malformed_positions_are_rejected() {
    let mut store = open_in_memory();
    let page = create_page(&mut store, None);
    let bad: Position = postcard::from_bytes(&postcard::to_stdvec("not hex").unwrap()).unwrap();
    let operation = Operation::Set(FieldChange::Page {
        id: page,
        field: PageField::Position(bad.clone()),
    });
    let result = store.apply_local_transaction(&Transaction::new(vec![operation]));
    assert_eq!(
        validation_error(result),
        ValidationError::MalformedPosition(bad)
    );
}

#[test]
fn property_values_must_fit_the_row_database_schema() {
    let mut store = open_in_memory();
    let fixture = row_fixture(&mut store);
    let set_value = |page, property, value| {
        Transaction::new(vec![Operation::Set(FieldChange::PropertyValue {
            page,
            property,
            value: Some(value),
        })])
    };

    let wrong_type = set_value(fixture.row, fixture.property, PropertyValue::Checkbox(true));
    assert!(matches!(
        validation_error(store.apply_local_transaction(&wrong_type)),
        ValidationError::PropertyTypeMismatch {
            expected: PropertyType::Select,
            actual: PropertyType::Checkbox,
            ..
        }
    ));

    let unknown_property = PropertyId::generate();
    let missing = set_value(
        fixture.row,
        unknown_property,
        PropertyValue::Select(fixture.option),
    );
    assert_eq!(
        validation_error(store.apply_local_transaction(&missing)),
        ValidationError::PropertyNotFound {
            database: fixture.database,
            property: unknown_property
        }
    );

    let plain_page = create_page(&mut store, None);
    let not_a_row = set_value(
        plain_page,
        fixture.property,
        PropertyValue::Select(fixture.option),
    );
    assert_eq!(
        validation_error(store.apply_local_transaction(&not_a_row)),
        ValidationError::PageIsNotADatabaseRow(plain_page)
    );

    store
        .apply_local_transaction(&set_value(
            fixture.row,
            fixture.property,
            PropertyValue::Select(fixture.option),
        ))
        .unwrap();
    assert_eq!(
        store.property_values(fixture.row).unwrap(),
        vec![(fixture.property, PropertyValue::Select(fixture.option))]
    );

    let clear = Operation::Set(FieldChange::PropertyValue {
        page: fixture.row,
        property: fixture.property,
        value: None,
    });
    store
        .apply_local_transaction(&Transaction::new(vec![clear]))
        .unwrap();
    assert!(store.property_values(fixture.row).unwrap().is_empty());
}

#[test]
fn views_may_only_mention_properties_of_their_database() {
    let mut store = open_in_memory();
    let fixture = row_fixture(&mut store);
    let view = View {
        id: ViewId::generate(),
        name: "All".into(),
        database: fixture.database,
        view_type: ViewType::Table,
        filter: vec![],
        sort: vec![],
        visible_columns: vec![fixture.property],
        column_widths: Vec::new(),
    };
    let view_id = view.id;
    create(&mut store, Record::View(view));

    let unknown = PropertyId::generate();
    let operation = Operation::Set(FieldChange::View {
        id: view_id,
        field: ViewField::VisibleColumns(vec![unknown]),
    });
    assert_eq!(
        validation_error(store.apply_local_transaction(&Transaction::new(vec![operation]))),
        ValidationError::PropertyNotFound {
            database: fixture.database,
            property: unknown
        }
    );
    assert_eq!(
        store.views(fixture.database).unwrap()[0].visible_columns,
        vec![fixture.property]
    );
}

#[test]
fn a_database_with_rows_cannot_be_deleted() {
    let mut store = open_in_memory();
    let fixture = row_fixture(&mut store);
    let delete = Transaction::new(vec![Operation::Delete(RecordId::Database(
        fixture.database,
    ))]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&delete)),
        ValidationError::DatabaseHasRows(fixture.database)
    );
}

#[test]
fn schema_properties_are_separate_fields() {
    let mut store = open_in_memory();
    let fixture = row_fixture(&mut store);
    let added = Property {
        id: PropertyId::generate(),
        name: "Due".into(),
        property_type: PropertyType::Date,
        options: vec![],
    };
    store
        .apply_local_transaction(&Transaction::new(vec![
            Operation::Set(FieldChange::Database {
                id: fixture.database,
                field: DatabaseField::Property(added.clone()),
            }),
            Operation::Set(FieldChange::Database {
                id: fixture.database,
                field: DatabaseField::RemovedProperty(fixture.property),
            }),
        ]))
        .unwrap();
    assert_eq!(
        store
            .database(fixture.database)
            .unwrap()
            .unwrap()
            .properties,
        vec![added]
    );
}

// Conflict resolution.

/// A linked store holding one page the server created at sequence 1.
fn linked_store_with_page() -> (WorkspaceStore, PageId) {
    let mut store = open_linked_in_memory();
    let page = new_page(None);
    let id = page.id;
    apply_remote(&mut store, 1, Operation::Create(Record::Page(page)));
    (store, id)
}

fn queue_local(store: &mut WorkspaceStore, operation: Operation) -> TransactionId {
    let transaction = Transaction::new(vec![operation]);
    store.apply_local_transaction(&transaction).unwrap();
    transaction.id
}

fn apply_remote(store: &mut WorkspaceStore, seq: u64, operation: Operation) {
    store
        .apply_remote_transaction(seq, &Transaction::new(vec![operation]))
        .unwrap();
}

#[test]
fn newer_remote_writes_apply_and_replays_are_ignored() {
    let (mut store, page) = linked_store_with_page();

    apply_remote(&mut store, 5, set_title(page, "five"));
    assert_eq!(title_of(&store, page), "five");
    apply_remote(&mut store, 4, set_title(page, "four, replayed"));
    apply_remote(&mut store, 5, set_title(page, "five, replayed"));
    assert_eq!(title_of(&store, page), "five");
    apply_remote(&mut store, 6, set_title(page, "six"));
    assert_eq!(title_of(&store, page), "six");
    assert_eq!(store.last_seen_workspace_seq().unwrap(), 6);
}

#[test]
fn a_pending_local_write_survives_an_older_remote_write_and_yields_to_a_newer_one() {
    let (mut store, page) = linked_store_with_page();
    apply_remote(&mut store, 2, set_title(page, "server"));

    let local = queue_local(&mut store, set_title(page, "local"));
    // The server committed someone else's write before ours: ours will be ordered after it.
    apply_remote(&mut store, 3, set_title(page, "older remote"));
    assert_eq!(title_of(&store, page), "local");

    store.acknowledge_transactions(&[(local, 4)]).unwrap();
    assert_eq!(title_of(&store, page), "local");
    assert!(test_support::queued_changes(&store).unwrap().is_empty());

    apply_remote(&mut store, 5, set_title(page, "newer remote"));
    assert_eq!(title_of(&store, page), "newer remote");
}

#[test]
fn a_field_stays_pending_until_its_latest_local_write_is_acknowledged() {
    let (mut store, page) = linked_store_with_page();
    let first = queue_local(&mut store, set_title(page, "first"));
    let second = queue_local(&mut store, set_title(page, "second"));

    store.acknowledge_transactions(&[(first, 2)]).unwrap();
    // Our second write is still in flight, so it will be ordered after this one.
    apply_remote(&mut store, 3, set_title(page, "remote"));
    assert_eq!(title_of(&store, page), "second");

    store.acknowledge_transactions(&[(second, 4)]).unwrap();
    apply_remote(&mut store, 5, set_title(page, "later remote"));
    assert_eq!(title_of(&store, page), "later remote");
}

#[test]
fn concurrent_writes_to_different_fields_both_survive() {
    let (mut store, page) = linked_store_with_page();
    queue_local(&mut store, set_title(page, "local title"));
    let icon = Operation::Set(FieldChange::Page {
        id: page,
        field: PageField::Icon(Some("🌱".into())),
    });
    apply_remote(&mut store, 2, icon);

    let stored = store.page(page).unwrap().unwrap();
    assert_eq!(
        (stored.title.as_str(), stored.icon.as_deref()),
        ("local title", Some("🌱"))
    );
}

#[test]
fn receiving_our_own_queued_transaction_counts_as_its_acknowledgement() {
    let (mut store, page) = linked_store_with_page();
    let transaction = Transaction::new(vec![set_title(page, "mine")]);
    store.apply_local_transaction(&transaction).unwrap();

    store.apply_remote_transaction(7, &transaction).unwrap();
    assert!(test_support::queued_changes(&store).unwrap().is_empty());
    assert_eq!(store.last_seen_workspace_seq().unwrap(), 7);

    // No longer pending, so a newer remote write applies.
    apply_remote(&mut store, 8, set_title(page, "theirs"));
    assert_eq!(title_of(&store, page), "theirs");
    // A late acknowledgement is harmless.
    store
        .acknowledge_transactions(&[(transaction.id, 7)])
        .unwrap();
    assert_eq!(title_of(&store, page), "theirs");
}

#[test]
fn a_rejected_transaction_shows_the_remote_write_it_was_hiding() {
    let (mut store, page) = linked_store_with_page();
    let rejected = queue_local(&mut store, set_title(page, "refused"));
    apply_remote(&mut store, 2, set_title(page, "hidden while pending"));
    assert_eq!(title_of(&store, page), "refused");

    store.reject_transaction(rejected).unwrap();
    assert!(test_support::queued_changes(&store).unwrap().is_empty());
    assert_eq!(title_of(&store, page), "hidden while pending");
    apply_remote(&mut store, 3, set_title(page, "later"));
    assert_eq!(title_of(&store, page), "later");
}

#[test]
fn a_remote_delete_beats_a_pending_edit_and_later_writes_to_it_are_dropped() {
    let mut store = open_linked_in_memory();
    let (fixture, operations) = row_fixture_operations();
    store
        .apply_remote_transaction(1, &Transaction::new(operations))
        .unwrap();
    queue_local(&mut store, set_title(fixture.row, "edited"));

    apply_remote(
        &mut store,
        2,
        Operation::Delete(RecordId::Page(fixture.row)),
    );
    assert_eq!(store.page(fixture.row).unwrap(), None);

    let value = PropertyValue::Select(fixture.option);
    apply_remote(
        &mut store,
        3,
        Operation::Set(FieldChange::PropertyValue {
            page: fixture.row,
            property: fixture.property,
            value: Some(value),
        }),
    );
    assert!(store.property_values(fixture.row).unwrap().is_empty());
}

#[test]
fn remote_creates_are_idempotent() {
    let mut store = open_linked_in_memory();
    let page = new_page(None);
    let create = Transaction::new(vec![Operation::Create(Record::Page(page.clone()))]);
    store.apply_remote_transaction(1, &create).unwrap();
    let again = Transaction::new(vec![Operation::Create(Record::Page(Page {
        title: "changed".into(),
        ..page.clone()
    }))]);
    store.apply_remote_transaction(2, &again).unwrap();
    assert_eq!(store.page(page.id).unwrap(), Some(page));
}

// Queue.

fn queued_update_ids(store: &WorkspaceStore) -> Vec<uuid::Uuid> {
    test_support::queued_changes(store)
        .unwrap()
        .into_iter()
        .filter_map(|change| match change {
            QueuedChange::PageUpdate(update) => Some(update.update_id),
            QueuedChange::Transaction(_) => None,
        })
        .collect()
}

#[test]
fn queued_changes_come_back_in_order_and_leave_on_acknowledgement() {
    let (mut store, page) = linked_store_with_page();
    let first = Transaction::new(vec![set_title(page, "a")]);
    let last = Transaction::new(vec![set_title(page, "b")]);
    store.apply_local_transaction(&first).unwrap();
    store.record_local_page_update(page, b"loro bytes").unwrap();
    store.apply_local_transaction(&last).unwrap();

    let queued = test_support::queued_changes(&store).unwrap();
    assert_eq!(queued.len(), 3);
    assert_eq!(queued[0], QueuedChange::Transaction(first.clone()));
    assert!(
        matches!(&queued[1], QueuedChange::PageUpdate(update) if update.update == b"loro bytes")
    );
    assert_eq!(queued[2], QueuedChange::Transaction(last.clone()));

    store.acknowledge_transactions(&[(first.id, 2)]).unwrap();
    store
        .acknowledge_page_update(queued_update_ids(&store)[0], 1)
        .unwrap();
    assert_eq!(
        test_support::queued_changes(&store).unwrap(),
        vec![QueuedChange::Transaction(last)]
    );
}

#[test]
fn the_queue_reads_in_bounded_steps_and_counts_cheaply() {
    let (mut store, page) = linked_store_with_page();
    let transactions: Vec<Transaction> = (0..5)
        .map(|index| Transaction::new(vec![set_title(page, &index.to_string())]))
        .collect();
    for transaction in &transactions {
        store.apply_local_transaction(transaction).unwrap();
    }
    assert_eq!(store.queued_change_count().unwrap(), 5);

    let first = store.queued_changes_after(0, 2).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(
        first[1].change,
        QueuedChange::Transaction(transactions[1].clone())
    );
    let rest = store.queued_changes_after(first[1].position, 10).unwrap();
    let rest: Vec<QueuedChange> = rest.into_iter().map(|entry| entry.change).collect();
    let expected: Vec<QueuedChange> = transactions[2..]
        .iter()
        .cloned()
        .map(QueuedChange::Transaction)
        .collect();
    assert_eq!(rest, expected);

    store
        .acknowledge_transactions(&[(transactions[0].id, 1)])
        .unwrap();
    assert_eq!(store.queued_change_count().unwrap(), 4);
}

#[test]
fn acknowledgements_can_be_applied_together() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let first = queue(&mut client, vec![set_icon(page, "a")]);
    let second = queue(&mut client, vec![set_icon(page, "b")]);
    let first_seq = server.commit(&first);
    let second_seq = server.commit(&second);

    client
        .acknowledge_transactions(&[(first.id, first_seq), (second.id, second_seq)])
        .unwrap();
    assert_eq!(client.queued_change_count().unwrap(), 0);
    assert_eq!(client.last_seen_workspace_seq().unwrap(), second_seq);
    assert_matches_server(&client, &server);
    // Nothing is pending, so a remote write now applies directly.
    server.commit_remote(&mut client, vec![set_icon(page, "c")]);
    assert_eq!(icon_of(&client, page).as_deref(), Some("c"));
}

#[test]
fn acknowledging_a_change_as_the_wrong_kind_is_an_error() {
    let (mut store, page) = linked_store_with_page();
    store.record_local_page_update(page, b"bytes").unwrap();
    let update_id = queued_update_ids(&store)[0];
    let result = store.acknowledge_transactions(&[(TransactionId(update_id), 2)]);
    assert!(matches!(result, Err(StoreError::QueueEntryKindMismatch(id)) if id == update_id));
    assert_eq!(test_support::queued_changes(&store).unwrap().len(), 1);
}

#[test]
fn local_only_changes_are_not_queued() {
    let mut store = open_in_memory();
    let page = create_page(&mut store, None);
    store.record_local_page_update(page, b"bytes").unwrap();
    assert!(test_support::queued_changes(&store).unwrap().is_empty());
    assert_eq!(store.last_seen_page_seq(page).unwrap(), 1);
}

#[test]
fn the_last_seen_sequence_persists_across_reopening() {
    let directory = std::env::temp_dir().join(format!("scroll-core-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    {
        let mut store = WorkspaceStore::open(&path).unwrap();
        store
            .link_to_server("https://scroll.test", WorkspaceId::generate())
            .unwrap();
        apply_remote(
            &mut store,
            9,
            Operation::Create(Record::Page(new_page(None))),
        );
    }
    let store = WorkspaceStore::open(&path).unwrap();
    assert_eq!(store.last_seen_workspace_seq().unwrap(), 9);
    assert!(matches!(
        store.link().unwrap(),
        WorkspaceLink::Linked { .. }
    ));
    std::fs::remove_dir_all(&directory).unwrap();
}

// Rejection: after it, the local state must equal the server's.

/// Stands in for the server: commits transactions in order and numbers them.
struct Server {
    store: WorkspaceStore,
    seq: u64,
}

impl Server {
    fn new() -> Self {
        Server {
            store: open_linked_in_memory(),
            seq: 0,
        }
    }

    /// Commits a transaction from another client and delivers it to `client`.
    fn commit_remote(&mut self, client: &mut WorkspaceStore, operations: Vec<Operation>) {
        let transaction = Transaction::new(operations);
        let seq = self.commit(&transaction);
        client.apply_remote_transaction(seq, &transaction).unwrap();
    }

    /// Commits a transaction `client` queued and acknowledges it.
    fn accept(&mut self, client: &mut WorkspaceStore, transaction: &Transaction) {
        let seq = self.commit(transaction);
        client
            .acknowledge_transactions(&[(transaction.id, seq)])
            .unwrap();
    }

    fn commit(&mut self, transaction: &Transaction) -> u64 {
        self.seq += 1;
        self.store
            .apply_remote_transaction(self.seq, transaction)
            .unwrap();
        self.seq
    }
}

fn queue(client: &mut WorkspaceStore, operations: Vec<Operation>) -> Transaction {
    let transaction = Transaction::new(operations);
    client.apply_local_transaction(&transaction).unwrap();
    transaction
}

/// Records and values, ordered so two stores can be compared.
fn records_of(store: &WorkspaceStore) -> (Vec<String>, Vec<String>) {
    let snapshot = store.export_workspace_snapshot().unwrap();
    let mut records: Vec<String> = snapshot
        .records
        .iter()
        .map(|record| format!("{record:?}"))
        .collect();
    let mut values: Vec<String> = snapshot
        .property_values
        .iter()
        .map(|entry| format!("{entry:?}"))
        .collect();
    records.sort();
    values.sort();
    (records, values)
}

fn assert_matches_server(client: &WorkspaceStore, server: &Server) {
    assert_eq!(records_of(client), records_of(&server.store));
}

fn icon_of(store: &WorkspaceStore, page: PageId) -> Option<String> {
    store.page(page).unwrap().unwrap().icon
}

fn set_icon(page: PageId, icon: &str) -> Operation {
    Operation::Set(FieldChange::Page {
        id: page,
        field: PageField::Icon(Some(icon.into())),
    })
}

fn shared_page(server: &mut Server, client: &mut WorkspaceStore) -> PageId {
    let page = new_page(None);
    let id = page.id;
    server.commit_remote(client, vec![Operation::Create(Record::Page(page))]);
    id
}

#[test]
fn acknowledgements_keep_a_record_pending_while_a_later_queued_transaction_touches_it() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let first = queue(&mut client, vec![set_icon(page, "a")]);
    let second = queue(&mut client, vec![set_icon(page, "b")]);
    server.accept(&mut client, &first);

    // The second write is still pending, so the record keeps its confirmed copy ("a") and a
    // rejection goes back to it.
    client.reject_transaction(second.id).unwrap();
    assert_eq!(icon_of(&client, page).as_deref(), Some("a"));
    assert_matches_server(&client, &server);
}

#[test]
#[ignore = "timing; run with --release --ignored to measure"]
fn acknowledging_a_large_publish_costs_in_proportion_to_each_transaction() {
    let mut store = open_in_memory();
    let parent = create_page(&mut store, None);
    let operations: Vec<Operation> = (0..200_000)
        .map(|_| Operation::Create(Record::Page(new_page(Some(parent)))))
        .collect();
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    store
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();
    let queued = test_support::queued_changes(&store).unwrap();
    let started = std::time::Instant::now();
    for (seq, change) in queued.iter().enumerate() {
        if let QueuedChange::Transaction(transaction) = change {
            store
                .acknowledge_transactions(&[(transaction.id, seq as u64 + 1)])
                .unwrap();
        }
    }
    println!(
        "acknowledging {} publishing transactions: {:?}",
        queued.len(),
        started.elapsed()
    );
    assert_eq!(store.queued_change_count().unwrap(), 0);
}

#[test]
fn a_rejected_write_restores_the_server_value() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let rejected = queue(
        &mut client,
        vec![set_title(page, "mine"), set_icon(page, "x")],
    );

    client.reject_transaction(rejected.id).unwrap();
    assert_eq!(title_of(&client, page), "Untitled");
    assert_eq!(icon_of(&client, page), None);
    assert_matches_server(&client, &server);
}

#[test]
fn rejection_after_an_interleaved_remote_write_shows_the_remote_value() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let rejected = queue(
        &mut client,
        vec![set_title(page, "mine"), set_icon(page, "x")],
    );
    server.commit_remote(&mut client, vec![set_title(page, "theirs")]);
    assert_eq!(title_of(&client, page), "mine");

    client.reject_transaction(rejected.id).unwrap();
    assert_eq!(title_of(&client, page), "theirs");
    assert_eq!(icon_of(&client, page), None);
    assert_matches_server(&client, &server);
}

#[test]
fn rejecting_the_first_of_two_pending_transactions_keeps_the_second() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let first = queue(
        &mut client,
        vec![set_title(page, "one"), set_icon(page, "1")],
    );
    let second = queue(&mut client, vec![set_title(page, "two")]);

    client.reject_transaction(first.id).unwrap();
    assert_eq!(title_of(&client, page), "two");
    // The icon was written only by the rejected transaction.
    assert_eq!(icon_of(&client, page), None);

    server.accept(&mut client, &second);
    assert_eq!(title_of(&client, page), "two");
    assert_matches_server(&client, &server);
}

#[test]
fn rejecting_both_pending_transactions_around_a_remote_write_matches_the_server() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let first = queue(&mut client, vec![set_title(page, "one")]);
    server.commit_remote(
        &mut client,
        vec![set_title(page, "theirs"), set_icon(page, "r")],
    );
    let second = queue(&mut client, vec![set_title(page, "two")]);
    assert_eq!(title_of(&client, page), "two");
    assert_eq!(icon_of(&client, page).as_deref(), Some("r"));

    client.reject_transaction(first.id).unwrap();
    assert_eq!(title_of(&client, page), "two");
    client.reject_transaction(second.id).unwrap();
    assert_eq!(title_of(&client, page), "theirs");
    assert_matches_server(&client, &server);
}

#[test]
fn an_acknowledged_write_becomes_the_value_a_later_rejection_restores() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let first = queue(&mut client, vec![set_title(page, "one")]);
    let second = queue(&mut client, vec![set_title(page, "two")]);
    server.accept(&mut client, &first);
    server.commit_remote(&mut client, vec![set_icon(page, "r")]);

    client.reject_transaction(second.id).unwrap();
    assert_eq!(title_of(&client, page), "one");
    assert_eq!(icon_of(&client, page).as_deref(), Some("r"));
    assert_matches_server(&client, &server);
}

#[test]
fn a_rejected_create_removes_the_record_and_what_depended_on_it() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let parent = shared_page(&mut server, &mut client);
    let child = new_page(Some(parent));
    let child_id = child.id;
    let create = queue(&mut client, vec![Operation::Create(Record::Page(child))]);
    let rename = queue(&mut client, vec![set_title(child_id, "named")]);

    client.reject_transaction(create.id).unwrap();
    assert_eq!(client.page(child_id).unwrap(), None);
    // The server rejects the rename too, since the page never reached it.
    client.reject_transaction(rename.id).unwrap();
    assert_eq!(client.page(child_id).unwrap(), None);
    assert!(test_support::queued_changes(&client).unwrap().is_empty());
    assert_matches_server(&client, &server);
}

#[test]
fn a_rejected_create_of_a_row_drops_its_values() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let (fixture, operations) = row_fixture_operations();
    server.commit_remote(&mut client, operations);
    let row = Page {
        row_of: Some(fixture.database),
        ..new_page(None)
    };
    let row_id = row.id;
    let rejected = queue(
        &mut client,
        vec![
            Operation::Create(Record::Page(row)),
            Operation::Set(FieldChange::PropertyValue {
                page: row_id,
                property: fixture.property,
                value: Some(PropertyValue::Select(fixture.option)),
            }),
        ],
    );

    client.reject_transaction(rejected.id).unwrap();
    assert_eq!(client.page(row_id).unwrap(), None);
    assert!(client.property_values(row_id).unwrap().is_empty());
    assert_matches_server(&client, &server);
}

#[test]
fn a_rejected_delete_brings_back_the_record_with_its_values_content_and_remote_edits() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let (fixture, operations) = row_fixture_operations();
    server.commit_remote(&mut client, operations);
    server.commit_remote(
        &mut client,
        vec![Operation::Set(FieldChange::PropertyValue {
            page: fixture.row,
            property: fixture.property,
            value: Some(PropertyValue::Select(fixture.option)),
        })],
    );
    let content = SequencedBytes {
        seq: 1,
        bytes: page_content_update(),
    };
    client
        .store_remote_page_update(fixture.row, &content)
        .unwrap();

    let rejected = queue(
        &mut client,
        vec![Operation::Delete(RecordId::Page(fixture.row))],
    );
    assert_eq!(client.page(fixture.row).unwrap(), None);
    server.commit_remote(
        &mut client,
        vec![set_title(fixture.row, "renamed meanwhile")],
    );

    client.reject_transaction(rejected.id).unwrap();
    assert_eq!(title_of(&client, fixture.row), "renamed meanwhile");
    assert_eq!(
        client.property_values(fixture.row).unwrap(),
        vec![(fixture.property, PropertyValue::Select(fixture.option))]
    );
    assert_eq!(
        client
            .load_page_doc(fixture.row)
            .unwrap()
            .blocks()
            .unwrap()
            .len(),
        1
    );
    let found: Vec<PageId> = client
        .search("findable", 10)
        .unwrap()
        .into_iter()
        .map(|hit| hit.page_id)
        .collect();
    assert_eq!(found, vec![fixture.row]);
    assert_matches_server(&client, &server);
}

#[test]
fn an_accepted_delete_drops_the_page_content() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let content = SequencedBytes {
        seq: 1,
        bytes: page_content_update(),
    };
    client.store_remote_page_update(page, &content).unwrap();
    let delete = queue(&mut client, vec![Operation::Delete(RecordId::Page(page))]);
    assert_eq!(client.last_seen_page_seq(page).unwrap(), 1);

    server.accept(&mut client, &delete);
    assert_eq!(client.last_seen_page_seq(page).unwrap(), 0);
    assert_matches_server(&client, &server);
}

#[test]
fn a_remote_delete_wins_over_a_rejected_edit() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let content = SequencedBytes {
        seq: 1,
        bytes: page_content_update(),
    };
    client.store_remote_page_update(page, &content).unwrap();
    let rejected = queue(&mut client, vec![set_title(page, "mine")]);
    server.commit_remote(&mut client, vec![Operation::Delete(RecordId::Page(page))]);
    assert_eq!(client.page(page).unwrap(), None);

    client.reject_transaction(rejected.id).unwrap();
    assert_eq!(client.page(page).unwrap(), None);
    assert_eq!(client.last_seen_page_seq(page).unwrap(), 0);
    assert_matches_server(&client, &server);
}

#[test]
fn after_every_acknowledgement_remote_writes_apply_directly() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let first = queue(&mut client, vec![set_title(page, "one")]);
    let second = queue(&mut client, vec![set_icon(page, "i")]);
    server.accept(&mut client, &first);
    server.accept(&mut client, &second);
    server.commit_remote(&mut client, vec![set_title(page, "theirs")]);

    assert_eq!(title_of(&client, page), "theirs");
    assert_matches_server(&client, &server);
}

fn page_content_update() -> Vec<u8> {
    let mut doc = PageDoc::new();
    let block = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    doc.insert_text(block, 0, "findable words").unwrap();
    doc.take_local_update().unwrap().unwrap()
}

// Database hosting.

#[test]
fn a_page_hosts_at_most_one_database_and_cannot_be_deleted_while_hosting() {
    let mut store = open_in_memory();
    let host = create_page(&mut store, None);
    let database = |id| Database {
        id,
        host_page: host,
        properties: vec![],
    };
    let first = DatabaseId::generate();
    create(&mut store, Record::Database(database(first)));

    let second = Transaction::new(vec![Operation::Create(Record::Database(database(
        DatabaseId::generate(),
    )))]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&second)),
        ValidationError::PageAlreadyHostsDatabase {
            page: host,
            database: first
        }
    );
    let delete_host = Transaction::new(vec![Operation::Delete(RecordId::Page(host))]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&delete_host)),
        ValidationError::PageHostsDatabase {
            page: host,
            database: first
        }
    );
    let own_row = Transaction::new(vec![Operation::Set(FieldChange::Page {
        id: host,
        field: PageField::RowOf(Some(first)),
    })]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&own_row)),
        ValidationError::PageCannotBeRowOfItsOwnDatabase {
            page: host,
            database: first
        }
    );

    let missing_host = Database {
        host_page: PageId::generate(),
        ..database(DatabaseId::generate())
    };
    let missing = missing_host.host_page;
    let result = store.apply_local_transaction(&Transaction::new(vec![Operation::Create(
        Record::Database(missing_host),
    )]));
    assert_eq!(
        validation_error(result),
        ValidationError::PageNotFound(missing)
    );
}

// Catch-up.

fn catch_up_with(
    snapshot: Option<crate::sync::WorkspaceSnapshot>,
    transactions: Vec<(u64, Transaction)>,
) -> crate::sync::WorkspaceCatchUp {
    crate::sync::WorkspaceCatchUp {
        snapshot,
        transactions: transactions
            .into_iter()
            .map(|(seq, transaction)| crate::sync::SequencedTransaction { seq, transaction })
            .collect(),
    }
}

#[test]
fn a_snapshot_replaces_the_confirmed_records() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let stale = shared_page(&mut server, &mut client);
    let content = SequencedBytes {
        seq: 1,
        bytes: page_content_update(),
    };
    client.store_remote_page_update(stale, &content).unwrap();

    let mut other_server = Server::new();
    let (fixture, operations) = row_fixture_operations();
    other_server.commit(&Transaction::new(operations));
    other_server.commit(&Transaction::new(vec![Operation::Set(
        FieldChange::PropertyValue {
            page: fixture.row,
            property: fixture.property,
            value: Some(PropertyValue::Select(fixture.option)),
        },
    )]));
    let snapshot = other_server.store.export_workspace_snapshot().unwrap();
    assert_eq!(snapshot.seq, 2);

    test_support::apply_snapshot(&mut client, snapshot).unwrap();
    assert_eq!(client.page(stale).unwrap(), None);
    assert_eq!(client.last_seen_page_seq(stale).unwrap(), 0);
    assert_eq!(client.last_seen_workspace_seq().unwrap(), 2);
    assert_eq!(records_of(&client), records_of(&other_server.store));
}

#[test]
fn a_snapshot_keeps_pending_local_changes_on_top() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let rename = queue(&mut client, vec![set_title(page, "mine")]);
    let created = new_page(None);
    let created_id = created.id;
    let create = queue(&mut client, vec![Operation::Create(Record::Page(created))]);

    // Meanwhile the server moved on; the client reconnects with a fresh snapshot.
    server.commit(&Transaction::new(vec![
        set_title(page, "server"),
        set_icon(page, "s"),
    ]));
    let snapshot = server.store.export_workspace_snapshot().unwrap();
    test_support::apply_snapshot(&mut client, snapshot).unwrap();

    assert_eq!(title_of(&client, page), "mine");
    assert_eq!(icon_of(&client, page).as_deref(), Some("s"));
    assert!(client.page(created_id).unwrap().is_some());

    client.reject_transaction(rename.id).unwrap();
    client.reject_transaction(create.id).unwrap();
    assert_eq!(title_of(&client, page), "server");
    assert_matches_server(&client, &server);
}

#[test]
fn catch_up_transactions_apply_once_in_order() {
    let (mut server, mut client) = (Server::new(), open_linked_in_memory());
    let page = shared_page(&mut server, &mut client);
    let missed = Transaction::new(vec![set_title(page, "missed")]);
    let later = Transaction::new(vec![set_icon(page, "later")]);
    let missed_seq = server.commit(&missed);
    let later_seq = server.commit(&later);
    let catch_up = catch_up_with(None, vec![(missed_seq, missed), (later_seq, later)]);

    client
        .apply_catch_up_transactions(&catch_up.transactions)
        .unwrap();
    client
        .apply_catch_up_transactions(&catch_up.transactions)
        .unwrap();
    assert_eq!(title_of(&client, page), "missed");
    assert_eq!(client.last_seen_workspace_seq().unwrap(), later_seq);
    assert_matches_server(&client, &server);
}

#[test]
fn an_exported_snapshot_rebuilds_the_same_records_elsewhere() {
    let mut source = open_in_memory();
    let fixture = row_fixture(&mut source);
    source
        .apply_local_transaction(&Transaction::new(vec![Operation::Set(
            FieldChange::PropertyValue {
                page: fixture.row,
                property: fixture.property,
                value: Some(PropertyValue::Select(fixture.option)),
            },
        )]))
        .unwrap();

    let mut copy = open_linked_in_memory();
    let snapshot = source.export_workspace_snapshot().unwrap();
    test_support::apply_snapshot(&mut copy, snapshot).unwrap();
    assert_eq!(records_of(&copy), records_of(&source));
}

#[test]
fn a_local_only_store_refuses_server_changes() {
    let mut store = open_in_memory();
    let page = create_page(&mut store, None);
    let transaction = Transaction::new(vec![set_title(page, "remote")]);
    fn not_linked<T>(result: Result<T, StoreError>) -> bool {
        matches!(result, Err(StoreError::NotLinked))
    }

    assert!(not_linked(store.apply_remote_transaction(1, &transaction)));
    assert!(not_linked(
        store.acknowledge_transactions(&[(transaction.id, 1)])
    ));
    assert!(not_linked(store.reject_transaction(transaction.id)));
    assert!(not_linked(store.apply_catch_up_transactions(&[])));
    let update = SequencedBytes {
        seq: 1,
        bytes: page_content_update(),
    };
    assert!(not_linked(store.store_remote_page_update(page, &update)));
    assert!(not_linked(
        store.acknowledge_page_update(uuid::Uuid::now_v7(), 1)
    ));
    assert_eq!(title_of(&store, page), "Untitled");
}

#[test]
fn a_property_added_earlier_in_a_transaction_can_be_given_values() {
    let mut store = open_in_memory();
    let fixture = row_fixture(&mut store);
    let added = Property {
        id: PropertyId::generate(),
        name: "Points".into(),
        property_type: PropertyType::Number,
        options: vec![],
    };
    let value = |value| {
        Operation::Set(FieldChange::PropertyValue {
            page: fixture.row,
            property: added.id,
            value: Some(value),
        })
    };
    let transaction = Transaction::new(vec![
        // Reads (and caches) the schema before the property exists.
        Operation::Set(FieldChange::PropertyValue {
            page: fixture.row,
            property: fixture.property,
            value: None,
        }),
        Operation::Set(FieldChange::Database {
            id: fixture.database,
            field: DatabaseField::Property(added.clone()),
        }),
        value(PropertyValue::Number(3.0)),
    ]);
    store.apply_local_transaction(&transaction).unwrap();
    assert_eq!(
        store.property_values(fixture.row).unwrap(),
        vec![(added.id, PropertyValue::Number(3.0))]
    );
}

#[test]
fn child_pages_leave_out_database_rows() {
    let mut store = open_in_memory();
    let (fixture, operations) = row_fixture_operations();
    store
        .apply_local_transaction(&Transaction::new(operations))
        .unwrap();
    let host = store.database(fixture.database).unwrap().unwrap().host_page;
    let child = create_page(&mut store, Some(host));
    store
        .apply_local_transaction(&Transaction::new(vec![set_parent(fixture.row, host)]))
        .unwrap();

    let children: Vec<PageId> = test_support::child_pages(&store, Some(host))
        .unwrap()
        .into_iter()
        .map(|page| page.id)
        .collect();
    assert_eq!(children, vec![child]);
}

#[test]
fn two_stores_on_one_file_can_write_at_the_same_time() {
    let directory = std::env::temp_dir().join(format!("scroll-core-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    WorkspaceStore::open(&path).unwrap();

    let writers: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut store = WorkspaceStore::open(&path).unwrap();
                for _ in 0..300 {
                    // Validation reads before the transaction writes.
                    create_page(&mut store, None);
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("a writer failed");
    }
    let store = WorkspaceStore::open(&path).unwrap();
    assert_eq!(test_support::child_pages(&store, None).unwrap().len(), 600);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn column_widths_are_saved_and_checked() {
    let mut store = open_in_memory();
    let fixture = row_fixture(&mut store);
    let view = View {
        id: ViewId::generate(),
        database: fixture.database,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: vec![],
        sort: vec![],
        visible_columns: vec![fixture.property],
        column_widths: Vec::new(),
    };
    let view_id = view.id;
    create(&mut store, Record::View(view));
    let set_widths = |widths| {
        Transaction::new(vec![Operation::Set(FieldChange::View {
            id: view_id,
            field: ViewField::ColumnWidths(widths),
        })])
    };
    let width = |property, width| crate::record::ColumnWidth { property, width };

    store
        .apply_local_transaction(&set_widths(vec![width(fixture.property, 240)]))
        .unwrap();
    assert_eq!(
        store.view(view_id).unwrap().unwrap().column_widths,
        vec![width(fixture.property, 240)]
    );

    let too_narrow = set_widths(vec![width(fixture.property, 0)]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&too_narrow)),
        ValidationError::InvalidColumnWidth {
            property: fixture.property,
            width: 0
        }
    );
    let unknown = PropertyId::generate();
    let missing = set_widths(vec![width(unknown, 100)]);
    assert_eq!(
        validation_error(store.apply_local_transaction(&missing)),
        ValidationError::PropertyNotFound {
            database: fixture.database,
            property: unknown
        }
    );
}

#[test]
fn a_reader_sees_what_the_writer_committed() {
    let directory = std::env::temp_dir().join(format!("scroll-core-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("workspace.scroll");
    let mut writer = WorkspaceStore::open(&path).unwrap();
    assert_eq!(writer.path(), Some(std::fs::canonicalize(&path).unwrap()));
    assert_eq!(open_in_memory().path(), None);

    let mut reader = WorkspaceStore::open_reader(&path).unwrap();
    let page = create_page(&mut writer, None);
    assert_eq!(title_of(&reader, page), "Untitled");
    let write = Transaction::new(vec![set_title(page, "through the reader")]);
    assert!(reader.apply_local_transaction(&write).is_err());
    std::fs::remove_dir_all(&directory).unwrap();
}
