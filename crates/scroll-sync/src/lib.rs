//! The app's side of sync: account requests over HTTP, and a `SyncEngine` per open linked workspace.
//!
//! The app keeps its own `WorkspaceStore` for the UI and writes local changes to it as usual; the engine opens
//! a second connection to the same file, uploads the queue and writes what the server sends.
//!
//! Live page documents: when the editor opens a page, load its `PageDoc` from the store and call
//! `SyncEngine::open_page`, in the same step on the app's thread. Then import the bytes of every
//! `PageCatchUp` and `RemotePageUpdate` event for the page into that document. Importing something the
//! document already has is harmless. Do not call `WorkspaceStore::save_page_doc` for a linked workspace:
//! the engine compacts the stored log when the page is closed. If the app must save, pass as `included_seq`
//! the `last_seen_page_seq` read *before* loading the document, raised to the `seq` of each `PageCatchUp`,
//! `RemotePageUpdate` and `PageUpdateAcknowledged` event imported since; never a larger number.

mod accounts;
mod browser_sign_in;
mod engine;
mod publish;
mod runtime;
mod server_check;
mod transport;

pub use accounts::{AccountClient, AccountError, NewAccount};
pub use browser_sign_in::BrowserSignIn;
pub use engine::{
    ConnectionStatus, EditRefusal, StartError, SyncEngine, SyncError, SyncEvent, SyncEvents,
};
pub use publish::{PublishError, create_linked_workspace_file, publish_workspace};
pub use scroll_api::accounts::{Membership, ServerInfo, Session, SignInMethod, SignUpMode};
pub use scroll_api::browser_sign_in::BrowserSignInRefusal;
pub use scroll_api::protocol::ConnectionId;
pub use server_check::{ServerCheckError, normalize_server_url};
