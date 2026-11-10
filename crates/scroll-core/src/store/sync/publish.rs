//! What linking a local workspace queues for the server: every record, created in an order the
//! server's validation accepts, then each page's content.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::id::{DatabaseId, PageId};
use crate::operation::{FieldChange, Operation, Record, Transaction};
use crate::store::content::log::{delete_content, pages_with_content, stored_page_doc};
use crate::store::kinds::database::{database_hosted_by, read_database};
use crate::store::kinds::page::{child_page_ids, read_page};
use crate::store::kinds::value::for_each_property_value_by_page;
use crate::store::kinds::{KINDS, Kind};
use crate::store::sync::confirmed::{ServerState, track_server_state};
use crate::store::sync::queue::{self, QueuedChange, QueuedPageUpdate};
use crate::store::{StoreError, encode};

/// Most operations in one publishing transaction, so the server holds its lock briefly.
pub(crate) const PUBLISH_CHUNK_OPERATIONS: usize = 2_000;

/// Roughly the most encoded bytes in one publishing transaction, well under the wire limit.
pub(crate) const PUBLISH_CHUNK_BYTES: usize = 1 << 20;

/// Queues transactions creating every record, in an order the server's validation accepts
/// without sorting by dependency: tags, then pages already linked, parents before children, each
/// database right after the page that hosts it (so its rows can follow), then views, tag
/// memberships, and property values in (page, property) order, the order the server stores them.
/// Every prefix of this order is valid, so any split point keeps each transaction valid. Records
/// are read and queued as they are reached; only page ids waiting to be visited are held.
pub(in crate::store) fn queue_publishing(conn: &Connection) -> Result<(), StoreError> {
    let mut chunks = PublishingChunks::default();
    queue_kinds_published(Stage::BeforePages, conn, &mut chunks)?;
    queue_page_tree(conn, &mut chunks)?;
    queue_kinds_published(Stage::AfterPages, conn, &mut chunks)?;
    for_each_property_value_by_page(conn, |page, property, value| {
        let change = FieldChange::PropertyValue {
            page,
            property,
            value: Some(value),
        };
        chunks.push(conn, Operation::Set(change))
    })?;
    chunks.flush(conn)
}

/// When a kind's records are published relative to the page tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    BeforePages,
    WithPageTree,
    AfterPages,
}

fn stage(kind: Kind) -> Stage {
    match kind {
        Kind::Tag => Stage::BeforePages,
        Kind::Page | Kind::Database => Stage::WithPageTree,
        Kind::View | Kind::PageTag => Stage::AfterPages,
    }
}

/// Creates every record of each kind published at `published`, kind by kind in `KINDS` order.
fn queue_kinds_published(
    published: Stage,
    conn: &Connection,
    chunks: &mut PublishingChunks,
) -> Result<(), StoreError> {
    for kind in KINDS.into_iter().filter(|kind| stage(*kind) == published) {
        for record in kind.all_records(conn)? {
            chunks.push(conn, Operation::Create(record))?;
        }
    }
    Ok(())
}

/// Every page, each after its parent and after the database it is a row of, depth first from
/// the top level. A page that is a row of a database not created yet waits, with the pages under
/// it, until that database's host page has been reached.
fn queue_page_tree(conn: &Connection, chunks: &mut PublishingChunks) -> Result<(), StoreError> {
    let mut waiting: Vec<PageId> = child_page_ids(conn, None)?;
    waiting.reverse();
    let mut created_databases = HashSet::new();
    let mut waiting_for_database: HashMap<DatabaseId, Vec<PageId>> = HashMap::new();
    while let Some(id) = waiting.pop() {
        let Some(page) = read_page(conn, id)? else {
            continue;
        };
        if let Some(database) = page.row_of
            && !created_databases.contains(&database)
        {
            waiting_for_database.entry(database).or_default().push(id);
            continue;
        }
        chunks.push(conn, Operation::Create(Record::Page(page)))?;
        if let Some(database) = database_hosted_by(conn, id)?
            .map(|id| read_database(conn, id))
            .transpose()?
            .flatten()
        {
            created_databases.insert(database.id);
            waiting.extend(
                waiting_for_database
                    .remove(&database.id)
                    .unwrap_or_default(),
            );
            chunks.push(conn, Operation::Create(Record::Database(database)))?;
        }
        let mut children = child_page_ids(conn, Some(id))?;
        children.reverse();
        waiting.extend(children);
    }
    Ok(())
}

/// Gathers publishing operations into transactions the wire and the server's lock can take, and
/// queues each as it fills. Each created record is marked as not on the server yet, so a server
/// snapshot without it keeps it until it is acknowledged.
#[derive(Default)]
struct PublishingChunks {
    operations: Vec<Operation>,
    bytes: usize,
}

impl PublishingChunks {
    fn push(&mut self, conn: &Connection, operation: Operation) -> Result<(), StoreError> {
        if let Operation::Create(record) = &operation {
            track_server_state(conn, record.id(), &ServerState(None))?;
        }
        let operation_bytes = encode(&operation)?.len();
        let is_full = self.operations.len() == PUBLISH_CHUNK_OPERATIONS
            || self.bytes + operation_bytes > PUBLISH_CHUNK_BYTES;
        if is_full {
            self.flush(conn)?;
        }
        self.bytes += operation_bytes;
        self.operations.push(operation);
        Ok(())
    }

    fn flush(&mut self, conn: &Connection) -> Result<(), StoreError> {
        if self.operations.is_empty() {
            return Ok(());
        }
        let transaction = Transaction::new(std::mem::take(&mut self.operations));
        self.bytes = 0;
        queue::enqueue(conn, &QueuedChange::Transaction(transaction))
    }
}

/// Moves each page's stored content into the queue as one update. The local log used local
/// sequence numbers, so it is cleared; acknowledged updates come back under the server's numbers.
pub(in crate::store) fn queue_page_contents(conn: &Connection) -> Result<(), StoreError> {
    for page_id in pages_with_content(conn)? {
        let update = stored_page_doc(conn, page_id)?.export_snapshot()?;
        let change = QueuedChange::PageUpdate(QueuedPageUpdate {
            update_id: uuid::Uuid::now_v7(),
            page_id,
            update,
        });
        queue::enqueue(conn, &change)?;
        delete_content(conn, page_id)?;
    }
    Ok(())
}
