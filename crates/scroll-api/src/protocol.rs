//! Messages exchanged between the app and the server over the WebSocket, one postcard-encoded message per binary frame.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use scroll_core::id::{DatabaseId, PageId, TransactionId, UserId, WorkspaceId};
use scroll_core::operation::Transaction;
use scroll_core::store::PageCatchUp;
use scroll_core::sync::{SequencedTransaction, WorkspaceSnapshot};
use scroll_core::validation::ValidationError;

pub const PROTOCOL_VERSION: u16 = 2;

/// Assigned by the server to each connection, so two devices of one user on the same page have separate presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub Uuid);

// Not `Eq`: a transaction can carry an `f64` property value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Must be the first message on a connection.
    /// Keep it the first variant with these fields in every protocol version, so an old server can still
    /// decode a new client's `Hello` and answer `UnsupportedProtocolVersion`.
    Hello {
        protocol_version: u16,
        session_token: String,
    },
    /// Subscribes to a page. The server replies with `PageCatchUp`, then streams `RemotePageUpdate`.
    OpenPage {
        workspace_id: WorkspaceId,
        page_id: PageId,
        last_seen_seq: u64,
    },
    ClosePage {
        page_id: PageId,
    },
    /// A Loro update. `update_id` lets the client match the `PageUpdateAck` and makes resends idempotent.
    PushPageUpdate {
        update_id: Uuid,
        page_id: PageId,
        update: Vec<u8>,
    },
    /// Cursor and selection state, relayed to other subscribers and never stored.
    Presence {
        page_id: PageId,
        state: Vec<u8>,
    },
    /// Subscribes to the workspace's records. The server replies with every `WorkspaceCatchUpPart`, then
    /// streams `RemoteTransaction`.
    OpenWorkspace {
        workspace_id: WorkspaceId,
        last_seen_seq: u64,
    },
    CloseWorkspace {
        workspace_id: WorkspaceId,
    },
    /// The workspace must be open. Resending a transaction with the same id returns the original ack.
    PushTransaction {
        workspace_id: WorkspaceId,
        transaction: Transaction,
    },
    /// The app is showing this database's rows. Transactions that only change rows of a database are
    /// relayed at once to connections that have it open; other connections receive them a little later,
    /// gathered into `Batch`es, still in sequence order with everything else. Opening one also delivers at
    /// once whatever was held back. The workspace must be open; there is no reply.
    OpenDatabase {
        workspace_id: WorkspaceId,
        database_id: DatabaseId,
    },
    CloseDatabase {
        workspace_id: WorkspaceId,
        database_id: DatabaseId,
    },
}

/// On one connection, a page's `PageCatchUp`, `RemotePageUpdate` and `PageUpdateAck` messages arrive in
/// increasing `seq` order (an ack for a resend repeats its original `seq`), so the highest `seq` seen is a safe
/// `last_seen_seq` for the next `OpenPage`. The same holds for a workspace's `WorkspaceCatchUpPart`s,
/// `RemoteTransaction` and `TransactionAck`, which use the workspace's own sequence: the catch-up parts
/// bring the client to one sequence number, and every remote transaction after them follows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome {
        user_id: UserId,
        connection_id: ConnectionId,
    },
    /// The connection is refused or broken; the server closes it after this message.
    /// Also sent unprompted with `InvalidSession` when the session is signed out or expires.
    Rejected { reason: RejectReason },
    PageCatchUp {
        page_id: PageId,
        catch_up: PageCatchUp,
    },
    PageUpdateAck {
        update_id: Uuid,
        page_id: PageId,
        seq: u64,
    },
    RemotePageUpdate {
        page_id: PageId,
        seq: u64,
        author: UserId,
        update: Vec<u8>,
    },
    Presence {
        page_id: PageId,
        user_id: UserId,
        connection_id: ConnectionId,
        state: Vec<u8>,
    },
    PresenceLeft {
        page_id: PageId,
        user_id: UserId,
        connection_id: ConnectionId,
    },
    /// An `OpenPage` or `Presence` was refused. The connection stays open.
    /// Also sent unprompted with `NotAMember` for an open page when the user loses access to its workspace;
    /// the page is then closed.
    PageRejected {
        page_id: PageId,
        reason: RejectReason,
    },
    /// A `PushPageUpdate` was refused and not stored. The connection stays open.
    PageUpdateRejected {
        update_id: Uuid,
        page_id: PageId,
        reason: RejectReason,
    },
    /// Part `index` (from 0) of the answer to `OpenWorkspace`, each about a megabyte; the last has `is_last`.
    /// Snapshot parts when `last_seen_seq` was 0 and the workspace has committed transactions (the server keeps
    /// the whole log), else the transactions after it.
    /// The total is not known in advance: the server streams the parts as it reads them.
    WorkspaceCatchUpPart {
        workspace_id: WorkspaceId,
        index: u32,
        is_last: bool,
        part: CatchUpPart,
    },
    TransactionAck {
        workspace_id: WorkspaceId,
        transaction_id: TransactionId,
        seq: u64,
    },
    RemoteTransaction {
        workspace_id: WorkspaceId,
        seq: u64,
        author: UserId,
        transaction: Transaction,
    },
    /// The transaction was not committed, logged or relayed. The connection stays open.
    TransactionRejected {
        workspace_id: WorkspaceId,
        transaction_id: TransactionId,
        reason: TransactionRejection,
    },
    /// An `OpenWorkspace` was refused. Also sent unprompted with `NotAMember` for an open workspace when the
    /// user loses access to it; it is then closed. The connection stays open.
    WorkspaceRejected {
        workspace_id: WorkspaceId,
        reason: RejectReason,
    },
    /// Several messages in one frame, sent when they queue up faster than one frame each would go out. Each is
    /// an encoded `ServerMessage` (never another `Batch`); handle them in order, as if they had come one by one.
    Batch { messages: Vec<Vec<u8>> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    UnsupportedProtocolVersion,
    InvalidSession,
    NotAMember,
    ReadOnly,
    PageNotOpen,
    /// The page id already belongs to a different workspace.
    PageInAnotherWorkspace,
    /// A frame that is not a decodable binary `ClientMessage`, or a missing or repeated `Hello`.
    ProtocolViolation,
    WorkspaceNotOpen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CatchUpPart {
    /// Some of the records and property values of the snapshot at `seq`, the same in every part. Together
    /// the parts are the whole snapshot; apply it once the last has arrived.
    Snapshot(WorkspaceSnapshot),
    /// Logged transactions in sequence order, following those of the previous part. Each part can be applied
    /// as it arrives, so a client cut off mid-way resumes from the last one it applied.
    Transactions(Vec<SequencedTransaction>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransactionRejection {
    /// Refused before it was checked: the workspace is not open, or the user may not edit it.
    Refused(RejectReason),
    /// It breaks a rule in `scroll_core::validation`.
    Invalid(ValidationError),
}

pub fn encode<M: Serialize>(message: &M) -> Vec<u8> {
    postcard::to_stdvec(message).expect("protocol messages always serialize")
}

pub fn decode<'a, M: Deserialize<'a>>(frame: &'a [u8]) -> Result<M, postcard::Error> {
    postcard::from_bytes(frame)
}
