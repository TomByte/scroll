//! Signing in with Google through the browser, end to end: the engine's loopback listener, the server and a
//! fake Google, with an HTTP client following redirects in place of the browser.

mod common;

use std::time::Duration;

use common::TestServer;
use scroll_server::test_support::fake_google::FakeGoogle;
use scroll_sync::{AccountClient, AccountError, SignInMethod, SignUpMode};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_in_through_the_browser_returns_a_session() {
    let google = FakeGoogle::start().await;
    let server = TestServer::start_with_google(google.sign_in_settings()).await;
    google.send_back_to(&server.url());
    let email = format!("browser-{}@example.test", Uuid::now_v7());
    google.sign_in_next_as(&email);
    let accounts = server.accounts();

    let info = accounts.server_info().await.unwrap();
    assert_eq!(info.sign_up, SignUpMode::Open);
    assert_eq!(
        info.sign_in_methods,
        vec![SignInMethod::Password, SignInMethod::Google]
    );

    let sign_in = accounts.start_google_sign_in(None).await.unwrap();
    // The browser's last stop is the loopback listener, which answers only while `finish` runs.
    let browser = tokio::spawn(reqwest::get(sign_in.url().to_owned()));
    let session = sign_in.finish(Duration::from_secs(10)).await.unwrap();
    assert_eq!(session.email, email);
    assert_eq!(session.display_name, email.split('@').next().unwrap());
    let page = browser.await.unwrap().unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains("Return to Scroll"));
    assert!(
        accounts
            .workspaces(&session.session_token)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_browser_that_never_comes_back_times_out_and_cancelling_closes_the_listener() {
    let google = FakeGoogle::start().await;
    let server = TestServer::start_with_google(google.sign_in_settings()).await;
    let accounts: AccountClient = server.accounts();

    let abandoned = accounts.start_google_sign_in(None).await.unwrap();
    let result = abandoned.finish(Duration::from_millis(200)).await;
    assert!(matches!(result, Err(AccountError::BrowserSignInTimedOut)));

    let cancelled = accounts.start_google_sign_in(None).await.unwrap();
    let port = reqwest::Url::parse(cancelled.url())
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == "redirect_port")
        .map(|(_, port)| port.into_owned())
        .unwrap();
    drop(cancelled);
    let refused = reqwest::get(format!("http://127.0.0.1:{port}/callback?code=x")).await;
    assert!(
        refused.is_err(),
        "nothing listens once the sign-in is dropped"
    );
}
