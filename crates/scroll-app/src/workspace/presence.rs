//! What this device tells others about where its person is on a page. The server passes the
//! bytes on without reading them, so the format is ours: versioned JSON, so a later app can
//! add fields and an older one can skip messages it does not understand.

use scroll_core::id::UserId;
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceState {
    pub version: u32,
    /// The name shown on this person's caret and avatar.
    pub name: String,
    /// Encoded `StableCursor`s for the selection's head and, if text is selected, its anchor.
    pub head: Vec<u8>,
    pub anchor: Option<Vec<u8>>,
}

impl PresenceState {
    pub fn new(name: String, head: Vec<u8>, anchor: Option<Vec<u8>>) -> Self {
        PresenceState {
            version: VERSION,
            name,
            head,
            anchor,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        // Serialising plain strings and byte lists cannot fail.
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// `None` for anything this version cannot read.
    pub fn decode(bytes: &[u8]) -> Option<PresenceState> {
        let state: PresenceState = serde_json::from_slice(bytes).ok()?;
        (state.version == VERSION).then_some(state)
    }
}

/// Which collaborator colour marks `user`: the same on every device and in every session.
pub fn color_index(user: UserId) -> usize {
    let bytes = user.0.as_bytes();
    // The low bytes of a UUIDv7 are random, unlike the leading timestamp.
    usize::from(bytes[15]) ^ usize::from(bytes[14])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_other_versions() {
        let state = PresenceState::new("Ada".into(), vec![1, 2, 3], None);
        assert_eq!(PresenceState::decode(&state.encode()), Some(state.clone()));
        let future = PresenceState {
            version: 2,
            ..state
        };
        assert_eq!(PresenceState::decode(&future.encode()), None);
        assert_eq!(PresenceState::decode(b"not json"), None);
    }

    #[test]
    fn colours_are_stable_per_person() {
        let user = UserId::generate();
        assert_eq!(color_index(user), color_index(user));
    }
}
