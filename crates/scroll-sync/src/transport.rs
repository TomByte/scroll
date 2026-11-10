//! Talking to a server: JSON requests and their errors, trusting an extra certificate for both HTTP and the sync
//! socket, and telling a failed secure connection apart from a lost one.

use std::error::Error;

use reqwest::{Method, StatusCode};
use scroll_api::errors::{ErrorBody, ErrorCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::task::JoinError;
use tokio_tungstenite::tungstenite;

use crate::accounts::AccountError;
use crate::runtime::{self, Response};

#[derive(Clone)]
pub(crate) struct Transport {
    server_url: String,
    http: reqwest::Client,
}

/// One request: what, where, as whom, and with what body.
pub(crate) struct Call<'a, B> {
    pub method: Method,
    pub path: &'a str,
    pub session_token: Option<&'a str>,
    pub body: Option<&'a B>,
}

impl Transport {
    /// `server_url` normalized. The system's trusted certificates apply, plus `root_certificate` (PEM) if given.
    pub fn new(server_url: String, root_certificate: Option<&[u8]>) -> Result<Self, AccountError> {
        let mut http = reqwest::Client::builder();
        if let Some(pem) = root_certificate {
            let certificate =
                reqwest::Certificate::from_pem(pem).map_err(AccountError::InvalidCertificate)?;
            http = http.add_root_certificate(certificate);
        }
        Ok(Self {
            server_url,
            http: http.build().map_err(AccountError::Network)?,
        })
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// The whole response to a `GET` needing no session, whatever its status.
    pub async fn fetch(&self, path: &str) -> Result<Result<Response, reqwest::Error>, JoinError> {
        let request = self.http.get(format!("{}{path}", self.server_url));
        runtime::send(request).await
    }

    pub async fn send_json<B: Serialize, T: DeserializeOwned>(
        &self,
        call: Call<'_, B>,
    ) -> Result<T, AccountError> {
        let body = self.send(call).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// The body of a successful response; any other status becomes the matching error.
    pub async fn send<B: Serialize>(&self, call: Call<'_, B>) -> Result<Vec<u8>, AccountError> {
        let mut request = self
            .http
            .request(call.method, format!("{}{}", self.server_url, call.path));
        if let Some(token) = call.session_token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = call.body {
            request = request.json(body);
        }
        let response = runtime::send(request).await?.map_err(network_error)?;
        if response.status.is_success() {
            return Ok(response.body);
        }
        Err(error_from(&response))
    }
}

pub(crate) fn post<'a, B>(
    path: &'a str,
    session_token: Option<&'a str>,
    body: &'a B,
) -> Call<'a, B> {
    Call {
        method: Method::POST,
        path,
        session_token,
        body: Some(body),
    }
}

pub(crate) fn bare<'a>(method: Method, path: &'a str, session_token: &'a str) -> Call<'a, ()> {
    Call {
        method,
        path,
        session_token: Some(session_token),
        body: None,
    }
}

fn network_error(error: reqwest::Error) -> AccountError {
    if is_tls_failure(&error) {
        AccountError::ServerNotTrusted(error)
    } else {
        AccountError::Network(error)
    }
}

/// Maps the server's error bodies to errors; one this version cannot read becomes `Server`.
fn error_from(response: &Response) -> AccountError {
    let server = AccountError::Server {
        status: response.status.as_u16(),
    };
    let Ok(body) = serde_json::from_slice::<ErrorBody>(&response.body) else {
        return server;
    };
    match body.error {
        ErrorCode::InvalidInvite => AccountError::InvalidInvite,
        ErrorCode::InviteRequired => AccountError::InviteRequired,
        ErrorCode::AccountConflict => AccountError::AccountConflict,
        ErrorCode::InvalidCode => AccountError::InvalidSignInCode,
        ErrorCode::NotFound => AccountError::NotOffered,
        ErrorCode::EmailTaken => AccountError::EmailTaken,
        ErrorCode::InvalidCredentials => AccountError::InvalidCredentials,
        ErrorCode::Unauthenticated => AccountError::SignedOut,
        ErrorCode::Forbidden => AccountError::Forbidden,
        ErrorCode::WorkspaceExists => AccountError::WorkspaceExists,
        ErrorCode::RateLimited => AccountError::RateLimited {
            retry_after: response.retry_after,
        },
        ErrorCode::InvalidInput => AccountError::InvalidInput {
            field: body.field.unwrap_or_default(),
        },
        ErrorCode::Unavailable | ErrorCode::Internal => server,
    }
}

pub(crate) fn is_rate_limited(response: &Response) -> bool {
    response.status == StatusCode::TOO_MANY_REQUESTS
}

/// For the sync socket: the system's trusted certificates plus `pem`.
pub(crate) fn tls_connector(pem: &[u8]) -> Result<native_tls::TlsConnector, native_tls::Error> {
    let certificate = native_tls::Certificate::from_pem(pem)?;
    native_tls::TlsConnector::builder()
        .add_root_certificate(certificate)
        .build()
}

/// Whether a secure-connection failure is anywhere in the chain of causes, from HTTP requests and the sync
/// socket alike: the handshake failed, most often because the server's certificate is not trusted. The socket's
/// TLS error does not expose the `native_tls` error as its source, so it is recognised by its variant.
pub(crate) fn is_tls_failure(error: &(dyn Error + 'static)) -> bool {
    let mut cause = Some(error);
    while let Some(error) = cause {
        let is_socket_tls = matches!(
            error.downcast_ref::<tungstenite::Error>(),
            Some(tungstenite::Error::Tls(_))
        );
        if is_socket_tls || error.is::<native_tls::Error>() {
            return true;
        }
        cause = error.source();
    }
    false
}
