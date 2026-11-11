//! The workspace file: one SQLite database holding records, page content, the sync queue and
//! the search index.

pub(in crate::store) mod connection;
pub(in crate::store) mod content;
#[cfg(feature = "fixture")]
pub mod fixture;
pub(in crate::store) mod kinds;
pub(in crate::store) mod query;
#[cfg(any(test, feature = "fixture"))]
pub(in crate::store) mod random;
pub(in crate::store) mod schema;
pub(in crate::store) mod search;
pub(in crate::store) mod sync;
#[cfg(test)]
pub(in crate::store) mod test_support;
pub(in crate::store) mod validate;

use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::page_doc::PageDocError;
use crate::validation::ValidationError;

pub use content::{PageCatchUp, SequencedBytes};
pub use query::{ChildPage, SearchHit, TagUsage, ViewRowIds};
pub use sync::{
    QueuedChange, QueuedEntry, QueuedPageUpdate, SnapshotFinishProgress, StoreChanges,
    WorkspaceLink,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored or queued data could not be encoded or decoded: {0}")]
    Encoding(#[from] postcard::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    PageDoc(#[from] PageDocError),
    #[error("queue entry {0} is not the kind of change it was acknowledged as")]
    QueueEntryKindMismatch(Uuid),
    #[error(
        "cannot save a page snapshot at sequence {offered}: the stored snapshot is already at {stored}"
    )]
    SnapshotOlderThanStored { offered: u64, stored: u64 },
    #[error("the workspace is already linked to a server")]
    AlreadyLinked,
    #[error("the workspace is local-only, so it has no server changes to apply")]
    NotLinked,
    #[error("the file was created before space could be reclaimed in steps")]
    CannotReclaimSpace,
    #[error("no snapshot at sequence {seq} is being staged (staged: {staged:?})")]
    NoStagedSnapshot { seq: u64, staged: Option<u64> },
}

/// One workspace file. The same format is used by the app and the server.
pub struct WorkspaceStore {
    conn: Connection,
}

/// Records nested inside SQLite columns use the same encoding as the wire.
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    Ok(postcard::to_stdvec(value)?)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// A caller's row limit as SQLite can bind it. SQLite integers stop at `i64::MAX`, and no table
/// holds more rows than that, so a larger limit means the same as no limit.
fn sql_limit(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

#[cfg(test)]
fn open_in_memory() -> WorkspaceStore {
    WorkspaceStore::open(std::path::Path::new(":memory:")).expect("an in-memory store always opens")
}

#[cfg(test)]
fn open_linked_in_memory() -> WorkspaceStore {
    let mut store = open_in_memory();
    store
        .link_to_server("https://scroll.test", crate::id::WorkspaceId::generate())
        .expect("an empty local store can always be linked");
    store
}
