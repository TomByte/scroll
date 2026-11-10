//! A stand-in for Google's OAuth endpoints, in-process. Its authorize page consents at once for the person the
//! test chose and sends the browser back to the server; tests driving the browser leg themselves can mint codes
//! for any claims instead. Tokens are signed with a key made for these tests only, whose public half it
//! publishes like Google does.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Form, Query, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::{GoogleEndpoints, GoogleSignIn};

pub const CLIENT_ID: &str = "scroll-test.apps.example";
pub const CLIENT_SECRET: &str = "test client secret";
pub const PUBLIC_URL: &str = "https://scroll.example.test";
pub const ISSUER: &str = "https://accounts.google.com";
const KEY_ID: &str = "test-key";

/// Made with `openssl genrsa 2048` for these tests; it signs nothing else.
const SIGNING_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEAmbY+1gXMog6Kf5LDdFJnzoDWp8i92o+KfaOSmdIh8yHxdxCq
nki1ARSvQO78Vzg3el3WjALIFKMQhLFs8hGIfRuwOFzsqn+gBMuSsxWpfjVD6De/
VguI2LMIHdmqltoixGQeyP28Rkq/zfIATnVkV8JOax///EIre9knbpK6OZLxFhSA
DtGKF2RIKXfWz8rLlabw8AuyU4F6WlxkKAc2lXCpsKTocxqTL8UWJXGYfCTlM73i
6j9Qd9KYfzWz5UrwblKxnxpX1gB90MSchL9atd9mJ96IwGRjA5hWHDZq+EaIp4G7
7wod3VIqe4Laeuk3wR5mloOpGavyuntZiye9yQIDAQABAoIBADOth2OqMHm9fMfW
Plsxx0+wljpzrYy5HJ0P7N5FPP/Chi17D3nGxP/3hdJFZtYmAtTjjpmk58K0bf3G
rvOVfHdfYzgX2cOaw+BI1unZmxHg85MzhuL1Xs1IJ67tesJb1TuE06rMT8usHt8t
APhf1FwC6h1u9T/IuVjakbzFnNF09arbakNhymIO46y6Lxn1nZp/MYM/wF/G3WJM
+Yjo8insLRdI0nRBzcrFplBu72/r85IsrVVdDjCHavOVGGxYtyGk4Ne0FcbkOLUY
wDiA/hmHlL/O9vEtQCv3tJEAq9eoD7M7Rryw49GBVae574Y8qRsjz6ushOw6wdfC
fcfgy4cCgYEAykbxhQKMbkfmnjL4U3donMj7VfAk9xwU2rSaFBuazRpK1jC/dBy2
roWgzHubAw8UTrdUFlJkTFSTY/POMXlzDbsPrgfMnoAUXRDzDJxraA1Zoh3eVFES
J0Hwtr7MqD/M4krPygGXLqHW0e0hXNsOQAttgIbmQ9DbEAT9kXPTnPMCgYEAwolL
vNCbgf+cvD/TPrlE5DNUV+QN2BmmEIDn//BqufoK/iEDcICcBb92crwGO75AZhTF
l6h1vE9hEW7T4e7n906+IyKAf28rdczSBHOTQb8Phm6uDtLVOV3FPnIBJGCmmS1j
08cHxQLlqSR/skRB5LipGVQcSa/sfCIguilveVMCgYEAieAHejE9hWIPCOAXMdVf
ghbgcjhpCeKjOFwH9hLG1mzIe+zoYv27a4P8EcVi6JfbsLf3YeuxH/I6MNQbRM/S
o5ZQ3r/EMnl8597IvFzWPSt0YDNZu0EzTXz91aKCH2C1h72Ums2wvgFkSt0Gc6OA
uBjR/9xzkeAHoYi1SfKYWA8CgYBh3QHN3b+tWTK2/QC/OCKxDiVrBZ31TYneGYsc
lQyLvH8VMvRw/zH+LkBfHbmhCmeJMv/n+GYqMmCpjXBn43CsV9+M+0lbUkixvTUV
6jJ1S7F6tmk3qdZ8QnD8pgtvCnQk8NGccy60bG2udl8LBg2EC2V+CXMlmqvEEsMW
A8xBYwKBgQCH0QdH1f1Ueb8uGNdr0xqmjl9G2TirSQ2ZALD95jEgcysOcxtjKjS1
/8ugkcXXtRoGaX817z3RNuKOjh1ywmfHBRvWIp4w22UD/6HNE4kq22+W+XxdUWGI
ZgM3AiKIxzAnA3m2eg3CpViowngmMts7Yhlx43SLdBcOmbZRldLsxw==
-----END RSA PRIVATE KEY-----";
const MODULUS: &str = "mbY-1gXMog6Kf5LDdFJnzoDWp8i92o-KfaOSmdIh8yHxdxCqnki1ARSvQO78Vzg3el3WjALIFKMQhLFs8hGIfRuwOFzsqn-gBMuSsxWpfjVD6De_VguI2LMIHdmqltoixGQeyP28Rkq_zfIATnVkV8JOax___EIre9knbpK6OZLxFhSADtGKF2RIKXfWz8rLlabw8AuyU4F6WlxkKAc2lXCpsKTocxqTL8UWJXGYfCTlM73i6j9Qd9KYfzWz5UrwblKxnxpX1gB90MSchL9atd9mJ96IwGRjA5hWHDZq-EaIp4G77wod3VIqe4Laeuk3wR5mloOpGavyuntZiye9yQ";

