//! Moving a workspace between this device and a server: publishing a local one, and joining a server one.

use std::path::Path;

use scroll_core::id::WorkspaceId;
use scroll_core::store::{StoreError, WorkspaceLink, WorkspaceStore};

use scroll_api::accounts::Membership;

use crate::accounts::{AccountClient, AccountError};

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("the workspace is already linked to a server")]
    AlreadyLinked,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Account(#[from] AccountError),
}

/// Call when the user publishes a local-only workspace. Creates it on the server under the store's workspace
/// name, links the file, and queues all its records and page content; then start a `SyncEngine` to upload them.
pub async fn publish_workspace(
    accounts: &AccountClient,
    session_token: &str,
    store: &mut WorkspaceStore,
) -> Result<Membership, PublishError> {
    if store.link()? != WorkspaceLink::LocalOnly {
        return Err(PublishError::AlreadyLinked);
    }
    let workspace_id = WorkspaceId::generate();
    let name = store.workspace_name()?;
    let membership = accounts
        .create_workspace(session_token, workspace_id, &name)
        .await?;
    store.link_to_server(accounts.server_url(), workspace_id)?;
    Ok(membership)
}

/// Call when the user opens a server workspace on this device for the first time: creates its file, linked
/// and empty; then start a `SyncEngine`, which downloads it.
pub fn create_linked_workspace_file(
    path: &Path,
    server_url: &str,
    membership: &Membership,
) -> Result<(), StoreError> {
    let mut store = WorkspaceStore::open(path)?;
    store.set_workspace_name(&membership.name)?;
    store.link_to_server(server_url, membership.workspace_id)
}
