//! How the workspace's sync state reads in the window: the toolbar's status line, and what an
//! empty sidebar or page area says while pages arrive.

use crate::workspace::sync_state::{Download, SyncState};

/// What the window says while the workspace downloads.
pub fn download_text(download: &Download) -> String {
    match download {
        Download::Starting => "Downloading this workspace…".to_string(),
        Download::Receiving { megabytes } => {
            format!("Downloading… about {megabytes} MB so far")
        }
        Download::Finishing {
            percent: Some(percent),
        } => format!("Finishing download — {percent}%"),
        Download::Finishing { percent: None } => {
            "Finishing download — saving pages on this Mac…".to_string()
        }
    }
}

/// The toolbar's status line; `None` while there is nothing to say. Signed out has its own
/// line with a button, so it is not here.
pub fn status_text(state: &SyncState) -> Option<String> {
    match state {
        SyncState::LocalOnly | SyncState::Settled | SyncState::SignedOut => None,
        SyncState::Downloading(download) => Some(download_text(download)),
        SyncState::Uploading { remaining } => Some(format!("Syncing {remaining} changes…")),
        SyncState::Connecting => Some("Connecting…".to_string()),
        SyncState::Offline => Some("Offline — changes will sync later".to_string()),
        SyncState::AccessLost => Some("You no longer have access to this workspace".to_string()),
        SyncState::ServerNotTrusted => {
            Some("The server's certificate is not trusted; not syncing".to_string())
        }
    }
}

/// What an empty page list or page area says.
pub fn empty_text(state: &SyncState, when_empty: &str) -> String {
    match state {
        SyncState::Downloading(download) => download_text(download),
        _ => when_empty.to_string(),
    }
}
