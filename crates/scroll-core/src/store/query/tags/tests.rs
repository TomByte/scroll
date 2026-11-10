use crate::id::{DatabaseId, PageId, TagId, WorkspaceId};
use crate::operation::{
    FieldChange, Operation, PageField, Record, RecordId, TagField, Transaction,
};
use crate::position::Position;
use crate::record::{Database, Page, PageTag, Tag, TagColor};
use crate::store::test_support;
use crate::store::{StoreError, WorkspaceStore, open_in_memory, open_linked_in_memory};
use crate::sync::WorkspaceSnapshot;
use crate::validation::ValidationError;

fn new_page(title: &str) -> Page {
    Page {
        id: PageId::generate(),
        parent: None,
        title: title.into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    }
}

fn new_tag(name: &str) -> Tag {
    Tag {
        id: TagId::generate(),
        name: name.into(),
        color: TagColor::Blue,
    }
}

fn apply(store: &mut WorkspaceStore, operations: Vec<Operation>) -> Result<(), StoreError> {
    store.apply_local_transaction(&Transaction::new(operations))?;
    Ok(())
}

fn create_page(store: &mut WorkspaceStore, title: &str) -> PageId {
    let page = new_page(title);
    let id = page.id;
    apply(store, vec![Operation::Create(Record::Page(page))]).unwrap();
    id
}

fn create_tag(store: &mut WorkspaceStore, name: &str) -> TagId {
    let tag = new_tag(name);
    let id = tag.id;
    apply(store, vec![Operation::Create(Record::Tag(tag))]).unwrap();
    id
}

fn add_tag(page: PageId, tag: TagId) -> Operation {
    Operation::Create(Record::PageTag(PageTag { page, tag }))
}

fn remove_tag(page: PageId, tag: TagId) -> Operation {
    Operation::Delete(RecordId::PageTag { page, tag })
}

fn rename(tag: TagId, name: &str) -> Operation {
    Operation::Set(FieldChange::Tag {
        id: tag,
        field: TagField::Name(name.into()),
    })
}

fn tag_names(store: &WorkspaceStore, page: PageId) -> Vec<String> {
    let tags = store.page_tags(page).unwrap();
    tags.into_iter().map(|tag| tag.name).collect()
}

fn membership_row_count(store: &WorkspaceStore) -> usize {
    store
        .conn
        .query_row("SELECT count(*) FROM page_tag", [], |row| row.get(0))
        .unwrap()
}

fn search_ids(store: &WorkspaceStore, query: &str) -> Vec<PageId> {
    let hits = store.search(query, 10).unwrap();
    hits.into_iter().map(|hit| hit.page_id).collect()
}

