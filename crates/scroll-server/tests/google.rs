//! Signing in with Google through the browser, against a fake Google, and the sign-up modes.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{PASSWORD, TestServer, unique_email};
use reqwest::{StatusCode, Url, redirect};
use scroll_server::Settings;
use scroll_server::SignUpMode;
use scroll_server::test_support::fake_google::{CLIENT_ID, FakeGoogle, ISSUER, PUBLIC_URL};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const VERIFIER: &str = "a-verifier-the-app-keeps-to-itself-0123456789";
const APP_PORT: u16 = 51_234;

struct Google {
    server: TestServer,
    fake: FakeGoogle,
    browser: reqwest::Client,
}

/// Where Google would send the person, with the secrets of this sign-in.
struct AtGoogle {
    state: String,
    nonce: String,
}

impl Google {
    async fn start(sign_up: SignUpMode) -> Self {
        let fake = FakeGoogle::start().await;
        let server = TestServer::start_with(Settings {
            sign_up,
            google: Some(fake.sign_in_settings()),
            ..TestServer::test_settings()
        })
        .await;
        let browser = reqwest::Client::builder()
            .redirect(redirect::Policy::none())
            .build()
            .unwrap();
        Google {
            server,
            fake,
            browser,
        }
    }

    /// What the app opens in the browser; returns what the server sent the browser on to Google with.
    async fn leave_for_google(&self, invite: Option<&str>) -> AtGoogle {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
        let port = APP_PORT.to_string();
        let mut query = vec![("redirect_port", port.as_str()), ("challenge", &challenge)];
        if let Some(invite) = invite {
            query.push(("invite", invite));
        }
        let response = self
            .browser
            .get(self.server.url("/auth/google/start"))
            .query(&query)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_redirection());
        let location = Url::parse(location(&response)).unwrap();
        let parameter = |name: &str| {
            location
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
                .unwrap()
        };
        assert_eq!(parameter("client_id"), CLIENT_ID);
        assert_eq!(
            parameter("redirect_uri"),
            format!("{PUBLIC_URL}/auth/google/callback")
        );
        assert_eq!(parameter("scope"), "openid email profile");
        AtGoogle {
            state: parameter("state"),
            nonce: parameter("nonce"),
        }
    }

    /// Google sends the browser back to the server; returns where the server sends it next.
    async fn come_back(&self, state: &str, code: &str) -> reqwest::Response {
        self.browser
            .get(self.server.url("/auth/google/callback"))
            .query(&[("state", state), ("code", code)])
            .send()
            .await
            .unwrap()
    }

    /// The whole browser leg: the outcome the app's loopback listener receives.
    async fn sign_in_as(&self, claims: impl FnOnce(&AtGoogle) -> Value) -> Outcome {
        self.sign_in_with_invite(None, claims).await
    }

    async fn sign_in_with_invite(
        &self,
        invite: Option<&str>,
        claims: impl FnOnce(&AtGoogle) -> Value,
    ) -> Outcome {
        let at_google = self.leave_for_google(invite).await;
        let code = self.fake.issue_code(&claims(&at_google));
        let response = self.come_back(&at_google.state, &code).await;
        loopback_outcome(&response)
    }

    async fn exchange(&self, code: &str, verifier: &str) -> reqwest::Response {
        self.server
            .post(
                "/sessions/exchange",
                None,
                json!({ "code": code, "verifier": verifier }),
            )
            .await
    }

    /// Exchanges the code and returns the session's user and token.
    async fn session(&self, code: &str) -> (String, String) {
        let response = self.exchange(code, VERIFIER).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = response.json().await.unwrap();
        (
            body["user_id"].as_str().unwrap().to_owned(),
            body["session_token"].as_str().unwrap().to_owned(),
        )
    }

    async fn is_signed_in(&self, token: &str) -> bool {
        self.server.get("/workspaces", token).await.status() == StatusCode::OK
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Code(String),
    Error(String),
}

fn location(response: &reqwest::Response) -> &str {
    response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
}