#[derive(Clone)]
pub struct FakeGoogle {
    address: SocketAddr,
    /// ID tokens by the authorization code that redeems them.
    tokens: Arc<Mutex<HashMap<String, String>>>,
    /// The server's address, where consent sends the browser back to; set once the server listens.
    server_url: Arc<Mutex<String>>,
    /// The email of whoever consents next.
    person: Arc<Mutex<String>>,
}

#[derive(Deserialize)]
struct AuthorizeRequest {
    state: String,
    nonce: String,
}

#[derive(Deserialize)]
struct TokenRequest {
    code: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    grant_type: String,
}

impl FakeGoogle {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = FakeGoogle {
            address: listener.local_addr().unwrap(),
            tokens: Arc::default(),
            server_url: Arc::default(),
            person: Arc::default(),
        };
        let app = Router::new()
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/keys", get(keys))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        fake
    }

    pub fn sign_in_settings(&self) -> GoogleSignIn {
        let base = format!("http://{}", self.address);
        GoogleSignIn {
            client_id: CLIENT_ID.into(),
            client_secret: CLIENT_SECRET.into(),
            public_url: PUBLIC_URL.into(),
            endpoints: GoogleEndpoints {
                authorize: format!("{base}/authorize"),
                token: format!("{base}/token"),
                keys: format!("{base}/keys"),
                issuers: vec![ISSUER.into()],
            },
        }
    }

    /// An authorization code that the token endpoint redeems for an ID token with these claims.
    pub fn issue_code(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(KEY_ID.into());
        let key = EncodingKey::from_rsa_pem(SIGNING_KEY.as_bytes()).unwrap();
        let id_token = jsonwebtoken::encode(&header, claims, &key).unwrap();
        let code = Uuid::now_v7().to_string();
        self.tokens.lock().unwrap().insert(code.clone(), id_token);
        code
    }

    /// For the authorize page: where to send the browser back to.
    pub fn send_back_to(&self, server_url: &str) {
        *self.server_url.lock().unwrap() = server_url.to_owned();
    }

    /// For the authorize page: who consents next.
    pub fn sign_in_next_as(&self, email: &str) {
        *self.person.lock().unwrap() = email.to_owned();
    }
}

async fn authorize(
    State(fake): State<FakeGoogle>,
    Query(request): Query<AuthorizeRequest>,
) -> Redirect {
    let email = fake.person.lock().unwrap().clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = json!({
        "iss": ISSUER, "aud": CLIENT_ID, "sub": format!("google-{email}"), "email": email,
        "email_verified": true, "nonce": request.nonce, "iat": now, "exp": now + 600,
    });
    let code = fake.issue_code(&claims);
    let server_url = fake.server_url.lock().unwrap().clone();
    Redirect::to(&format!(
        "{server_url}/auth/google/callback?state={}&code={code}",
        request.state
    ))
}

/// Like Google: a code works once, and only for the client it was issued to with its registered redirect.
async fn token(
    State(fake): State<FakeGoogle>,
    Form(request): Form<TokenRequest>,
) -> Result<Json<Value>, StatusCode> {
    let is_our_client = request.client_id == CLIENT_ID
        && request.client_secret == CLIENT_SECRET
        && request.redirect_uri == format!("{PUBLIC_URL}/auth/google/callback")
        && request.grant_type == "authorization_code";
    if !is_our_client {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let id_token = fake
        .tokens
        .lock()
        .unwrap()
        .remove(&request.code)
        .ok_or(StatusCode::BAD_REQUEST)?;
    Ok(Json(
        json!({ "id_token": id_token, "token_type": "Bearer" }),
    ))
}

async fn keys() -> Json<Value> {
    Json(json!({ "keys": [{
        "kty": "RSA", "kid": KEY_ID, "use": "sig", "alg": "RS256", "n": MODULUS, "e": "AQAB"
    }]}))
}
