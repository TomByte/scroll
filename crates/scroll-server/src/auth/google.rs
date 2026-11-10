//! Sign-in with Google (OpenID Connect, authorization-code flow). The server holds the client secret: it sends
//! the person to Google, exchanges the code Google sends back, and verifies the ID token with `jsonwebtoken`
//! against Google's published keys.

use std::time::Duration;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use reqwest::{StatusCode, Url};
use scroll_api::paths;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::auth::browser_sign_in::Departure;
use crate::auth::identities::VerifiedIdentity;
use crate::auth::secrets;

const PROVIDER: &str = "google";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Google rotates its keys over days; an hour keeps a removed key from being trusted for long.
const KEYS_MAX_AGE: Duration = Duration::from_secs(3600);
/// A token naming a key not yet fetched triggers a fetch, but not more often than this.
const KEYS_MIN_REFETCH: Duration = Duration::from_secs(60);

/// The Google OAuth client a host registered: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` and `PUBLIC_URL`.
#[derive(Clone)]
pub struct GoogleSignIn {
    pub client_id: String,
    pub client_secret: String,
    /// The server's external URL; Google sends people back to `paths::GOOGLE_CALLBACK` under it.
    pub public_url: String,
    pub endpoints: GoogleEndpoints,
}

/// Where Google's endpoints are; the tests point them at a fake provider.
#[derive(Clone)]
pub struct GoogleEndpoints {
    pub authorize: String,
    pub token: String,
    pub keys: String,
    /// The `iss` values Google's ID tokens carry.
    pub issuers: Vec<String>,
}

impl Default for GoogleEndpoints {
    fn default() -> Self {
        Self {
            authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            keys: "https://www.googleapis.com/oauth2/v3/certs".into(),
            issuers: vec![
                "https://accounts.google.com".into(),
                "accounts.google.com".into(),
            ],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GoogleError {
    #[error("a request to Google failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Google refused the authorization code with status {0}")]
    CodeRefused(StatusCode),
    #[error("the ID token is not valid: {0}")]
    InvalidToken(#[from] jsonwebtoken::errors::Error),
    #[error("the ID token is signed with a key Google does not publish")]
    UnknownKey,
    #[error("the ID token was issued for another sign-in")]
    NonceMismatch,
    #[error("Google has not verified this account's email")]
    EmailNotVerified,
    #[error("a configured Google URL is not valid: {0}")]
    InvalidUrl(String),
}

impl GoogleSignIn {
    fn redirect_uri(&self) -> String {
        format!(
            "{}{}",
            self.public_url.trim_end_matches('/'),
            paths::GOOGLE_CALLBACK
        )
    }

    /// Where to send the person: Google's consent page for this sign-in.
    fn authorize_url(&self, departure: &Departure) -> Result<Url, GoogleError> {
        let redirect_uri = self.redirect_uri();
        let parameters = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", departure.state.as_str()),
            ("nonce", departure.nonce.as_str()),
            ("prompt", "select_account"),
        ];
        Url::parse_with_params(&self.endpoints.authorize, parameters)
            .map_err(|_| GoogleError::InvalidUrl(self.endpoints.authorize.clone()))
    }
}

/// Exchanges codes and verifies ID tokens, keeping Google's signing keys between sign-ins.
pub struct GoogleVerifier {
    config: GoogleSignIn,
    http: reqwest::Client,
    keys: Mutex<Option<FetchedKeys>>,
}

struct FetchedKeys {
    at: Instant,
    keys: JwkSet,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    /// Shared with the `profile` scope.
    name: Option<String>,
    /// A boolean, though older tokens carried the string `"true"`.
    email_verified: Option<serde_json::Value>,
}

impl IdClaims {
    fn has_verified_email(&self) -> bool {
        matches!(&self.email_verified, Some(serde_json::Value::Bool(true)))
            || matches!(&self.email_verified, Some(serde_json::Value::String(verified)) if verified == "true")
    }
}

impl GoogleVerifier {
    pub fn new(config: GoogleSignIn) -> Result<Self, GoogleError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            config,
            http,
            keys: Mutex::new(None),
        })
    }

    pub fn authorize_url(&self, departure: &Departure) -> Result<Url, GoogleError> {
        self.config.authorize_url(departure)
    }

    /// The person Google vouches for after sending them back with `code`. `nonce_hash` is the hash of the nonce
    /// this sign-in departed with; the ID token must carry that nonce.
    pub async fn identity_for_code(
        &self,
        code: &str,
        nonce_hash: &[u8],
    ) -> Result<VerifiedIdentity, GoogleError> {
        let id_token = self.exchange(code).await?;
        let claims = self.verify(&id_token).await?;
        let nonce_matches = claims
            .nonce
            .as_deref()
            .is_some_and(|nonce| secrets::hash_secret(nonce) == nonce_hash);
        if !nonce_matches {
            return Err(GoogleError::NonceMismatch);
        }
        if !claims.has_verified_email() {
            return Err(GoogleError::EmailNotVerified);
        }
        let email = claims
            .email
            .ok_or(GoogleError::EmailNotVerified)?
            .trim()
            .to_lowercase();
        let name = claims.name.filter(|name| !name.trim().is_empty());
        Ok(VerifiedIdentity {
            provider: PROVIDER,
            subject: claims.sub,
            email,
            name,
        })
    }

    async fn exchange(&self, code: &str) -> Result<String, GoogleError> {
        let redirect_uri = self.config.redirect_uri();
        let form = [
            ("code", code),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ];
        let response = self
            .http
            .post(&self.config.endpoints.token)
            .form(&form)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(GoogleError::CodeRefused(response.status()));
        }
        Ok(response.json::<TokenResponse>().await?.id_token)
    }

    /// Signature by one of Google's keys, RS256 only, the right issuer and audience, and not expired.
    async fn verify(&self, id_token: &str) -> Result<IdClaims, GoogleError> {
        let header = jsonwebtoken::decode_header(id_token)?;
        let kid = header.kid.ok_or(GoogleError::UnknownKey)?;
        let key = self.key(&kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.config.client_id]);
        validation.set_issuer(&self.config.endpoints.issuers);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        Ok(jsonwebtoken::decode::<IdClaims>(id_token, &key, &validation)?.claims)
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, GoogleError> {
        let mut fetched = self.keys.lock().await;
        let is_stale = fetched
            .as_ref()
            .is_none_or(|fetched| fetched.at.elapsed() > KEYS_MAX_AGE);
        let lacks_key = fetched
            .as_ref()
            .is_none_or(|fetched| fetched.keys.find(kid).is_none());
        let may_fetch_again = fetched
            .as_ref()
            .is_none_or(|fetched| fetched.at.elapsed() > KEYS_MIN_REFETCH);
        if is_stale || (lacks_key && may_fetch_again) {
            let keys = self
                .http
                .get(&self.config.endpoints.keys)
                .send()
                .await?
                .error_for_status()?
                .json::<JwkSet>()
                .await?;
            *fetched = Some(FetchedKeys {
                at: Instant::now(),
                keys,
            });
        }
        let jwk = fetched
            .as_ref()
            .and_then(|fetched| fetched.keys.find(kid))
            .ok_or(GoogleError::UnknownKey)?;
        Ok(DecodingKey::from_jwk(jwk)?)
    }
}
