//! Keeping the local records in step with the server: applying local and remote transactions,
//! the queue of changes waiting for the server, the confirmed copy of records with pending
//! changes, snapshots, and publishing a local workspace.

pub(in crate::store) mod apply;
pub(in crate::store) mod catch_up;
pub(in crate::store) mod changes;
pub(in crate::store) mod confirmed;
pub(in crate::store) mod link;
pub(in crate::store) mod publish;
pub(in crate::store) mod queue;
pub(in crate::store) mod staged_snapshot;

pub use changes::StoreChanges;
pub use link::WorkspaceLink;
pub use queue::{QueuedChange, QueuedEntry, QueuedPageUpdate};
pub use staged_snapshot::SnapshotFinishProgress;
