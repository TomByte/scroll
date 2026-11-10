//! How the workspace stands with its server, as one answer for every view to show.

use scroll_sync::ConnectionStatus;

use super::Workspace;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SyncState {
    /// Lives only on this Mac.
    LocalOnly,
    /// In step with its server, or no longer syncing by choice: nothing to say.
    Settled,
    /// The workspace's pages are still arriving on this Mac.
    Downloading(Download),
    Uploading {
        remaining: usize,
    },
    Connecting,
    Offline,
    AccessLost,
    ServerNotTrusted,
    SignedOut,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Download {
    /// Linked to a server but nothing has arrived yet.
    Starting,
    /// A large first download, in parts of about a megabyte; the total is not known.
    Receiving { megabytes: u32 },
    /// Everything has arrived and is being written into the file; a percentage once known.
    Finishing { percent: Option<usize> },
}

impl Workspace {
    /// The one answer to "how is syncing going?": a download first, then the connection.
    pub fn sync_state(&self) -> SyncState {
        let Some(connection) = self.connection else {
            return SyncState::LocalOnly;
        };
        if let Some(megabytes) = self.downloaded_parts {
            return SyncState::Downloading(Download::Receiving { megabytes });
        }
        if self.is_finishing_download {
            let percent = self
                .finish_progress
                .filter(|(_, total)| *total > 0)
                .map(|(applied, total)| applied * 100 / total);
            return SyncState::Downloading(Download::Finishing { percent });
        }
        // An engine that has never connected reports pending until it does, so offline this
        // must not say downloading.
        let is_waiting_for_pages = self.is_first_catch_up_pending
            && matches!(
                connection,
                ConnectionStatus::Connecting | ConnectionStatus::Online
            );
        if is_waiting_for_pages {
            return SyncState::Downloading(Download::Starting);
        }
        match connection {
            ConnectionStatus::Online if self.upload_remaining > 0 => SyncState::Uploading {
                remaining: self.upload_remaining,
            },
            ConnectionStatus::Online | ConnectionStatus::Stopped => SyncState::Settled,
            ConnectionStatus::Connecting => SyncState::Connecting,
            ConnectionStatus::Offline => SyncState::Offline,
            ConnectionStatus::AccessLost => SyncState::AccessLost,
            ConnectionStatus::ServerNotTrusted => SyncState::ServerNotTrusted,
            ConnectionStatus::SignedOut => SyncState::SignedOut,
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, TestAppContext};
    use scroll_core::id::PageId;
    use scroll_sync::{ConnectionStatus, SyncEvent};

    use super::*;
    use crate::workspace::records::WorkspaceRecords;
    use crate::workspace::storage::Storage;

    #[gpui::test]
    fn an_empty_linked_workspace_stops_downloading_once_caught_up(cx: &mut TestAppContext) {
        let path =
            std::env::temp_dir().join(format!("scroll-sync-state-{}.sqlite", PageId::generate().0));
        let storage = Storage::open(&path).unwrap();
        let workspace = cx.new(|_| {
            let mut workspace = Workspace::new(path.clone(), storage, WorkspaceRecords::default());
            workspace.connection = Some(ConnectionStatus::Online);
            workspace.is_first_catch_up_pending = true;
            workspace
        });
        let state = |cx: &mut TestAppContext| workspace.read_with(cx, |w, _| w.sync_state());
        assert_eq!(state(cx), SyncState::Downloading(Download::Starting));

        workspace.update(cx, |workspace, cx| {
            workspace.handle_sync_event(SyncEvent::FirstCatchUpComplete, cx)
        });
        assert_eq!(
            state(cx),
            SyncState::Settled,
            "no pages, but nothing to wait for"
        );

        workspace.update(cx, |workspace, _| workspace.storage().shut_down());
        std::fs::remove_file(path).ok();
    }

    #[gpui::test]
    fn a_pending_catch_up_offline_is_not_downloading(cx: &mut TestAppContext) {
        let path =
            std::env::temp_dir().join(format!("scroll-sync-state-{}.sqlite", PageId::generate().0));
        let storage = Storage::open(&path).unwrap();
        let workspace = cx.new(|_| {
            let mut workspace = Workspace::new(path.clone(), storage, WorkspaceRecords::default());
            workspace.connection = Some(ConnectionStatus::Offline);
            workspace.is_first_catch_up_pending = true;
            workspace
        });
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.sync_state()),
            SyncState::Offline
        );
        workspace.update(cx, |workspace, _| workspace.storage().shut_down());
        std::fs::remove_file(path).ok();
    }
}
