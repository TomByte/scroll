//! Live connections losing access, and limits on sign-in and sign-up attempts.

mod common;

use std::time::Duration;

use common::{PASSWORD, SyncClient, TestServer, unique_email};
use reqwest::{StatusCode, header};
use scroll_api::protocol::{ClientMessage, RejectReason, ServerMessage};
use scroll_core::id::PageId;
use scroll_server::Settings;
use scroll_server::{AttemptLimits, Limit};
use serde_json::json;

const CLOSE_POLICY: u16 = 1008;

async fn assert_session_ended(client: &mut SyncClient) {
    assert_eq!(
        client.receive().await,
        ServerMessage::Rejected {
            reason: RejectReason::InvalidSession
        }
    );
    let close = client.receive_close().await.expect("a close frame");
    assert_eq!(u16::from(close.code), CLOSE_POLICY);
}

#[tokio::test]
async fn signing_out_closes_that_sessions_connections_only() {
    let server = TestServer::start().await;
    let user = server.new_user().await;
    let other_session = server.sign_in(&user.email).await;
    let workspace_id = server.create_workspace(&user).await;
    let mut first = server.connect(&user).await;
    let mut second = server.connect(&user).await;
    let mut elsewhere = server.connect(&other_session).await;

    let signed_out = server
        .http
        .delete(server.url("/sessions/current"))
        .bearer_auth(&user.token)
        .send()
        .await
        .unwrap();
    assert_eq!(signed_out.status(), StatusCode::NO_CONTENT);

    assert_session_ended(&mut first).await;
    assert_session_ended(&mut second).await;
    let catch_up = elsewhere
        .open_page(workspace_id, PageId::generate(), 0)
        .await;
    assert!(catch_up.updates.is_empty());
}

#[tokio::test]
async fn an_expired_session_closes_its_connections() {
    let server = TestServer::start().await;
    let user = server.new_user().await;
    let mut client = server.connect(&user).await;

    server
        .pool
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE sessions SET expires_at = now() - interval '1 second'
             WHERE token_hash = sha256(convert_to($1, 'UTF8'))",
            &[&user.token],
        )
        .await
        .unwrap();

    assert_session_ended(&mut client).await;
}

#[tokio::test]
async fn losing_membership_closes_the_workspaces_open_pages() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let page_id = PageId::generate();
    let mut owner_client = server.connect(&owner).await;
    let mut editor_client = server.connect(&editor).await;
    owner_client.open_page(workspace_id, page_id, 0).await;
    editor_client.open_page(workspace_id, page_id, 0).await;

    server
        .pool
        .get()
        .await
        .unwrap()
        .execute(
            "DELETE FROM memberships WHERE workspace_id = $1 AND user_id = $2",
            &[&workspace_id.0, &editor.user_id.0],
        )
        .await
        .unwrap();

    assert_eq!(
        editor_client.receive().await,
        ServerMessage::PageRejected {
            page_id,
            reason: RejectReason::NotAMember
        }
    );
    assert_eq!(
        owner_client.receive().await,
        ServerMessage::PresenceLeft {
            page_id,
            user_id: editor.user_id,
            connection_id: editor_client.connection_id.unwrap(),
        }
    );
    owner_client.push(page_id, b"after they left").await;
    editor_client.assert_silent().await;
    editor_client
        .send(&ClientMessage::OpenPage {
            workspace_id,
            page_id,
            last_seen_seq: 0,
        })
        .await;
    assert_eq!(
        editor_client.receive().await,
        ServerMessage::PageRejected {
            page_id,
            reason: RejectReason::NotAMember
        }
    );
}

fn strict_limits(client_ip_header: Option<&str>) -> Settings {
    let window = Duration::from_secs(600);
    Settings {
        client_ip_header: client_ip_header.map(|name| name.parse().unwrap()),
        sign_in_limits: AttemptLimits {
            per_address: Limit {
                attempts: 3,
                window,
            },
            failures_per_address_and_email: Limit {
                attempts: 2,
                window,
            },
        },
        ..TestServer::test_settings()
    }
}

async fn sign_in_from(
    server: &TestServer,
    forwarded_for: &str,
    email: &str,
    password: &str,
) -> reqwest::Response {
    server
        .http
        .post(server.url("/sessions"))
        .header("x-forwarded-for", forwarded_for)
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn failed_guesses_at_an_email_never_lock_out_its_owner_elsewhere() {
    let server = TestServer::start_with(strict_limits(Some("x-forwarded-for"))).await;
    let user = server.new_user().await;

    for _ in 0..2 {
        let wrong = sign_in_from(&server, "10.1.0.1", &user.email, "wrong password").await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }
    let guesser_limited = sign_in_from(&server, "10.1.0.1", &user.email, PASSWORD).await;

    assert_eq!(guesser_limited.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = guesser_limited.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=600).contains(&retry_after));
    let body: serde_json::Value = guesser_limited.json().await.unwrap();
    assert_eq!(body["error"], "rate_limited");
    for _ in 0..3 {
        let owner = sign_in_from(&server, "10.1.0.2", &user.email, PASSWORD).await;
        assert_eq!(owner.status(), StatusCode::CREATED);
    }
}

#[tokio::test]
async fn attempts_from_one_address_are_limited_using_the_last_forwarded_entry() {
    let server = TestServer::start_with(strict_limits(Some("x-forwarded-for"))).await;

    for _ in 0..3 {
        let response = sign_in_from(&server, "10.2.0.1", &unique_email(), PASSWORD).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let limited = sign_in_from(&server, "10.2.0.1", &unique_email(), PASSWORD).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);

    let sign_up = server
        .http
        .post(server.url("/accounts"))
        .header("x-forwarded-for", "10.2.0.1")
        .json(&json!({ "email": unique_email(), "password": PASSWORD, "invite_code": "unused" }))
        .send()
        .await
        .unwrap();
    assert_eq!(sign_up.status(), StatusCode::TOO_MANY_REQUESTS);

    // A client can put anything first; only the entry the proxy appended counts.
    let spoofed_first =
        sign_in_from(&server, "10.2.0.9, 10.2.0.1", &unique_email(), PASSWORD).await;
    assert_eq!(spoofed_first.status(), StatusCode::TOO_MANY_REQUESTS);
    let other_client = sign_in_from(&server, "10.2.0.1, 10.2.0.2", &unique_email(), PASSWORD).await;
    assert_eq!(other_client.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn without_a_proxy_header_the_peer_address_is_limited_and_forwarded_headers_are_ignored() {
    let server = TestServer::start_with(strict_limits(None)).await;

    for address in ["10.3.0.1", "10.3.0.2", "10.3.0.3"] {
        let response = sign_in_from(&server, address, &unique_email(), PASSWORD).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let limited = sign_in_from(&server, "10.3.0.4", &unique_email(), PASSWORD).await;

    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
}
