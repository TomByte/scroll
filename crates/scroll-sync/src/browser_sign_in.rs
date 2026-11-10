//! Signing in with Google through the system browser (RFC 8252). The app opens a URL in the browser; the server
//! sends the person to Google and back to a listener on this device's loopback address with a one-time code,
//! which only this process can exchange: the server bound it to a challenge whose verifier never left here.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use scroll_api::accounts::Session;
use scroll_api::browser_sign_in::{
    GoogleStart, LOOPBACK_CALLBACK_PATH, LoopbackReturn, challenge_of,
};
use scroll_api::paths;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::accounts::{AccountClient, AccountError};
use crate::runtime::run_cancellable;

/// A browser's request line and headers fit easily; anything longer is not the redirect.
const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// For one browser request; the person's own time at Google is bounded by `finish`'s timeout.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A Google sign-in waiting in the system browser. Open `url()` in the browser, then await `finish`. Dropping
/// it, or the `finish` future, cancels: the loopback listener closes and nothing is exchanged.
pub struct BrowserSignIn {
    url: String,
    verifier: String,
    listener: TcpListener,
    client: AccountClient,
}

impl AccountClient {
    /// Call when someone chooses "Sign in with Google" (the server lists `SignInMethod::Google`). Listens on a
    /// loopback port; open the returned sign-in's `url()` in the system browser and await its `finish`. Pass
    /// an invite code to create an account on a server that needs one; an existing account ignores it.
    pub async fn start_google_sign_in(
        &self,
        invite_code: Option<&str>,
    ) -> Result<BrowserSignIn, AccountError> {
        let listener = run_cancellable(TcpListener::bind(("127.0.0.1", 0)))
            .await?
            .map_err(AccountError::Loopback)?;
        let port = listener
            .local_addr()
            .map_err(AccountError::Loopback)?
            .port();
        let verifier = new_verifier();
        let query = GoogleStart {
            redirect_port: port,
            challenge: challenge_of(&verifier),
            invite: invite_code.map(str::to_owned),
        };
        let start = format!("{}{}", self.server_url(), paths::GOOGLE_START);
        let url = Url::parse_with_params(&start, query.query_pairs())
            .map_err(|_| AccountError::InvalidServerUrl)?;
        Ok(BrowserSignIn {
            url: url.into(),
            verifier,
            listener,
            client: self.clone(),
        })
    }
}

impl BrowserSignIn {
    /// Open this in the system browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Waits up to `timeout` for the browser to come back, then exchanges the code for a session: the same
    /// result as `AccountClient::sign_in`. Keep the session token as after a password sign-in.
    pub async fn finish(self, timeout: Duration) -> Result<Session, AccountError> {
        let BrowserSignIn {
            listener,
            verifier,
            client,
            ..
        } = self;
        let returned = run_cancellable(tokio::time::timeout(timeout, await_return(listener)))
            .await?
            .map_err(|_| AccountError::BrowserSignInTimedOut)??;
        match returned {
            LoopbackReturn::Code(code) => client.exchange_sign_in_code(&code, &verifier).await,
            LoopbackReturn::Refused(refusal) => Err(AccountError::BrowserSignInRefused(refusal)),
        }
    }
}

/// 32 random bytes, base64url: an RFC 7636 verifier of 43 characters.
fn new_verifier() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system's random source is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Answers browser requests until one is the sign-in's return. Others (a favicon, a stray probe) get a 404.
async fn await_return(listener: TcpListener) -> Result<LoopbackReturn, AccountError> {
    loop {
        let (stream, _) = listener.accept().await.map_err(AccountError::Loopback)?;
        match tokio::time::timeout(REQUEST_READ_TIMEOUT, answer(stream)).await {
            Ok(Ok(Some(returned))) => return Ok(returned),
            Ok(Ok(None)) => {}
            Ok(Err(error)) => tracing::debug!(%error, "a loopback request failed"),
            Err(_) => tracing::debug!("a loopback request was too slow"),
        }
    }
}

/// Reads one request; if it is `GET /callback`, tells the person to go back to Scroll and returns what it
/// carried.
async fn answer(mut stream: TcpStream) -> Result<Option<LoopbackReturn>, std::io::Error> {
    let request_line = read_request_line(&mut stream).await?;
    let Some(returned) = request_line.as_deref().and_then(parse_return) else {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(None);
    };
    let message = match returned {
        LoopbackReturn::Code(_) => "You are signed in. Return to Scroll.",
        LoopbackReturn::Refused(_) => "Sign-in did not complete. Return to Scroll for details.",
    };
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Scroll</title><p style=\"font:16px system-ui;margin:3em\">{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(Some(returned))
}

/// The first line of the request, once the headers have arrived; `None` for anything that is not one.
async fn read_request_line(stream: &mut TcpStream) -> Result<Option<String>, std::io::Error> {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || request.len() + read > MAX_REQUEST_BYTES {
            return Ok(None);
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&request);
    Ok(text.lines().next().map(str::to_owned))
}

/// `GET /callback?code=...` or `GET /callback?error=...`.
fn parse_return(request_line: &str) -> Option<LoopbackReturn> {
    let mut parts = request_line.split(' ');
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return None;
    };
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != LOOPBACK_CALLBACK_PATH {
        return None;
    }
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    LoopbackReturn::from_query_pairs(
        pairs
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )
}

#[cfg(test)]
mod tests {
    use scroll_api::browser_sign_in::BrowserSignInRefusal;

    use super::*;

    #[test]
    fn the_browser_return_is_read_from_the_request_line() {
        let code = parse_return("GET /callback?code=abc HTTP/1.1");
        assert!(matches!(code, Some(LoopbackReturn::Code(code)) if code == "abc"));
        let refused = parse_return("GET /callback?error=invite_required HTTP/1.1");
        assert!(matches!(
            refused,
            Some(LoopbackReturn::Refused(
                BrowserSignInRefusal::InviteRequired
            ))
        ));
        assert!(parse_return("GET /favicon.ico HTTP/1.1").is_none());
        assert!(parse_return("POST /callback?code=abc HTTP/1.1").is_none());
    }

    #[test]
    fn a_verifier_has_the_length_rfc_7636_asks_for() {
        assert_eq!(new_verifier().len(), 43);
    }
}
