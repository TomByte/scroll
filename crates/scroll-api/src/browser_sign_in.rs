//! The rules both ends of a sign-in through the system browser follow (RFC 8252 with PKCE, RFC 7636): the app
//! listens on a loopback port, the server sends the browser back there with a one-time code or a refusal, and
//! the app exchanges the code with the verifier only it knows.

use std::ops::RangeInclusive;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// Where on the app's loopback address the browser comes back to.
pub const LOOPBACK_CALLBACK_PATH: &str = "/callback";
/// Ports below this are not ones a desktop app gets for a loopback listener.
pub const MIN_REDIRECT_PORT: u16 = 1024;
pub const VERIFIER_LENGTHS: RangeInclusive<usize> = 43..=128;
/// A base64url SHA-256.
pub const CHALLENGE_LENGTH: usize = 43;

/// The S256 challenge for a verifier: base64url without padding of its SHA-256.
pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Why a browser sign-in came back without a code; travels as the `error` query parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSignInRefusal {
    /// The person cancelled at the provider, or the provider refused.
    Cancelled,
    /// No account yet, and this server needs an invite code to create one.
    InviteRequired,
    InvalidInvite,
    /// The provider has not verified the account's email.
    EmailNotVerified,
    /// The account with this email is linked to a different account at the provider.
    AccountConflict,
    /// Something went wrong on the server or at the provider; trying again may work.
    Failed,
}

impl BrowserSignInRefusal {
    pub fn code(self) -> &'static str {
        match self {
            BrowserSignInRefusal::Cancelled => "cancelled",
            BrowserSignInRefusal::InviteRequired => "invite_required",
            BrowserSignInRefusal::InvalidInvite => "invalid_invite",
            BrowserSignInRefusal::EmailNotVerified => "email_not_verified",
            BrowserSignInRefusal::AccountConflict => "account_conflict",
            BrowserSignInRefusal::Failed => "sign_in_failed",
        }
    }

    /// An unknown code is a refusal of a kind this version does not know: `Failed`.
    pub fn from_code(code: &str) -> Self {
        [
            BrowserSignInRefusal::Cancelled,
            BrowserSignInRefusal::InviteRequired,
            BrowserSignInRefusal::InvalidInvite,
            BrowserSignInRefusal::EmailNotVerified,
            BrowserSignInRefusal::AccountConflict,
        ]
        .into_iter()
        .find(|refusal| refusal.code() == code)
        .unwrap_or(BrowserSignInRefusal::Failed)
    }
}

/// The query of `GET /auth/google/start`, which the app opens in the browser.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoogleStart {
    /// The app's loopback port, where the browser is sent back to.
    pub redirect_port: u16,
    pub challenge: String,
    /// Used only to create an account on a server that needs an invite.
    #[serde(default)]
    pub invite: Option<String>,
}

impl GoogleStart {
    /// The query parameters, under the names `Deserialize` reads them by.
    pub fn query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = vec![
            ("redirect_port", self.redirect_port.to_string()),
            ("challenge", self.challenge.clone()),
        ];
        if let Some(invite) = &self.invite {
            pairs.push(("invite", invite.clone()));
        }
        pairs
    }
}

/// What the server sends the browser back to the app's loopback address with: one query parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopbackReturn {
    Code(String),
    Refused(BrowserSignInRefusal),
}

impl LoopbackReturn {
    pub fn query_pair(&self) -> (&'static str, &str) {
        match self {
            LoopbackReturn::Code(code) => ("code", code),
            LoopbackReturn::Refused(refusal) => ("error", refusal.code()),
        }
    }

    /// `None` when the query carries neither.
    pub fn from_query_pairs<'a>(pairs: impl Iterator<Item = (&'a str, &'a str)>) -> Option<Self> {
        let mut code = None;
        let mut error = None;
        for (name, value) in pairs {
            match name {
                "code" => code = Some(value.to_owned()),
                "error" => error = Some(value),
                _ => {}
            }
        }
        match (code, error) {
            (_, Some(error)) => Some(LoopbackReturn::Refused(BrowserSignInRefusal::from_code(
                error,
            ))),
            (Some(code), None) => Some(LoopbackReturn::Code(code)),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_codes_round_trip() {
        for refusal in [
            BrowserSignInRefusal::Cancelled,
            BrowserSignInRefusal::InviteRequired,
            BrowserSignInRefusal::InvalidInvite,
            BrowserSignInRefusal::EmailNotVerified,
            BrowserSignInRefusal::AccountConflict,
            BrowserSignInRefusal::Failed,
        ] {
            assert_eq!(BrowserSignInRefusal::from_code(refusal.code()), refusal);
        }
    }

    #[test]
    fn a_loopback_return_reads_back_from_its_query() {
        for returned in [
            LoopbackReturn::Code("abc".into()),
            LoopbackReturn::Refused(BrowserSignInRefusal::InviteRequired),
        ] {
            let pair = returned.query_pair();
            assert_eq!(
                LoopbackReturn::from_query_pairs(std::iter::once(pair)),
                Some(returned)
            );
        }
        assert_eq!(LoopbackReturn::from_query_pairs(std::iter::empty()), None);
    }

    #[test]
    fn the_start_query_reads_back_as_the_server_parses_it() {
        for invite in [None, Some("invite code".to_owned())] {
            let start = GoogleStart {
                redirect_port: 49152,
                challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned(),
                invite,
            };
            let query = serde_urlencoded::to_string(start.query_pairs()).unwrap();
            assert_eq!(
                serde_urlencoded::from_str::<GoogleStart>(&query).unwrap(),
                start
            );
        }
    }

    #[test]
    fn the_challenge_is_the_rfc_7636_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_of(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(challenge_of(verifier).len(), CHALLENGE_LENGTH);
    }
}