/// Only ever the app's loopback address.
fn loopback_outcome(response: &reqwest::Response) -> Outcome {
    assert!(response.status().is_redirection(), "{}", response.status());
    let url = Url::parse(location(response)).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.port(), Some(APP_PORT));
    assert_eq!(url.path(), "/callback");
    let (name, value) = url.query_pairs().next().unwrap();
    match name.as_ref() {
        "code" => Outcome::Code(value.into_owned()),
        "error" => Outcome::Error(value.into_owned()),
        other => panic!("unexpected parameter {other}"),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn claims(at_google: &AtGoogle, subject: &str, email: &str) -> Value {
    json!({
        "iss": ISSUER,
        "aud": CLIENT_ID,
        "sub": subject,
        "email": email,
        "email_verified": true,
        "nonce": at_google.nonce,
        "iat": now(),
        "exp": now() + 600,
    })
}

fn code_of(outcome: Outcome) -> String {
    match outcome {
        Outcome::Code(code) => code,
        Outcome::Error(error) => panic!("sign-in refused: {error}"),
    }
}

#[tokio::test]
async fn signing_in_with_google_makes_an_account_once_and_signs_in_to_it_after() {
    let google = Google::start(SignUpMode::Open).await;
    let email = unique_email();
    let subject = format!("google-{email}");

    let first = code_of(
        google
            .sign_in_as(|at| claims(at, &subject, &email.to_uppercase()))
            .await,
    );
    let (user_id, token) = google.session(&first).await;
    assert!(google.is_signed_in(&token).await);

    let again = code_of(google.sign_in_as(|at| claims(at, &subject, &email)).await);
    let (same_user, _) = google.session(&again).await;
    assert_eq!(same_user, user_id);

    // The account has no password to sign in with.
    let by_password = google
        .server
        .post(
            "/sessions",
            None,
            json!({ "email": email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(by_password.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_code_works_once_only_with_its_verifier_and_only_while_fresh() {
    let google = Google::start(SignUpMode::Open).await;
    let email = unique_email();
    let subject = format!("google-{email}");
    let person = |at: &AtGoogle| claims(at, &subject, &email);
    let replayed = code_of(google.sign_in_as(person).await);
    google.session(&replayed).await;
    let again = google.exchange(&replayed, VERIFIER).await;
    assert_eq!(again.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        again.json::<Value>().await.unwrap()["error"],
        "invalid_code"
    );

    let guessed = code_of(google.sign_in_as(person).await);
    let wrong = google
        .exchange(&guessed, "another-verifier-that-is-long-enough-0123456789")
        .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let right_after_wrong = google.exchange(&guessed, VERIFIER).await;
    assert_eq!(
        right_after_wrong.status(),
        StatusCode::UNAUTHORIZED,
        "a wrong guess uses the code up"
    );

    let stale = code_of(google.sign_in_as(person).await);
    google
        .server
        .pool
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE sign_in_codes SET expires_at = now() - interval '1 second' WHERE code_hash = $1",
            &[&Sha256::digest(stale.as_bytes()).to_vec()],
        )
        .await
        .unwrap();
    let expired = google.exchange(&stale, VERIFIER).await;
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unknown_or_used_state_goes_nowhere() {
    let google = Google::start(SignUpMode::Open).await;
    let made_up = google.come_back("not-a-state", "some-code").await;
    assert_eq!(made_up.status(), StatusCode::BAD_REQUEST);
    assert!(made_up.headers().get("location").is_none());

    let email = unique_email();
    let at_google = google.leave_for_google(None).await;
    let code = google
        .fake
        .issue_code(&claims(&at_google, &format!("google-{email}"), &email));
    let first = google.come_back(&at_google.state, &code).await;
    assert!(matches!(loopback_outcome(&first), Outcome::Code(_)));
    let replayed = google.come_back(&at_google.state, &code).await;
    assert_eq!(replayed.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tokens_google_would_not_vouch_for_are_refused() {
    let google = Google::start(SignUpMode::Open).await;
    let email = unique_email();
    let subject = format!("google-{email}");
    let refused = |error: &str| Outcome::Error(error.to_owned());

    let unverified = google
        .sign_in_as(|at| {
            let mut claims = claims(at, &subject, &email);
            claims["email_verified"] = json!(false);
            claims
        })
        .await;
    assert_eq!(unverified, refused("email_not_verified"));

    let other_audience = google
        .sign_in_as(|at| {
            let mut claims = claims(at, &subject, &email);
            claims["aud"] = json!("someone-elses-client");
            claims
        })
        .await;
    assert_eq!(other_audience, refused("sign_in_failed"));

    let other_sign_in = google
        .sign_in_as(|at| {
            let mut claims = claims(at, &subject, &email);
            claims["nonce"] = json!("a-nonce-from-another-sign-in");
            claims
        })
        .await;
    assert_eq!(other_sign_in, refused("sign_in_failed"));

    let expired = google
        .sign_in_as(|at| {
            let mut claims = claims(at, &subject, &email);
            claims["exp"] = json!(now() - 3600);
            claims
        })
        .await;
    assert_eq!(expired, refused("sign_in_failed"));
}

#[tokio::test]
async fn an_invite_only_server_needs_an_invite_for_a_new_google_account() {
    let google = Google::start(SignUpMode::Invite).await;
    let email = unique_email();
    let subject = format!("google-{email}");

    let without = google.sign_in_as(|at| claims(at, &subject, &email)).await;
    assert_eq!(without, Outcome::Error("invite_required".into()));

    let invite = google.server.signup_invite().await;
    let with = google
        .sign_in_with_invite(Some(&invite), |at| claims(at, &subject, &email))
        .await;
    google.session(&code_of(with)).await;

    // The account exists now; signing in needs no invite.
    let again = google.sign_in_as(|at| claims(at, &subject, &email)).await;
    google.session(&code_of(again)).await;
}

#[tokio::test]
async fn google_proving_an_email_takes_its_account_back_from_whoever_registered_it_first() {
    let google = Google::start(SignUpMode::Open).await;
    let invite = google.server.signup_invite().await;
    let squatter = google.server.sign_up(&invite).await;
    assert!(google.is_signed_in(&squatter.token).await);
    let mut squatter_connection = google.server.connect(&squatter).await;

    let owner = google
        .sign_in_as(|at| claims(at, &format!("owner-{}", squatter.email), &squatter.email))
        .await;
    let (user_id, token) = google.session(&code_of(owner)).await;

    assert_eq!(user_id, squatter.user_id.0.to_string(), "the same account");
    assert!(google.is_signed_in(&token).await);
    assert!(
        !google.is_signed_in(&squatter.token).await,
        "the squatter's session ended"
    );
    assert!(
        squatter_connection.skip_to_close().await.is_some(),
        "and the squatter's open sync connection closed at once"
    );
    let squatter_password = google
        .server
        .post(
            "/sessions",
            None,
            json!({ "email": squatter.email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(
        squatter_password.status(),
        StatusCode::UNAUTHORIZED,
        "the squatter's password no longer works"
    );

    // Another Google account with the same address cannot take it again.
    let impostor = google
        .sign_in_as(|at| {
            claims(
                at,
                &format!("someone-else-{}", squatter.email),
                &squatter.email,
            )
        })
        .await;
    assert_eq!(impostor, Outcome::Error("account_conflict".into()));
}

#[tokio::test]
async fn the_server_says_how_to_sign_in_and_up_and_open_sign_up_needs_no_invite() {
    let google = Google::start(SignUpMode::Open).await;
    let info: Value = google
        .browser
        .get(google.server.url("/server-info"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["sign_up"], "open");
    assert_eq!(info["sign_up_requires_invite"], false);
    assert_eq!(info["sign_in_methods"], json!(["password", "google"]));
    let created = google
        .server
        .post(
            "/accounts",
            None,
            json!({ "email": unique_email(), "password": PASSWORD }),
        )
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);

    let invite_only = TestServer::start().await;
    let info: Value = reqwest::get(invite_only.url("/server-info"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["sign_up"], "invite");
    assert_eq!(info["sign_in_methods"], json!(["password"]));
    let refused = invite_only
        .post(
            "/accounts",
            None,
            json!({ "email": unique_email(), "password": PASSWORD }),
        )
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        refused.json::<Value>().await.unwrap()["error"],
        "invite_required"
    );
    let google_off = invite_only
        .http
        .get(invite_only.url("/auth/google/start?redirect_port=50000&challenge=x"))
        .send()
        .await
        .unwrap();
    assert_eq!(google_off.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_an_unprivileged_loopback_port_and_a_well_formed_challenge_are_accepted() {
    let google = Google::start(SignUpMode::Open).await;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    for (port, challenge) in [("80", challenge.as_str()), ("50000", "too-short")] {
        let response = google
            .browser
            .get(google.server.url("/auth/google/start"))
            .query(&[("redirect_port", port), ("challenge", challenge)])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn browser_sign_in_endpoints_count_against_the_address_limit() {
    let fake = FakeGoogle::start().await;
    let limit = scroll_server::Limit {
        attempts: 2,
        window: std::time::Duration::from_secs(60),
    };
    let server = TestServer::start_with(Settings {
        sign_up: SignUpMode::Open,
        google: Some(fake.sign_in_settings()),
        sign_in_limits: scroll_server::AttemptLimits {
            per_address: limit,
            failures_per_address_and_email: limit,
        },
        ..TestServer::test_settings()
    })
    .await;
    let exchange = || {
        server.post(
            "/sessions/exchange",
            None,
            json!({ "code": "made-up", "verifier": VERIFIER }),
        )
    };
    assert_eq!(exchange().await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(exchange().await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(exchange().await.status(), StatusCode::TOO_MANY_REQUESTS);
    let start = server
        .http
        .get(server.url("/auth/google/start?redirect_port=50000&challenge=x"))
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn every_session_says_whose_it_is_by_email_and_name() {
    let google = Google::start(SignUpMode::Open).await;
    let email = unique_email();
    let subject = format!("google-{email}");
    let outcome = google
        .sign_in_as(|at| {
            let mut claims = claims(at, &subject, &email);
            claims["name"] = json!("Ada Lovelace");
            claims
        })
        .await;
    let session: Value = google
        .exchange(&code_of(outcome), VERIFIER)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(session["email"], email);
    assert_eq!(session["display_name"], "Ada Lovelace");

    let password_email = unique_email();
    google
        .server
        .post(
            "/accounts",
            None,
            json!({ "email": password_email, "password": PASSWORD }),
        )
        .await;
    let session: Value = google
        .server
        .post(
            "/sessions",
            None,
            json!({ "email": password_email, "password": PASSWORD }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(session["email"], password_email);
    let local_part = password_email.split('@').next().unwrap();
    assert_eq!(session["display_name"], local_part);
}
