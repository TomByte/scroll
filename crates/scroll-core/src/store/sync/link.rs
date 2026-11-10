//! Whether the workspace lives only in this file or is linked to a server, and linking it.
//! The session token is not stored here: the app keeps it in the Keychain.

use rusqlite::{Connection, params};

use crate::store::sync::publish::{queue_page_contents, queue_publishing};
use crate::store::sync::queue;
use crate::store::{StoreError, WorkspaceStore};

use crate::id::WorkspaceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceLink {
    LocalOnly,
    Linked {
        server_url: String,
        workspace_id: WorkspaceId,
    },
}

impl WorkspaceStore {
    pub fn link(&self) -> Result<WorkspaceLink, StoreError> {
        read_link(&self.conn)
    }

    pub fn workspace_name(&self) -> Result<String, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT name FROM workspace", [], |row| row.get(0))?)
    }

    pub fn set_workspace_name(&mut self, name: &str) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE workspace SET name = ?1", [name])?;
        Ok(())
    }

    /// Links a local-only workspace to a server and queues everything needed to publish it:
    /// transactions creating every record (each at most `PUBLISH_CHUNK_OPERATIONS` operations and
    /// about `PUBLISH_CHUNK_BYTES` encoded, every one valid on top of the ones before), and each
    /// page's content as one update. The normal sync
    /// path then uploads them. If the server rejects the publishing transaction, local records are
    /// left as they are; the caller decides whether to unlink.
    pub fn link_to_server(
        &mut self,
        server_url: &str,
        workspace_id: WorkspaceId,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        if read_link(&tx)? != WorkspaceLink::LocalOnly {
            return Err(StoreError::AlreadyLinked);
        }
        tx.execute(
            "UPDATE workspace SET server_url = ?1, workspace_id = ?2",
            params![server_url, workspace_id],
        )?;
        queue_publishing(&tx)?;
        queue_page_contents(&tx)?;
        // Sequence numbers so far were local; the server's start afresh.
        queue::set_last_seen_workspace_seq(&tx, 0)?;
        tx.commit()?;
        Ok(())
    }
}

pub(in crate::store) fn read_link(conn: &Connection) -> Result<WorkspaceLink, StoreError> {
    let (server_url, workspace_id): (Option<String>, Option<WorkspaceId>) = conn.query_row(
        "SELECT server_url, workspace_id FROM workspace",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(match (server_url, workspace_id) {
        (Some(server_url), Some(workspace_id)) => WorkspaceLink::Linked {
            server_url,
            workspace_id,
        },
        _ => WorkspaceLink::LocalOnly,
    })
}

/// Sync bookkeeping only makes sense for a workspace the server knows about.
pub(in crate::store) fn require_linked(conn: &Connection) -> Result<(), StoreError> {
    if !is_linked(conn)? {
        return Err(StoreError::NotLinked);
    }
    Ok(())
}

pub(in crate::store) fn is_linked(conn: &Connection) -> Result<bool, StoreError> {
    Ok(read_link(conn)? != WorkspaceLink::LocalOnly)
}

#[cfg(test)]
mod tests;
