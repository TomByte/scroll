mod common;

use std::collections::HashMap;

use common::{SyncClient, TestServer};
use scroll_api::protocol::{ClientMessage, PROTOCOL_VERSION, RejectReason, ServerMessage};
use scroll_core::id::PageId;
use scroll_core::store::{PageCatchUp, SequencedBytes};
use uuid::Uuid;

const CLOSE_POLICY: u16 = 1008;
const CLOSE_PROTOCOL: u16 = 1002;
const CLOSE_RESTART: u16 = 1012;
const CLOSE_AGAIN: u16 = 1013;

fn sequenced(seq: u64, bytes: &[u8]) -> SequencedBytes {
    SequencedBytes {
        seq,
        bytes: bytes.to_vec(),
    }
}

async fn assert_rejected_and_closed(
    client: &mut SyncClient,
    reason: RejectReason,
    close_code: u16,
) {
    assert_eq!(client.receive().await, ServerMessage::Rejected { reason });
    let close = client.receive_close().await.expect("a close frame");
    assert_eq!(u16::from(close.code), close_code);
}

#[tokio::test]
async fn a_pushed_update_is_acked_with_the_next_seq_and_relayed_to_other_subscribers() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let page_id = PageId::generate();
    let mut owner_client = server.connect(&owner).await;
    let mut editor_client = server.connect(&editor).await;
    let empty = PageCatchUp {
        snapshot: None,
        updates: vec![],
    };
    assert_eq!(
        owner_client.open_page(workspace_id, page_id, 0).await,
        empty
    );
    assert_eq!(
        editor_client.open_page(workspace_id, page_id, 0).await,
        empty
    );

    assert_eq!(owner_client.push(page_id, b"one").await, 1);
    assert_eq!(
        editor_client.receive().await,
        ServerMessage::RemotePageUpdate {
            page_id,
            seq: 1,
            author: owner.user_id,
            update: b"one".to_vec()
        }
    );

    assert_eq!(editor_client.push(page_id, b"two").await, 2);
    assert_eq!(
        owner_client.receive().await,
        ServerMessage::RemotePageUpdate {
            page_id,
            seq: 2,
            author: editor.user_id,
            update: b"two".to_vec()
        }
    );
    owner_client.assert_silent().await;
    editor_client.assert_silent().await;
}

#[tokio::test]
async fn reopening_a_page_returns_only_the_updates_after_last_seen_seq() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    writer.open_page(workspace_id, page_id, 0).await;
    for update in [b"a", b"b", b"c"] {
        writer.push(page_id, update).await;
    }
    writer.close().await;

    let mut reader = server.connect(&owner).await;
    let after_one = reader.open_page(workspace_id, page_id, 1).await;
    assert_eq!(
        after_one,
        PageCatchUp {
            snapshot: None,
            updates: vec![sequenced(2, b"b"), sequenced(3, b"c")]
        }
    );
    let everything = reader.open_page(workspace_id, page_id, 0).await;
    assert_eq!(everything.updates.len(), 3);
    let nothing_new = reader.open_page(workspace_id, page_id, 3).await;
    assert!(nothing_new.updates.is_empty());
}

#[tokio::test]
async fn a_resent_update_gets_the_same_ack_and_is_neither_stored_nor_relayed_again() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    let mut watcher = server.connect(&owner).await;
    writer.open_page(workspace_id, page_id, 0).await;
    watcher.open_page(workspace_id, page_id, 0).await;
    let update_id = Uuid::now_v7();

    writer.send_update(update_id, page_id, b"once").await;
    assert_eq!(writer.receive_ack(update_id).await, 1);
    assert!(matches!(
        watcher.receive().await,
        ServerMessage::RemotePageUpdate { seq: 1, .. }
    ));

    writer.send_update(update_id, page_id, b"once").await;
    assert_eq!(writer.receive_ack(update_id).await, 1);
    watcher.assert_silent().await;

    // A resend after reconnecting, as from the client's offline queue, is recognised too.
    writer.close().await;
    assert!(matches!(
        watcher.receive().await,
        ServerMessage::PresenceLeft { .. }
    ));
    let mut reconnected = server.connect(&owner).await;
    reconnected.open_page(workspace_id, page_id, 1).await;
    reconnected.send_update(update_id, page_id, b"once").await;
    assert_eq!(reconnected.receive_ack(update_id).await, 1);
    watcher.assert_silent().await;

    assert_eq!(reconnected.push(page_id, b"next").await, 2);
    let stored = reconnected.open_page(workspace_id, page_id, 0).await;
    assert_eq!(
        stored.updates,
        vec![sequenced(1, b"once"), sequenced(2, b"next")]
    );
}