fn validation_error<T: std::fmt::Debug>(result: Result<T, StoreError>) -> ValidationError {
    match result {
        Err(StoreError::Validation(error)) => error,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// Records, ordered so two stores can be compared.
fn records_of(store: &WorkspaceStore) -> Vec<String> {
    let snapshot = store.export_workspace_snapshot().unwrap();
    let mut records: Vec<String> = snapshot
        .records
        .iter()
        .map(|record| format!("{record:?}"))
        .collect();
    records.sort();
    records
}

#[test]
fn tags_list_with_usage_counts_and_pages_list_their_tags() {
    let mut store = open_in_memory();
    let host = create_page(&mut store, "Tasks");
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host,
        properties: vec![],
    };
    let row = Page {
        row_of: Some(database.id),
        ..new_page("Row")
    };
    let row_id = row.id;
    apply(
        &mut store,
        vec![
            Operation::Create(Record::Database(database)),
            Operation::Create(Record::Page(row)),
        ],
    )
    .unwrap();
    let archived = create_page(&mut store, "Old");
    let urgent = create_tag(&mut store, "urgent");
    let design = create_tag(&mut store, "Design");
    create_tag(&mut store, "unused");

    apply(
        &mut store,
        vec![
            add_tag(host, urgent),
            add_tag(host, design),
            add_tag(row_id, urgent),
            add_tag(archived, urgent),
            Operation::Set(FieldChange::Page {
                id: archived,
                field: PageField::IsArchived(true),
            }),
        ],
    )
    .unwrap();

    let usage: Vec<(String, usize)> = store
        .tags()
        .unwrap()
        .into_iter()
        .map(|usage| (usage.tag.name, usage.page_count))
        .collect();
    assert_eq!(
        usage,
        vec![
            ("Design".into(), 1),
            ("unused".into(), 0),
            ("urgent".into(), 2)
        ]
    );
    assert_eq!(tag_names(&store, host), vec!["Design", "urgent"]);
    assert_eq!(tag_names(&store, row_id), vec!["urgent"]);
    assert_eq!(store.tag_named(" URGENT ").unwrap().unwrap().id, urgent);
    assert_eq!(store.tag_named("urgen").unwrap(), None);
}

#[test]
fn tag_names_are_unique_ignoring_case_and_never_empty() {
    let mut store = open_in_memory();
    let urgent = create_tag(&mut store, "Urgent");
    let later = create_tag(&mut store, "Later");

    let duplicate = new_tag("  uRGENT");
    assert_eq!(
        validation_error(apply(
            &mut store,
            vec![Operation::Create(Record::Tag(duplicate))]
        )),
        ValidationError::TagNameTaken {
            name: "  uRGENT".into(),
            existing: urgent
        }
    );
    assert_eq!(
        validation_error(apply(&mut store, vec![rename(later, "urgent")])),
        ValidationError::TagNameTaken {
            name: "urgent".into(),
            existing: urgent
        }
    );
    assert_eq!(
        validation_error(apply(&mut store, vec![rename(later, " ")])),
        ValidationError::TagNameEmpty
    );
    apply(&mut store, vec![rename(urgent, "URGENT")]).unwrap();
    assert_eq!(store.tag(urgent).unwrap().unwrap().name, "URGENT");

    // The old name is free once a tag gives it up.
    apply(&mut store, vec![rename(urgent, "Now")]).unwrap();
    apply(&mut store, vec![rename(later, "urgent")]).unwrap();
}

#[test]
fn memberships_need_their_page_and_tag_and_repeating_them_changes_nothing() {
    let mut store = open_in_memory();
    let page = create_page(&mut store, "Plan");
    let tag = create_tag(&mut store, "urgent");
    let missing_page = PageId::generate();
    let missing_tag = TagId::generate();

    assert_eq!(
        validation_error(apply(&mut store, vec![add_tag(missing_page, tag)])),
        ValidationError::PageNotFound(missing_page)
    );
    assert_eq!(
        validation_error(apply(&mut store, vec![add_tag(page, missing_tag)])),
        ValidationError::TagNotFound(missing_tag)
    );
    assert_eq!(
        validation_error(apply(&mut store, vec![rename(missing_tag, "x")])),
        ValidationError::TagNotFound(missing_tag)
    );

    apply(&mut store, vec![add_tag(page, tag)]).unwrap();
    apply(&mut store, vec![add_tag(page, tag)]).unwrap();
    assert_eq!(tag_names(&store, page), vec!["urgent"]);
    apply(&mut store, vec![remove_tag(page, tag)]).unwrap();
    apply(&mut store, vec![remove_tag(page, tag)]).unwrap();
    assert!(tag_names(&store, page).is_empty());
}

#[test]
fn deleting_a_tag_or_a_page_deletes_their_memberships() {
    let mut store = open_in_memory();
    let kept = create_page(&mut store, "Kept");
    let doomed = create_page(&mut store, "Doomed");
    let tag = create_tag(&mut store, "urgent");
    let other = create_tag(&mut store, "other");
    apply(
        &mut store,
        vec![
            add_tag(kept, tag),
            add_tag(doomed, tag),
            add_tag(kept, other),
        ],
    )
    .unwrap();

    apply(&mut store, vec![Operation::Delete(RecordId::Page(doomed))]).unwrap();
    assert_eq!(membership_row_count(&store), 2);
    apply(&mut store, vec![Operation::Delete(RecordId::Tag(tag))]).unwrap();
    assert_eq!(membership_row_count(&store), 1);
    assert_eq!(tag_names(&store, kept), vec!["other"]);
    assert!(search_ids(&store, "urgent").is_empty());
}

#[test]
fn searching_a_tag_name_finds_its_pages_and_follows_renames_and_removal() {
    let mut store = open_in_memory();
    let page = create_page(&mut store, "Quarterly plan");
    create_page(&mut store, "Recipes");
    let tag = create_tag(&mut store, "finance");
    apply(&mut store, vec![add_tag(page, tag)]).unwrap();

    assert_eq!(search_ids(&store, "fin"), vec![page]);
    assert_eq!(search_ids(&store, "finance plan"), vec![page]);

    apply(&mut store, vec![rename(tag, "budget")]).unwrap();
    assert!(search_ids(&store, "finance").is_empty());
    assert_eq!(search_ids(&store, "budget"), vec![page]);

    apply(&mut store, vec![remove_tag(page, tag)]).unwrap();
    assert!(search_ids(&store, "budget").is_empty());
}

#[test]
fn pages_with_tags_need_every_tag() {
    let mut store = open_in_memory();
    let both = create_page(&mut store, "b both");
    let also_both = create_page(&mut store, "A both");
    let one = create_page(&mut store, "one");
    let archived = create_page(&mut store, "archived");
    let red = create_tag(&mut store, "red");
    let blue = create_tag(&mut store, "blue");
    apply(
        &mut store,
        vec![
            add_tag(both, red),
            add_tag(both, blue),
            add_tag(also_both, red),
            add_tag(also_both, blue),
            add_tag(one, red),
            add_tag(archived, red),
            add_tag(archived, blue),
            Operation::Set(FieldChange::Page {
                id: archived,
                field: PageField::IsArchived(true),
            }),
        ],
    )
    .unwrap();

    let ids = |tags: &[TagId], limit| -> Vec<PageId> {
        let pages = store.pages_with_tags(tags, limit).unwrap();
        pages.into_iter().map(|page| page.id).collect()
    };
    assert_eq!(ids(&[red, blue], 10), vec![also_both, both]);
    assert_eq!(ids(&[blue, red, blue], 10), vec![also_both, both]);
    assert_eq!(ids(&[red], 10), vec![also_both, both, one]);
    assert_eq!(ids(&[red], 1), vec![also_both]);
    assert!(ids(&[], 10).is_empty());
}

// Sync. The server stand-in is a local-only store, so it validates like the real one.

struct Server {
    store: WorkspaceStore,
    seq: u64,
}

impl Server {
    fn new() -> Self {
        Server {
            store: open_in_memory(),
            seq: 0,
        }
    }

    /// Commits the transaction if it is valid, returning its sequence number.
    fn commit(&mut self, transaction: &Transaction) -> Result<u64, StoreError> {
        self.store.apply_local_transaction(transaction)?;
        self.seq += 1;
        Ok(self.seq)
    }

    /// Commits a transaction made elsewhere and delivers it to each client.
    fn commit_remote(&mut self, clients: &mut [&mut WorkspaceStore], operations: Vec<Operation>) {
        let transaction = Transaction::new(operations);
        let seq = self.commit(&transaction).unwrap();
        for client in clients {
            client.apply_remote_transaction(seq, &transaction).unwrap();
        }
    }
}

fn queue(client: &mut WorkspaceStore, operations: Vec<Operation>) -> Transaction {
    let transaction = Transaction::new(operations);
    client.apply_local_transaction(&transaction).unwrap();
    transaction
}

/// The server's answer to `author`'s transaction, delivered to the author and, if committed,
/// to the others.
fn deliver(
    server: &mut Server,
    author: &mut WorkspaceStore,
    others: &mut [&mut WorkspaceStore],
    transaction: &Transaction,
) {
    match server.commit(transaction) {
        Ok(seq) => {
            author
                .acknowledge_transactions(&[(transaction.id, seq)])
                .unwrap();
            for other in others {
                other.apply_remote_transaction(seq, transaction).unwrap();
            }
        }
        Err(_) => {
            author.reject_transaction(transaction.id).unwrap();
        }
    }
}

fn shared_page_and_tags(
    server: &mut Server,
    clients: &mut [&mut WorkspaceStore],
) -> (PageId, TagId, TagId) {
    let page = new_page("Shared");
    let (first, second) = (new_tag("first"), new_tag("second"));
    let ids = (page.id, first.id, second.id);
    server.commit_remote(
        clients,
        vec![
            Operation::Create(Record::Page(page)),
            Operation::Create(Record::Tag(first)),
            Operation::Create(Record::Tag(second)),
        ],
    );
    ids
}

#[test]
fn concurrent_adds_of_different_tags_both_survive() {
    let mut server = Server::new();
    let (mut alice, mut bob) = (open_linked_in_memory(), open_linked_in_memory());
    let (page, first, second) = shared_page_and_tags(&mut server, &mut [&mut alice, &mut bob]);

    let from_alice = queue(&mut alice, vec![add_tag(page, first)]);
    let from_bob = queue(&mut bob, vec![add_tag(page, second)]);
    deliver(&mut server, &mut alice, &mut [&mut bob], &from_alice);
    deliver(&mut server, &mut bob, &mut [&mut alice], &from_bob);

    for client in [&alice, &bob] {
        assert_eq!(tag_names(client, page), vec!["first", "second"]);
        assert_eq!(records_of(client), records_of(&server.store));
        assert!(test_support::queued_changes(client).unwrap().is_empty());
    }
}

#[test]
fn a_concurrent_remove_and_add_of_the_same_tag_end_in_server_order() {
    let mut server = Server::new();
    let (mut alice, mut bob) = (open_linked_in_memory(), open_linked_in_memory());
    let (page, tag, _) = shared_page_and_tags(&mut server, &mut [&mut alice, &mut bob]);
    server.commit_remote(&mut [&mut alice, &mut bob], vec![add_tag(page, tag)]);

    let removal = queue(&mut alice, vec![remove_tag(page, tag)]);
    let re_add = queue(&mut bob, vec![add_tag(page, tag)]);
    deliver(&mut server, &mut alice, &mut [&mut bob], &removal);
    deliver(&mut server, &mut bob, &mut [&mut alice], &re_add);

    for client in [&alice, &bob] {
        assert_eq!(tag_names(client, page), vec!["first"]);
        assert_eq!(records_of(client), records_of(&server.store));
    }
}

#[test]
fn of_two_tags_created_with_the_same_name_at_once_the_second_is_rejected_with_its_memberships() {
    let mut server = Server::new();
    let (mut alice, mut bob) = (open_linked_in_memory(), open_linked_in_memory());
    let (page, _, _) = shared_page_and_tags(&mut server, &mut [&mut alice, &mut bob]);

    let alice_tag = new_tag("Urgent");
    let bob_tag = new_tag("urgent");
    let (alice_tag_id, bob_tag_id) = (alice_tag.id, bob_tag.id);
    let from_alice = queue(
        &mut alice,
        vec![
            Operation::Create(Record::Tag(alice_tag)),
            add_tag(page, alice_tag_id),
        ],
    );
    let from_bob = queue(
        &mut bob,
        vec![
            Operation::Create(Record::Tag(bob_tag)),
            add_tag(page, bob_tag_id),
        ],
    );
    assert_eq!(tag_names(&bob, page), vec!["urgent"]);

    deliver(&mut server, &mut alice, &mut [&mut bob], &from_alice);
    // Bob briefly has both tags; the server then rejects his.
    deliver(&mut server, &mut bob, &mut [&mut alice], &from_bob);

    for client in [&alice, &bob] {
        assert_eq!(tag_names(client, page), vec!["Urgent"]);
        assert_eq!(client.tag(bob_tag_id).unwrap(), None);
        assert_eq!(records_of(client), records_of(&server.store));
    }
    assert_eq!(bob.tag_named("URGENT").unwrap().unwrap().id, alice_tag_id);
}

#[test]
fn a_rejected_tag_delete_brings_back_its_memberships_and_an_accepted_one_drops_them() {
    let mut server = Server::new();
    let mut client = open_linked_in_memory();
    let (page, tag, _) = shared_page_and_tags(&mut server, &mut [&mut client]);
    server.commit_remote(&mut [&mut client], vec![add_tag(page, tag)]);

    let delete = queue(&mut client, vec![Operation::Delete(RecordId::Tag(tag))]);
    assert!(tag_names(&client, page).is_empty());
    assert!(search_ids(&client, "first").is_empty());
    client.reject_transaction(delete.id).unwrap();
    assert_eq!(tag_names(&client, page), vec!["first"]);
    assert_eq!(search_ids(&client, "first"), vec![page]);
    assert_eq!(records_of(&client), records_of(&server.store));

    let delete = queue(&mut client, vec![Operation::Delete(RecordId::Tag(tag))]);
    deliver(&mut server, &mut client, &mut [], &delete);
    assert_eq!(membership_row_count(&client), 0);
    assert_eq!(records_of(&client), records_of(&server.store));
}

#[test]
fn a_rejected_page_delete_brings_back_its_tags() {
    let mut server = Server::new();
    let mut client = open_linked_in_memory();
    let (page, tag, _) = shared_page_and_tags(&mut server, &mut [&mut client]);
    server.commit_remote(&mut [&mut client], vec![add_tag(page, tag)]);

    let delete = queue(&mut client, vec![Operation::Delete(RecordId::Page(page))]);
    let count_of_first = |client: &WorkspaceStore| {
        let usage = client.tags().unwrap();
        usage
            .into_iter()
            .find(|usage| usage.tag.id == tag)
            .unwrap()
            .page_count
    };
    assert_eq!(count_of_first(&client), 0);
    client.reject_transaction(delete.id).unwrap();
    assert_eq!(count_of_first(&client), 1);
    assert_eq!(tag_names(&client, page), vec!["first"]);
    assert_eq!(search_ids(&client, "first"), vec![page]);

    let delete = queue(&mut client, vec![Operation::Delete(RecordId::Page(page))]);
    deliver(&mut server, &mut client, &mut [], &delete);
    assert_eq!(membership_row_count(&client), 0);
}

#[test]
fn tagging_with_a_tag_deleted_meanwhile_is_rejected_and_leaves_nothing() {
    let mut server = Server::new();
    let (mut alice, mut bob) = (open_linked_in_memory(), open_linked_in_memory());
    let (page, tag, _) = shared_page_and_tags(&mut server, &mut [&mut alice, &mut bob]);

    let tagging = queue(&mut alice, vec![add_tag(page, tag)]);
    let delete = queue(&mut bob, vec![Operation::Delete(RecordId::Tag(tag))]);
    deliver(&mut server, &mut bob, &mut [&mut alice], &delete);
    deliver(&mut server, &mut alice, &mut [&mut bob], &tagging);

    for client in [&alice, &bob] {
        assert!(tag_names(client, page).is_empty());
        assert_eq!(membership_row_count(client), 0);
        assert_eq!(records_of(client), records_of(&server.store));
    }
}

#[test]
fn snapshots_carry_tags_and_memberships_and_replace_stale_ones() {
    let mut server = Server::new();
    let mut client = open_linked_in_memory();
    let (page, first, second) = shared_page_and_tags(&mut server, &mut [&mut client]);
    server.commit_remote(&mut [&mut client], vec![add_tag(page, first)]);

    // The server moved on without telling this client: first untagged, second tagged and renamed.
    let changes = Transaction::new(vec![
        remove_tag(page, first),
        add_tag(page, second),
        rename(second, "renamed"),
    ]);
    let seq = server.commit(&changes).unwrap();
    let snapshot = WorkspaceSnapshot {
        seq,
        ..server.store.export_workspace_snapshot().unwrap()
    };
    test_support::apply_snapshot(&mut client, snapshot).unwrap();

    assert_eq!(tag_names(&client, page), vec!["renamed"]);
    assert_eq!(search_ids(&client, "renamed"), vec![page]);
    assert!(search_ids(&client, "first").is_empty());
    assert_eq!(records_of(&client), records_of(&server.store));
}

#[test]
fn publishing_a_local_workspace_sends_tags_and_memberships_in_a_valid_order() {
    let mut local = open_in_memory();
    let page = create_page(&mut local, "Plan");
    let tag = create_tag(&mut local, "urgent");
    apply(&mut local, vec![add_tag(page, tag)]).unwrap();
    local
        .link_to_server("https://scroll.test", WorkspaceId::generate())
        .unwrap();

    let mut server = Server::new();
    for change in test_support::queued_changes(&local).unwrap() {
        if let crate::store::QueuedChange::Transaction(transaction) = change {
            server.commit(&transaction).unwrap();
        }
    }
    assert_eq!(records_of(&server.store), records_of(&local));
    assert_eq!(tag_names(&server.store, page), vec!["urgent"]);
}

#[test]
fn memberships_of_a_pending_deleted_page_stay_out_of_sight() {
    let mut client = open_linked_in_memory();
    let page = create_page(&mut client, "Plan");
    let tag = create_tag(&mut client, "urgent");
    apply(&mut client, vec![add_tag(page, tag)]).unwrap();
    apply(&mut client, vec![Operation::Delete(RecordId::Page(page))]).unwrap();

    assert_eq!(membership_row_count(&client), 1);
    assert_eq!(client.tags().unwrap()[0].page_count, 0);
    assert!(client.pages_with_tags(&[tag], 10).unwrap().is_empty());
    let exported = records_of(&client);
    assert!(exported.iter().all(|record| !record.starts_with("PageTag")));
}

#[test]
fn the_tag_colour_is_stored() {
    let mut store = open_in_memory();
    let tag = create_tag(&mut store, "urgent");
    apply(
        &mut store,
        vec![Operation::Set(FieldChange::Tag {
            id: tag,
            field: TagField::Color(TagColor::Red),
        })],
    )
    .unwrap();
    assert_eq!(store.tag(tag).unwrap().unwrap().color, TagColor::Red);
}

#[test]
fn a_remote_tag_delete_settles_a_pending_re_add_of_one_of_its_memberships() {
    let mut server = Server::new();
    let mut client = open_linked_in_memory();
    let (page, tag, _) = shared_page_and_tags(&mut server, &mut [&mut client]);
    server.commit_remote(&mut [&mut client], vec![add_tag(page, tag)]);

    let re_add = queue(&mut client, vec![add_tag(page, tag)]);
    server.commit_remote(
        &mut [&mut client],
        vec![Operation::Delete(RecordId::Tag(tag))],
    );
    deliver(&mut server, &mut client, &mut [], &re_add);

    assert_eq!(membership_row_count(&client), 0);
    assert_eq!(records_of(&client), records_of(&server.store));
}

#[test]
fn a_snapshot_removes_a_membership_hidden_behind_a_pending_tag_delete() {
    let mut server = Server::new();
    let mut client = open_linked_in_memory();
    let (page, tag, _) = shared_page_and_tags(&mut server, &mut [&mut client]);
    server.commit_remote(&mut [&mut client], vec![add_tag(page, tag)]);

    let delete = queue(&mut client, vec![Operation::Delete(RecordId::Tag(tag))]);
    let seq = server
        .commit(&Transaction::new(vec![remove_tag(page, tag)]))
        .unwrap();
    let snapshot = WorkspaceSnapshot {
        seq,
        ..server.store.export_workspace_snapshot().unwrap()
    };
    test_support::apply_snapshot(&mut client, snapshot).unwrap();
    client.reject_transaction(delete.id).unwrap();

    assert!(tag_names(&client, page).is_empty());
    assert_eq!(records_of(&client), records_of(&server.store));
}
