mod common;

use common::{PASSWORD, TestServer, unique_email};
use reqwest::StatusCode;
use scroll_core::id::WorkspaceId;
use serde_json::{Value, json};

async fn error_code(response: reqwest::Response) -> String {
    let body: Value = response.json().await.unwrap();
    body["error"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn an_invite_creates_one_account_that_can_then_sign_in() {
    let server = TestServer::start().await;
    let invite_code = server.signup_invite().await;
    let email = unique_email();

    let created = server
        .post("/accounts", None, json!({ "email": email.to_uppercase(), "password": PASSWORD, "invite_code": invite_code }))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = created.json().await.unwrap();

    let reused = server
        .post(
            "/accounts",
            None,
            json!({ "email": unique_email(), "password": PASSWORD, "invite_code": invite_code }),
        )
        .await;
    assert_eq!(reused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(reused).await, "invalid_invite");

    let user = server.sign_in(&email).await;
    assert_eq!(
        serde_json::to_value(user.user_id).unwrap(),
        created["user_id"]
    );
}

#[tokio::test]
async fn sign_up_rejects_unknown_invites_taken_emails_and_bad_input() {
    let server = TestServer::start().await;
    let existing = server.new_user().await;

    let unknown_invite = server
        .post(
            "/accounts",
            None,
            json!({ "email": unique_email(), "password": PASSWORD, "invite_code": "nope" }),
        )
        .await;
    assert_eq!(error_code(unknown_invite).await, "invalid_invite");

    let invite_code = server.signup_invite().await;
    let taken = server
        .post(
            "/accounts",
            None,
            json!({ "email": existing.email, "password": PASSWORD, "invite_code": invite_code }),
        )
        .await;
    assert_eq!(taken.status(), StatusCode::CONFLICT);
    assert_eq!(error_code(taken).await, "email_taken");

    let short_password = server
        .post(
            "/accounts",
            None,
            json!({ "email": unique_email(), "password": "short", "invite_code": invite_code }),
        )
        .await;
    assert_eq!(short_password.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bad_email = server
        .post(
            "/accounts",
            None,
            json!({ "email": "not an email", "password": PASSWORD, "invite_code": invite_code }),
        )
        .await;
    assert_eq!(bad_email.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // The failed attempts did not use up the invite.
    server.sign_up(&invite_code).await;
}

#[tokio::test]
async fn sign_in_rejects_a_wrong_password_and_an_unknown_email_alike() {
    let server = TestServer::start().await;
    let user = server.new_user().await;

    let wrong_password = server
        .post(
            "/sessions",
            None,
            json!({ "email": user.email, "password": "wrong password" }),
        )
        .await;
    let unknown_email = server
        .post(
            "/sessions",
            None,
            json!({ "email": unique_email(), "password": PASSWORD }),
        )
        .await;

    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(wrong_password).await, "invalid_credentials");
    assert_eq!(unknown_email.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(unknown_email).await, "invalid_credentials");
}

#[tokio::test]
async fn signing_out_ends_the_session() {
    let server = TestServer::start().await;
    let user = server.new_user().await;
    assert_eq!(
        server.get("/workspaces", &user.token).await.status(),
        StatusCode::OK
    );

    let signed_out = server
        .http
        .delete(server.url("/sessions/current"))
        .bearer_auth(&user.token)
        .send()
        .await
        .unwrap();
    assert_eq!(signed_out.status(), StatusCode::NO_CONTENT);

    let after = server.get("/workspaces", &user.token).await;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(after).await, "unauthenticated");
    let no_token = server
        .http
        .get(server.url("/workspaces"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_creator_of_a_workspace_owns_it_and_sees_it_listed() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = WorkspaceId::generate();

    let created = server
        .post(
            "/workspaces",
            Some(&owner.token),
            json!({ "id": workspace_id, "name": "  Design team " }),
        )
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let expected = json!({ "workspace_id": workspace_id, "name": "Design team", "role": "Owner" });
    assert_eq!(created.json::<Value>().await.unwrap(), expected);

    let listed: Value = server
        .get("/workspaces", &owner.token)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(listed, json!([expected]));

    let duplicate = server
        .post(
            "/workspaces",
            Some(&owner.token),
            json!({ "id": workspace_id, "name": "Again" }),
        )
        .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let blank_name = server
        .post(
            "/workspaces",
            Some(&owner.token),
            json!({ "id": WorkspaceId::generate(), "name": "   " }),
        )
        .await;
    assert_eq!(blank_name.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn owners_invite_members_with_a_role_and_nobody_else_can() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;

    let editor = server.new_member(&owner, workspace_id, "Editor").await;
    let listed: Value = server
        .get("/workspaces", &editor.token)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed,
        json!([{ "workspace_id": workspace_id, "name": "Test workspace", "role": "Editor" }])
    );

    let invites_path = format!("/workspaces/{}/invites", workspace_id.0);
    let by_editor = server
        .post(
            &invites_path,
            Some(&editor.token),
            json!({ "role": "Viewer" }),
        )
        .await;
    assert_eq!(by_editor.status(), StatusCode::FORBIDDEN);
    let outsider = server.new_user().await;
    let by_outsider = server
        .post(
            &invites_path,
            Some(&outsider.token),
            json!({ "role": "Viewer" }),
        )
        .await;
    assert_eq!(by_outsider.status(), StatusCode::FORBIDDEN);
    let unknown_role = server
        .post(
            &invites_path,
            Some(&owner.token),
            json!({ "role": "admin" }),
        )
        .await;
    assert!(unknown_role.status().is_client_error());
}

#[tokio::test]
async fn an_existing_user_joins_another_workspace_by_accepting_an_invite() {
    let server = TestServer::start().await;
    let owner = server.new_user().await;
    let workspace_id = server.create_workspace(&owner).await;
    let user = server.new_user().await;
    let invite_code = server.invite(&owner, workspace_id, "Viewer").await;

    let accepted = server
        .post(
            "/invites/accept",
            Some(&user.token),
            json!({ "invite_code": invite_code }),
        )
        .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let membership =
        json!({ "workspace_id": workspace_id, "name": "Test workspace", "role": "Viewer" });
    assert_eq!(accepted.json::<Value>().await.unwrap(), membership);
    let listed: Value = server
        .get("/workspaces", &user.token)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(listed, json!([membership]));

    let again = server
        .post(
            "/invites/accept",
            Some(&user.token),
            json!({ "invite_code": invite_code }),
        )
        .await;
    assert_eq!(error_code(again).await, "invalid_invite");
    let signup_only = server.signup_invite().await;
    let not_for_a_workspace = server
        .post(
            "/invites/accept",
            Some(&user.token),
            json!({ "invite_code": signup_only }),
        )
        .await;
    assert_eq!(error_code(not_for_a_workspace).await, "invalid_invite");
}

#[tokio::test]
async fn the_health_check_reports_a_reachable_database() {
    let server = TestServer::start().await;

    let response = server
        .http
        .get(server.url("/healthz"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn server_info_identifies_a_scroll_server_without_signing_in() {
    let server = TestServer::start().await;

    let info: Value = server
        .http
        .get(server.url("/server-info"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(info["product"], "scroll-server");
    assert_eq!(info["name"], "Scroll");
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        info["protocol_version"],
        scroll_api::protocol::PROTOCOL_VERSION
    );
    assert_eq!(info["sign_up_requires_invite"], true);
}