#[tokio::test]
async fn a_viewer_can_read_but_not_push() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let viewer = server.new_member(&owner, workspace_id, "Viewer").await;
    let page_id = PageId::generate();
    let mut owner_client = server.connect(&owner).await;
    owner_client.open_page(workspace_id, page_id, 0).await;
    owner_client.push(page_id, b"by the owner").await;
    let mut viewer_client = server.connect(&viewer).await;

    let catch_up = viewer_client.open_page(workspace_id, page_id, 0).await;
    assert_eq!(catch_up.updates, vec![sequenced(1, b"by the owner")]);
    let update_id = Uuid::now_v7();
    viewer_client
        .send_update(update_id, page_id, b"by the viewer")
        .await;

    assert_eq!(
        viewer_client.receive().await,
        ServerMessage::PageUpdateRejected {
            update_id,
            page_id,
            reason: RejectReason::ReadOnly
        }
    );
    owner_client.assert_silent().await;
    assert_eq!(owner_client.push(page_id, b"still the owner").await, 2);
}

#[tokio::test]
async fn a_non_member_cannot_open_a_page_or_push_and_presence_needs_an_open_page() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let outsider = server.new_user().await;
    let page_id = PageId::generate();
    let mut owner_client = server.connect(&owner).await;
    owner_client.open_page(workspace_id, page_id, 0).await;
    owner_client.push(page_id, b"private").await;
    let mut outsider_client = server.connect(&outsider).await;

    outsider_client
        .send(&ClientMessage::OpenPage {
            workspace_id,
            page_id,
            last_seen_seq: 0,
        })
        .await;
    assert_eq!(
        outsider_client.receive().await,
        ServerMessage::PageRejected {
            page_id,
            reason: RejectReason::NotAMember
        }
    );

    let update_id = Uuid::now_v7();
    outsider_client
        .send_update(update_id, page_id, b"intrusion")
        .await;
    assert_eq!(
        outsider_client.receive().await,
        ServerMessage::PageUpdateRejected {
            update_id,
            page_id,
            reason: RejectReason::PageNotOpen
        }
    );
    outsider_client
        .send(&ClientMessage::Presence {
            page_id,
            state: b"here".to_vec(),
        })
        .await;
    assert_eq!(
        outsider_client.receive().await,
        ServerMessage::PageRejected {
            page_id,
            reason: RejectReason::PageNotOpen
        }
    );
    owner_client.assert_silent().await;
}

#[tokio::test]
async fn a_page_id_cannot_be_used_in_a_second_workspace() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let first_workspace = server.create_workspace(&owner).await;
    let second_workspace = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut first = server.connect(&owner).await;
    let mut second = server.connect(&owner).await;
    // Both open the page before it exists, so both are subscribed when the first push creates it.
    first.open_page(first_workspace, page_id, 0).await;
    second.open_page(second_workspace, page_id, 0).await;

    first.push(page_id, b"belongs to the first workspace").await;
    second.assert_silent().await;

    let update_id = Uuid::now_v7();
    second
        .send_update(update_id, page_id, b"belongs to the second")
        .await;
    assert_eq!(
        second.receive().await,
        ServerMessage::PageUpdateRejected {
            update_id,
            page_id,
            reason: RejectReason::PageInAnotherWorkspace
        }
    );
    second
        .send(&ClientMessage::OpenPage {
            workspace_id: second_workspace,
            page_id,
            last_seen_seq: 0,
        })
        .await;
    assert_eq!(
        second.receive().await,
        ServerMessage::PageRejected {
            page_id,
            reason: RejectReason::PageInAnotherWorkspace
        }
    );
    first.assert_silent().await;
}

