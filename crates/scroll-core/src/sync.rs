//! Bringing a client's records up to date, the record counterpart of `store::PageCatchUp`.

use serde::{Deserialize, Serialize};

use crate::id::{PageId, PropertyId};
use crate::operation::{Record, Transaction};
use crate::record::PropertyValue;

/// A whole catch-up assembled from the parts the server sends it in: the snapshot (if any) from
/// its snapshot parts, then the transactions in order. The server sends a snapshot when the
/// client has seen nothing (`last_seen_seq` 0) and only transactions otherwise. The client
/// applies the parts as they arrive and never holds this; tests assemble it to compare.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCatchUp {
    pub snapshot: Option<WorkspaceSnapshot>,
    /// In increasing `seq` order, all after the snapshot's `seq` (or after the client's last seen).
    pub transactions: Vec<SequencedTransaction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub seq: u64,
    pub records: Vec<Record>,
    pub property_values: Vec<PropertyValueEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyValueEntry {
    pub page: PageId,
    pub property: PropertyId,
    pub value: PropertyValue,
}

/// A committed transaction with the workspace sequence number the server gave it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencedTransaction {
    pub seq: u64,
    pub transaction: Transaction,
}
