//! The sync engine against a real server: convergence, offline work, rejections, publishing, cut
//! connections, signing out and the open-database limit.

mod common;

use std::time::Duration;

use common::{
    Proxy, Team, TempDir, TestApp, TestServer, new_page, sorted_records, wait_for_status,
    wait_until,
};
use scroll_api::limits::MAX_OPEN_DATABASES;
use scroll_api::protocol::TransactionRejection;
use scroll_core::id::{DatabaseId, PageId, PropertyId, ViewId};
use scroll_core::operation::{FieldChange, Operation, PageField, Record, RecordId};
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue, View, ViewType};
use scroll_core::store::WorkspaceStore;
use scroll_core::validation::ValidationError;
use scroll_sync::{ConnectionStatus, SyncEngine, SyncEvent, publish_workspace};

fn set_title(page_id: PageId, title: &str) -> Operation {
    Operation::Set(FieldChange::Page {
        id: page_id,
        field: PageField::Title(title.into()),
    })
}

fn online_count(app: &TestApp) -> usize {
    app.statuses
        .iter()
        .filter(|status| **status == ConnectionStatus::Online)
        .count()
}

fn title_of(app: &TestApp, page_id: PageId) -> Option<String> {
    app.store.page(page_id).unwrap().map(|page| page.title)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_clients_converge_on_records_and_concurrently_edited_pages() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    owner.start(&team.owner.session_token);
    editor.start(&team.editor.session_token);

    let page = owner.create_page("Plan");
    // Depending on timing the page arrives as a remote transaction or in the first snapshot.
    let is_told_of_page = |app: &TestApp| {
        app.log.iter().any(|event| match event {
            SyncEvent::RecordsChanged(changes) => {
                changes.records.contains(&RecordId::Page(page.id))
                    && changes.child_lists.contains(&None)
            }
            SyncEvent::AllRecordsChanged => true,
            _ => false,
        })
    };
    wait_until(&mut [&mut editor], |apps| {
        title_of(apps[0], page.id).is_some() && is_told_of_page(apps[0])
    })
    .await;
    editor.commit(vec![set_title(page.id, "Plan B")]);
    wait_until(&mut [&mut owner], |apps| {
        title_of(apps[0], page.id).as_deref() == Some("Plan B")
    })
    .await;

    owner.open_page(page.id);
    editor.open_page(page.id);
    for step in 0..5 {
        owner.write_paragraph(page.id, &format!("owner {step}"));
        editor.write_paragraph(page.id, &format!("editor {step}"));
    }
    wait_until(&mut [&mut owner, &mut editor], |apps| {
        apps.iter().all(|app| app.is_settled())
            && apps[0].live_text(page.id) == apps[1].live_text(page.id)
            && apps[0].live_text(page.id).lines().count() == 10
    })
    .await;
    let text = owner.live_text(page.id);
    assert_eq!(owner.stored_text(page.id), text);
    assert_eq!(editor.stored_text(page.id), text);
    assert_eq!(sorted_records(&owner.store), sorted_records(&editor.store));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_done_offline_is_caught_up_and_uploaded_on_reconnect() {
    let mut server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    owner.start(&team.owner.session_token);
    editor.start(&team.editor.session_token);
    let shared = owner.create_page("Shared");
    wait_until(&mut [&mut editor], |apps| {
        title_of(apps[0], shared.id).is_some()
    })
    .await;

    // The server goes away; both keep working.
    server.shut_down().await;
    wait_for_status(&mut owner, ConnectionStatus::Offline).await;
    let offline_page = owner.create_page("Written offline");
    owner.open_page(shared.id);
    owner.write_paragraph(shared.id, "typed on a plane");
    editor.stop();
    editor.commit(vec![set_title(shared.id, "Shared, renamed offline")]);

    server.restart().await;
    editor.start(&team.editor.session_token);
    wait_until(&mut [&mut owner, &mut editor], |apps| {
        apps.iter().all(|app| app.is_settled())
            && title_of(apps[1], offline_page.id).is_some()
            && title_of(apps[0], shared.id).as_deref() == Some("Shared, renamed offline")
    })
    .await;
    assert_eq!(sorted_records(&owner.store), sorted_records(&editor.store));
    editor.open_page(shared.id);
    wait_until(&mut [&mut editor], |apps| {
        apps[0].live_text(shared.id) == "typed on a plane"
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_transaction_is_undone_locally_and_reported() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    owner.start(&team.owner.session_token);
    editor.start(&team.editor.session_token);
    let doomed = owner.create_page("Doomed");
    wait_until(&mut [&mut editor], |apps| {
        title_of(apps[0], doomed.id).is_some()
    })
    .await;

    // The editor renames the page offline while the owner deletes it.
    editor.stop();
    editor.commit(vec![set_title(doomed.id, "Saved?")]);
    owner.commit(vec![Operation::Delete(RecordId::Page(doomed.id))]);
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;
    editor.start(&team.editor.session_token);

    let rejection = |app: &TestApp| {
        app.log.iter().find_map(|event| match event {
            SyncEvent::TransactionRejected {
                reason, records, ..
            } => Some((reason.clone(), records.clone())),
            _ => None,
        })
    };
    wait_until(&mut [&mut editor], |apps| {
        apps[0].is_settled() && rejection(apps[0]).is_some()
    })
    .await;
    assert_eq!(
        rejection(&editor),
        Some((
            TransactionRejection::Invalid(ValidationError::PageNotFound(doomed.id)),
            vec![RecordId::Page(doomed.id)]
        ))
    );
    assert_eq!(title_of(&editor, doomed.id), None);
    assert_eq!(sorted_records(&owner.store), sorted_records(&editor.store));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_showing_a_table_gets_its_row_changes_at_once_and_the_rest_in_order() {
    // Row changes of databases a device does not show then reach it only with other changes.
    let server = TestServer::start_with_deferred_delivery(Duration::from_secs(600)).await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    owner.start(&team.owner.session_token);
    editor.start(&team.editor.session_token);
    let host = new_page("Tasks", None);
    let property = Property {
        id: PropertyId::generate(),
        name: "Estimate".into(),
        property_type: PropertyType::Number,
        options: vec![],
    };
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![property.clone()],
    };
    let row = Page {
        row_of: Some(database.id),
        ..new_page("Row", None)
    };
    owner.commit(vec![
        Operation::Create(Record::Page(host.clone())),
        Operation::Create(Record::Database(database.clone())),
        Operation::Create(Record::Page(row.clone())),
    ]);
    wait_until(&mut [&mut editor], |apps| {
        title_of(apps[0], row.id).is_some()
    })
    .await;
    let set_estimate = |estimate| {
        vec![Operation::Set(FieldChange::PropertyValue {
            page: row.id,
            property: property.id,
            value: Some(PropertyValue::Number(estimate)),
        })]
    };
    let estimate_of = |app: &TestApp| app.store.property_values(row.id).unwrap();

    editor.engine().open_database(database.id);
    round_trip(&mut editor, host.id).await;
    owner.commit(set_estimate(1.0));
    wait_until(&mut [&mut editor], |apps| {
        estimate_of(apps[0]) == vec![(property.id, PropertyValue::Number(1.0))]
    })
    .await;

    editor.engine().close_database(database.id);
    round_trip(&mut editor, row.id).await;
    owner.commit(set_estimate(2.0));
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    editor.pump();
    assert_eq!(
        estimate_of(&editor),
        vec![(property.id, PropertyValue::Number(1.0))],
        "held back while the table is not shown"
    );
    owner.commit(vec![set_title(host.id, "Tasks, renamed")]);
    wait_until(&mut [&mut editor], |apps| {
        title_of(apps[0], host.id).as_deref() == Some("Tasks, renamed")
    })
    .await;
    assert_eq!(
        estimate_of(&editor),
        vec![(property.id, PropertyValue::Number(2.0))],
        "delivered, in order, before the rename"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_more_databases_than_a_connection_may_keeps_the_connection() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    owner.start(&team.owner.session_token);
    let page = owner.create_page("Plan");
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;

    for _ in 0..=MAX_OPEN_DATABASES {
        owner.engine().open_database(DatabaseId::generate());
    }
    round_trip(&mut owner, page.id).await;
    owner.commit(vec![set_title(page.id, "Plan B")]);
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;

    assert_eq!(
        online_count(&owner),
        1,
        "the server never rejected the connection for opening too many"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_linked_workspace_reports_its_first_catch_up_complete() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    editor.start(&team.editor.session_token);
    assert!(editor.engine().is_first_catch_up_pending());

    let is_complete = |app: &TestApp| {
        app.log
            .iter()
            .any(|event| matches!(event, SyncEvent::FirstCatchUpComplete))
    };
    wait_until(&mut [&mut editor], |apps| is_complete(apps[0])).await;
    assert!(!editor.engine().is_first_catch_up_pending());

    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    owner.start(&team.owner.session_token);
    let page = owner.create_page("Plan");
    wait_until(&mut [&mut editor], |apps| {
        title_of(apps[0], page.id).is_some()
    })
    .await;
    editor.stop();
    editor.start(&team.editor.session_token);
    assert!(
        !editor.engine().is_first_catch_up_pending(),
        "a file that has received the workspace knows it after a restart"
    );
}

/// Opens a page and waits for its catch-up. The engine sends commands in order on one connection and the server
/// answers them in order, so commands given before this have reached the server.
async fn round_trip(app: &mut TestApp, page_id: PageId) {
    app.open_page(page_id);
    wait_until(&mut [app], |apps| {
        apps[0].log.iter().any(|event| {
            matches!(event, SyncEvent::PageCatchUp { page_id: caught_up, .. } if *caught_up == page_id)
        })
    })
    .await;
}

/// Pages, a page tree, a database with a view, a row with a value, and page content, in a local-only file.
fn populate_local_workspace(store: &mut WorkspaceStore) -> PageId {
    store.set_workspace_name("Published").unwrap();
    let top = new_page("Top", None);
    let host = new_page("Tasks", Some(top.id));
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
    let row = Page {
        row_of: Some(database.id),
        ..new_page("Write the report", Some(host.id))
    };
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: vec![],
        sort: vec![],
        visible_columns: vec![property.id],
        column_widths: vec![],
    };
    let operations = vec![
        Operation::Create(Record::Page(top.clone())),
        Operation::Create(Record::Page(host)),
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
        .apply_local_transaction(&scroll_core::operation::Transaction::new(operations))
        .unwrap();
    let mut doc = store.load_page_doc(top.id).unwrap();
    let block = doc
        .insert_block(
            scroll_core::page_doc::BlockPlacement::LastChildOf(None),
            scroll_core::page_doc::BlockKind::Paragraph,
        )
        .unwrap();
    doc.insert_text(block, 0, "written before publishing")
        .unwrap();
    let update = doc.take_local_update().unwrap().unwrap();
    store.record_local_page_update(top.id, &update).unwrap();
    top.id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_published_local_workspace_is_reproduced_on_another_device() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let owner_session = server.new_user(&accounts, None).await;
    let files = TempDir::new();
    let mut laptop = TestApp::open(files.file("laptop.scroll"));
    let page_with_content = populate_local_workspace(&mut laptop.store);

    let membership = publish_workspace(&accounts, &owner_session.session_token, &mut laptop.store)
        .await
        .unwrap();
    assert_eq!(membership.name, "Published");
    laptop.start(&owner_session.session_token);
    wait_until(&mut [&mut laptop], |apps| apps[0].is_settled()).await;

    let mut desktop = TestApp::join(files.file("desktop.scroll"), &server.url(), &membership);
    desktop.start(&owner_session.session_token);
    wait_until(&mut [&mut desktop], |apps| {
        apps[0]
            .log
            .iter()
            .any(|event| matches!(event, SyncEvent::AllRecordsChanged))
    })
    .await;
    assert_eq!(
        sorted_records(&laptop.store),
        sorted_records(&desktop.store)
    );
    desktop.open_page(page_with_content);
    wait_until(&mut [&mut desktop], |apps| {
        apps[0].live_text(page_with_content) == "written before publishing"
    })
    .await;
    assert_eq!(
        desktop.stored_text(page_with_content),
        "written before publishing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cutting_the_connection_mid_upload_loses_and_duplicates_nothing() {
    const PAGES: usize = 150;
    const PARAGRAPHS: usize = 40;
    let server = TestServer::start().await;
    let proxy = Proxy::start(server.address).await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &proxy.url(),
        &team.owner_membership,
    );
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    let notes = owner.create_page("Notes");
    owner.open_page(notes.id);
    owner.start(&team.owner.session_token);
    editor.start(&team.editor.session_token);

    // Each batch is queued at once and the connection cut while it is still uploading.
    const CUTS: usize = 5;
    for batch in 0..CUTS {
        wait_until(&mut [&mut owner], |apps| online_count(apps[0]) > batch).await;
        for index in (batch * PAGES / CUTS)..((batch + 1) * PAGES / CUTS) {
            owner.create_page(&format!("Page {index}"));
            if index < PARAGRAPHS {
                owner.write_paragraph(notes.id, &format!("paragraph {index}"));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        proxy.cut_connections();
    }
    // The last cut can come after everything was acknowledged; the engine reconnects all the same.
    wait_until(&mut [&mut owner], |apps| {
        apps[0].is_settled() && online_count(apps[0]) > CUTS
    })
    .await;

    let workspace_id = team.owner_membership.workspace_id.0;
    let database = server.pool.get().await.unwrap();
    let committed: i64 = database
        .query_one(
            "SELECT count(*) FROM record_transactions WHERE workspace_id = $1",
            &[&workspace_id],
        )
        .await
        .unwrap()
        .get(0);
    let stored_updates: i64 = database
        .query_one(
            "SELECT count(*) FROM page_updates WHERE page_id = $1",
            &[&notes.id.0],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(committed as usize, PAGES + 1);
    assert_eq!(stored_updates as usize, PARAGRAPHS);
    assert!(
        !owner
            .log
            .iter()
            .any(|event| matches!(event, SyncEvent::TransactionRejected { .. }))
    );

    editor.open_page(notes.id);
    wait_until(&mut [&mut editor], |apps| {
        apps[0].store.last_seen_workspace_seq().unwrap() == (PAGES + 1) as u64
            && apps[0].live_text(notes.id).lines().count() == PARAGRAPHS
    })
    .await;
    assert_eq!(sorted_records(&owner.store), sorted_records(&editor.store));
    assert_eq!(owner.live_text(notes.id), editor.live_text(notes.id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_out_stops_the_engine_as_signed_out() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    owner.start(&team.owner.session_token);
    wait_for_status(&mut owner, ConnectionStatus::Online).await;

    accounts.sign_out(&team.owner.session_token).await.unwrap();

    wait_for_status(&mut owner, ConnectionStatus::SignedOut).await;
    owner.stop();
    assert_eq!(owner.status(), Some(ConnectionStatus::SignedOut));

    let (_engine, mut events) =
        SyncEngine::start(&owner.path, "not a session token".into(), None).unwrap();
    let mut statuses = Vec::new();
    while let Some(event) = events.next().await {
        if let SyncEvent::Status(status) = event {
            statuses.push(status);
        }
    }
    assert_eq!(
        statuses,
        vec![ConnectionStatus::Connecting, ConnectionStatus::SignedOut]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_app_and_engine_write_to_one_file_at_once_without_errors() {
    const CHANGES: usize = 200;
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    let mut editor = TestApp::join(
        files.file("editor.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    owner.start(&team.owner.session_token);
    editor.start(&team.editor.session_token);

    // The editor's engine writes the owner's pages while the editor's app writes its own.
    let writer = tokio::task::spawn_blocking(move || {
        for index in 0..CHANGES {
            owner.create_page(&format!("Owner {index}"));
        }
        owner
    });
    for index in 0..CHANGES {
        editor.create_page(&format!("Editor {index}"));
        editor.pump();
    }
    let mut owner = writer.await.unwrap();
    wait_until(&mut [&mut owner, &mut editor], |apps| {
        apps.iter().all(|app| {
            app.is_settled() && app.store.last_seen_workspace_seq().unwrap() == 2 * CHANGES as u64
        })
    })
    .await;
    assert_eq!(sorted_records(&owner.store), sorted_records(&editor.store));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_viewers_page_edits_are_refused_dropped_and_the_page_reloaded() {
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let invite = accounts
        .create_invite(
            &team.owner.session_token,
            team.owner_membership.workspace_id,
            scroll_core::id::Role::Viewer,
        )
        .await
        .unwrap();
    let viewer_session = server.new_user(&accounts, Some(invite)).await;
    let viewer_membership = accounts
        .workspaces(&viewer_session.session_token)
        .await
        .unwrap()
        .remove(0);
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    let mut viewer = TestApp::join(
        files.file("viewer.scroll"),
        &server.url(),
        &viewer_membership,
    );
    owner.start(&team.owner.session_token);
    viewer.start(&viewer_session.session_token);
    let page = owner.create_page("Read me");
    owner.open_page(page.id);
    owner.write_paragraph(page.id, "by the owner");
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;
    wait_until(&mut [&mut viewer], |apps| {
        title_of(apps[0], page.id).is_some()
    })
    .await;
    viewer.open_page(page.id);
    wait_until(&mut [&mut viewer], |apps| {
        apps[0].live_text(page.id) == "by the owner"
    })
    .await;

    viewer.write_paragraph(page.id, "by the viewer");

    wait_until(&mut [&mut viewer], |apps| {
        apps[0].is_settled() && apps[0].live_text(page.id) == "by the owner"
    })
    .await;
    assert!(viewer.log.iter().any(|event| matches!(
        event,
        SyncEvent::PageEditsRefused { page_id, reason: scroll_sync::EditRefusal::ReadOnly }
            if *page_id == page.id
    )));
    assert_eq!(viewer.stored_text(page.id), "by the owner");
    assert_eq!(owner.live_text(page.id), "by the owner");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_join_stopped_while_applying_its_snapshot_resumes_without_downloading_it_again() {
    // More pages than the store applies in one batch, so the finish can stop part-way.
    const PAGES: usize = 6_000;
    let server = TestServer::start().await;
    let accounts = server.accounts();
    let team = Team::create(&server, &accounts).await;
    let files = TempDir::new();
    let mut owner = TestApp::join(
        files.file("owner.scroll"),
        &server.url(),
        &team.owner_membership,
    );
    owner.start(&team.owner.session_token);
    let pages: Vec<Operation> = (0..PAGES)
        .map(|index| Operation::Create(Record::Page(new_page(&format!("Page {index}"), None))))
        .collect();
    owner.commit(pages);
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;
    let snapshot = owner.store.export_workspace_snapshot().unwrap();

    // The download completed and the finish was cut off after its first batch.
    let mut desktop = TestApp::join(
        files.file("desktop.scroll"),
        &server.url(),
        &team.editor_membership,
    );
    desktop
        .store
        .begin_workspace_snapshot(snapshot.seq)
        .unwrap();
    desktop
        .store
        .stage_workspace_snapshot_part(&snapshot)
        .unwrap();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        desktop
            .store
            .finish_workspace_snapshot_reporting(snapshot.seq, |progress| {
                assert!(progress.applied == 0, "stopping after the first batch");
            })
    }));
    assert!(interrupted.is_err());
    assert_eq!(
        desktop.store.unfinished_snapshot().unwrap(),
        Some(snapshot.seq)
    );
    let later = owner.create_page("Made after the snapshot");
    wait_until(&mut [&mut owner], |apps| apps[0].is_settled()).await;

    desktop.start(&team.editor.session_token);
    // Reported after the resumed finish's progress, so every `ApplyProgress` is in the log by then.
    wait_until(&mut [&mut desktop], |apps| {
        apps[0].log.iter().any(|event| {
            matches!(event, SyncEvent::RecordsChanged(changes) if changes.records.contains(&RecordId::Page(later.id)))
        })
    })
    .await;
    assert_eq!(desktop.store.unfinished_snapshot().unwrap(), None);
    let applied: Vec<usize> = desktop
        .log
        .iter()
        .filter_map(|event| match event {
            SyncEvent::ApplyProgress { applied, .. } => Some(*applied),
            _ => None,
        })
        .collect();
    // A snapshot downloaded again would be applied again from nothing, reporting 0 first.
    assert!(
        !applied.is_empty() && applied.iter().all(|applied| *applied > 0),
        "the finish resumed past its first batch and nothing was applied from scratch: {applied:?}"
    );
    assert_eq!(sorted_records(&owner.store), sorted_records(&desktop.store));
}