#[tokio::test]
async fn hello_must_come_first_with_the_current_protocol_version_and_a_live_session() {
    let server = TestServer::start().await;
    let user = server.new_user().await;

    let mut old_client = server.connect_without_hello().await;
    old_client.hello(PROTOCOL_VERSION + 1, &user.token).await;
    assert_rejected_and_closed(
        &mut old_client,
        RejectReason::UnsupportedProtocolVersion,
        CLOSE_POLICY,
    )
    .await;

    let mut bad_token = server.connect_without_hello().await;
    bad_token
        .hello(PROTOCOL_VERSION, "not-a-session-token")
        .await;
    assert_rejected_and_closed(&mut bad_token, RejectReason::InvalidSession, CLOSE_POLICY).await;

    let mut no_hello = server.connect_without_hello().await;
    no_hello
        .send(&ClientMessage::ClosePage {
            page_id: PageId::generate(),
        })
        .await;
    assert_rejected_and_closed(
        &mut no_hello,
        RejectReason::ProtocolViolation,
        CLOSE_PROTOCOL,
    )
    .await;

    let mut second_hello = server.connect(&user).await;
    second_hello.hello(PROTOCOL_VERSION, &user.token).await;
    assert_rejected_and_closed(
        &mut second_hello,
        RejectReason::ProtocolViolation,
        CLOSE_PROTOCOL,
    )
    .await;

    server
        .http
        .delete(server.url("/sessions/current"))
        .bearer_auth(&user.token)
        .send()
        .await
        .unwrap();
    let mut signed_out = server.connect_without_hello().await;
    signed_out.hello(PROTOCOL_VERSION, &user.token).await;
    assert_rejected_and_closed(&mut signed_out, RejectReason::InvalidSession, CLOSE_POLICY).await;
}

#[tokio::test]
async fn presence_is_relayed_per_connection_and_leaving_is_announced() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let page_id = PageId::generate();
    let mut laptop = server.connect(&owner).await;
    let mut phone = server.connect(&owner).await;
    let mut editor_client = server.connect(&editor).await;
    for client in [&mut laptop, &mut phone, &mut editor_client] {
        client.open_page(workspace_id, page_id, 0).await;
    }
    let laptop_id = laptop.connection_id.unwrap();
    let phone_id = phone.connection_id.unwrap();
    assert_ne!(laptop_id, phone_id);

    laptop
        .send(&ClientMessage::Presence {
            page_id,
            state: b"cursor at 3".to_vec(),
        })
        .await;
    let from_laptop = ServerMessage::Presence {
        page_id,
        user_id: owner.user_id,
        connection_id: laptop_id,
        state: b"cursor at 3".to_vec(),
    };
    assert_eq!(editor_client.receive().await, from_laptop);
    assert_eq!(phone.receive().await, from_laptop);
    laptop.assert_silent().await;

    laptop.send(&ClientMessage::ClosePage { page_id }).await;
    let laptop_left = ServerMessage::PresenceLeft {
        page_id,
        user_id: owner.user_id,
        connection_id: laptop_id,
    };
    assert_eq!(editor_client.receive().await, laptop_left);
    assert_eq!(phone.receive().await, laptop_left);

    phone.close().await;
    assert_eq!(
        editor_client.receive().await,
        ServerMessage::PresenceLeft {
            page_id,
            user_id: owner.user_id,
            connection_id: phone_id,
        }
    );
}

