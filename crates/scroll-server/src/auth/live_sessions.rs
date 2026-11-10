//! Which sync connections each session has open, so signing out closes them at once instead of at the next
//! periodic check.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use scroll_api::protocol::ConnectionId;
use tokio::sync::Notify;

use super::secrets;
use crate::lock;

/// Each connection's signal to close, by connection.
type SessionConnections = HashMap<ConnectionId, Arc<Notify>>;

/// Keyed by the session token's hash, like the `sessions` table.
#[derive(Default)]
pub struct LiveSessions {
    connections: Mutex<HashMap<Vec<u8>, SessionConnections>>,
}

impl LiveSessions {
    /// `ended` is signalled when the session is signed out.
    pub fn register(&self, session_token: &str, connection_id: ConnectionId, ended: Arc<Notify>) {
        lock(&self.connections)
            .entry(secrets::hash_secret(session_token))
            .or_default()
            .insert(connection_id, ended);
    }

    pub fn unregister(&self, session_token: &str, connection_id: ConnectionId) {
        let key = secrets::hash_secret(session_token);
        let mut connections = lock(&self.connections);
        let Some(session_connections) = connections.get_mut(&key) else {
            return;
        };
        session_connections.remove(&connection_id);
        if session_connections.is_empty() {
            connections.remove(&key);
        }
    }

    pub fn end(&self, session_token: &str) {
        self.end_by_hash(&secrets::hash_secret(session_token));
    }

    /// For sessions ended without their token at hand, by the hash the `sessions` table keeps.
    pub fn end_by_hash(&self, token_hash: &[u8]) {
        let ended = lock(&self.connections).remove(token_hash);
        for notify in ended.into_iter().flat_map(SessionConnections::into_values) {
            notify.notify_one();
        }
    }
}