#[tokio::test]
async fn concurrent_pushes_get_unique_gapless_seqs_delivered_in_order() {
    const WRITERS: usize = 6;
    const UPDATES_PER_WRITER: usize = 25;
    const TOTAL: u64 = (WRITERS * UPDATES_PER_WRITER) as u64;
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut observer = server.connect(&owner).await;
    observer.open_page(workspace_id, page_id, 0).await;
    let mut writers = Vec::new();
    for _ in 0..WRITERS {
        let editor = server.new_member(&owner, workspace_id, "Editor").await;
        let mut client = server.connect(&editor).await;
        client.open_page(workspace_id, page_id, 0).await;
        writers.push(client);
    }

    let tasks: Vec<_> = writers
        .into_iter()
        .map(|mut client| {
            tokio::spawn(async move {
                let update_ids: Vec<Uuid> =
                    (0..UPDATES_PER_WRITER).map(|_| Uuid::now_v7()).collect();
                for update_id in &update_ids {
                    client
                        .send_update(*update_id, page_id, update_id.as_bytes())
                        .await;
                }
                let mut acked_seqs = HashMap::new();
                let mut seqs_in_arrival_order = Vec::new();
                while seqs_in_arrival_order.len() < TOTAL as usize {
                    let seq = match client.receive().await {
                        ServerMessage::PageUpdateAck { update_id, seq, .. } => {
                            acked_seqs.insert(update_id, seq);
                            seq
                        }
                        ServerMessage::RemotePageUpdate { seq, .. } => seq,
                        other => panic!("unexpected {other:?}"),
                    };
                    seqs_in_arrival_order.push(seq);
                }
                assert_eq!(acked_seqs.len(), UPDATES_PER_WRITER);
                (acked_seqs, seqs_in_arrival_order)
            })
        })
        .collect();

    let mut all_acked: Vec<u64> = Vec::new();
    for task in tasks {
        let (acked_seqs, seqs_in_arrival_order) = task.await.unwrap();
        assert_eq!(
            seqs_in_arrival_order,
            (1..=TOTAL).collect::<Vec<_>>(),
            "each writer sees every seq in order"
        );
        all_acked.extend(acked_seqs.into_values());
    }
    all_acked.sort_unstable();
    assert_eq!(all_acked, (1..=TOTAL).collect::<Vec<_>>());
    for expected in 1..=TOTAL {
        assert!(
            matches!(observer.receive().await, ServerMessage::RemotePageUpdate { seq, .. } if seq == expected)
        );
    }
}

#[tokio::test]
async fn a_client_that_stops_reading_does_not_hold_up_the_others() {
    const UPDATES: usize = 300;
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut writer = server.connect(&owner).await;
    let mut reader = server.connect(&owner).await;
    let mut stalled = server.connect(&owner).await;
    for client in [&mut writer, &mut reader, &mut stalled] {
        client.open_page(workspace_id, page_id, 0).await;
    }
    let large_update = vec![7u8; 64 * 1024];

    for expected_seq in 1..=UPDATES as u64 {
        assert_eq!(writer.push(page_id, &large_update).await, expected_seq);
        assert!(
            matches!(reader.receive_skipping_presence().await, ServerMessage::RemotePageUpdate { seq, .. } if seq == expected_seq)
        );
    }

    // Reading again within the server's send timeout, it is told to reconnect promptly. When this loop outlasts that
    // timeout, the server has already dropped it without a close frame it could not have read.
    if let Some(close) = stalled.skip_to_close().await {
        assert_eq!(u16::from(close.code), CLOSE_AGAIN);
    }
}

#[tokio::test]
async fn shutting_down_closes_sync_connections() {
    let mut server = TestServer::start().await;
    let user = server.new_user().await;
    let mut client = server.connect(&user).await;

    server.shut_down();

    let close = client.receive_close().await.expect("a close frame");
    assert_eq!(u16::from(close.code), CLOSE_RESTART);
}

#[tokio::test]
async fn an_ack_is_delivered_even_if_the_page_is_reopened_before_it_is_read() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let page_id = PageId::generate();
    let mut client = server.connect(&owner).await;
    client.open_page(workspace_id, page_id, 0).await;
    let update_id = Uuid::now_v7();

    client.send_update(update_id, page_id, b"in flight").await;
    client
        .send(&ClientMessage::OpenPage {
            workspace_id,
            page_id,
            last_seen_seq: 0,
        })
        .await;

    assert_eq!(client.receive_ack(update_id).await, 1);
    match client.receive().await {
        ServerMessage::PageCatchUp { catch_up, .. } => {
            assert_eq!(catch_up.updates, vec![sequenced(1, b"in flight")])
        }
        other => panic!("expected the reopened page's catch-up, got {other:?}"),
    }
}
