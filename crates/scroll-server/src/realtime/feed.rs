//! A workspace's live record stream, kept in the workspace's room: transactions waiting their turn to commit,
//! and the committed ones, held until every connection subscribed to the workspace has read them.
//!
//! Every connection reads the whole stream in sequence order, so the client's view stays the server's order.
//! What changes is when: a connection is woken to read at once for a transaction that changes anything other
//! than rows, and for row changes of a database it has open. Row changes of databases it does not have open
//! wait for its next read, which comes within `Settings::deferred_delivery` or with the next transaction it must
//! read at once. So one person typing in a table wakes only the people looking at that table, and everyone else gets
//! those changes a few hundred at a time.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use axum::body::Bytes;
use scroll_api::protocol::ConnectionId;
use scroll_core::id::{DatabaseId, UserId};
use scroll_core::operation::Transaction;
use tokio::sync::{Notify, oneshot};

use crate::records::CommitOutcome;

/// Transactions with more operations than this are bulk work (publishing, pasting): they commit one at a time,
/// after the small ones waiting with them, so a bulk upload cannot hold up someone editing a cell for long.
const INTERACTIVE_OPERATIONS: usize = 64;
/// Committed transactions are kept past this much encoded data only while connections still have to read
/// them; a connection that falls this far behind is disconnected and catches up from the log.
const RETAINED_BYTES: usize = 64 * 1024 * 1024;
/// How often to look for transactions every reader has passed, which costs a look at every reader.
const TRIM_EVERY: usize = 256;

/// A transaction waiting for the room's sequencing lock. Whichever waiting connection gets the lock commits
/// everything waiting, and answers each through `reply`.
pub struct WaitingCommit {
    pub transaction: Transaction,
    pub author: UserId,
    pub connection: ConnectionId,
    pub reply: oneshot::Sender<Result<CommitOutcome, CommitFailed>>,
}

impl WaitingCommit {
    fn is_bulk(&self) -> bool {
        self.transaction.operations.len() > INTERACTIVE_OPERATIONS
    }
}

/// The commit a transaction was part of failed for a reason unrelated to it, logged by whoever ran the commit.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the commit this transaction was part of failed")]
pub struct CommitFailed;

/// One committed transaction, encoded once for every reader.
pub struct Entry {
    pub seq: u64,
    pub author: ConnectionId,
    /// The database whose rows are all the transaction changes, if that is all it changes.
    pub rows_of: Option<DatabaseId>,
    /// `RemoteTransaction`, for everyone but the author.
    pub remote: Bytes,
    /// `TransactionAck`, for the author.
    pub ack: Bytes,
}

pub struct Reader {
    /// The last sequence number this reader has read (or that its catch-up covered).
    pub read_seq: u64,
    pub wake: Arc<Notify>,
}

/// The reader has fallen further behind than the feed keeps.
#[derive(Debug, Clone, Copy)]
pub struct FellBehind;

#[derive(Default)]
pub struct Feed {
    waiting: Vec<WaitingCommit>,
    /// Groups alternate between small and bulk transactions while both wait, so neither waits on the other
    /// for more than one group.
    last_group_was_bulk: bool,
    /// Consecutive sequence numbers, oldest first.
    entries: VecDeque<Entry>,
    bytes: usize,
    readers: HashMap<ConnectionId, Reader>,
    openers: HashMap<DatabaseId, HashSet<ConnectionId>>,
    appended_since_trim: usize,
}

impl Feed {
    pub fn wait_to_commit(&mut self, commit: WaitingCommit) {
        self.waiting.push(commit);
    }

    /// The next transactions to commit together: every small one waiting, or the bulk one that has waited
    /// longest, taking turns while both kinds wait. A connection waits for its transaction before sending the
    /// next, so taking them out of arrival order never reorders one client's transactions.
    pub fn take_next_group(&mut self) -> Vec<WaitingCommit> {
        let oldest_bulk = self.waiting.iter().position(WaitingCommit::is_bulk);
        let has_small = self.waiting.iter().any(|commit| !commit.is_bulk());
        if let Some(index) = oldest_bulk
            && (!has_small || !self.last_group_was_bulk)
        {
            self.last_group_was_bulk = true;
            return vec![self.waiting.remove(index)];
        }
        self.last_group_was_bulk = false;
        let (small, bulk) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|commit| !commit.is_bulk());
        self.waiting = bulk;
        small
    }

    pub fn has_readers(&self) -> bool {
        !self.readers.is_empty()
    }

    pub fn subscribe(&mut self, connection: ConnectionId, reader: Reader) {
        self.readers.insert(connection, reader);
    }

    pub fn unsubscribe(&mut self, connection: ConnectionId) {
        self.readers.remove(&connection);
        self.openers.retain(|_, openers| {
            openers.remove(&connection);
            !openers.is_empty()
        });
    }

    /// Wakes the reader too, so whatever was held back for it arrives now.
    pub fn open_database(&mut self, connection: ConnectionId, database: DatabaseId) {
        self.openers.entry(database).or_default().insert(connection);
        if let Some(reader) = self.readers.get(&connection) {
            reader.wake.notify_one();
        }
    }

    pub fn close_database(&mut self, connection: ConnectionId, database: DatabaseId) {
        if let Some(openers) = self.openers.get_mut(&database) {
            openers.remove(&connection);
            if openers.is_empty() {
                self.openers.remove(&database);
            }
        }
    }

    /// Adds the next committed transaction and wakes the readers who should see it at once. The author is not
    /// woken: it reads its own answer as soon as the commit returns.
    pub fn append(&mut self, entry: Entry) {
        match entry.rows_of {
            None => {
                for (connection, reader) in &self.readers {
                    if *connection != entry.author {
                        reader.wake.notify_one();
                    }
                }
            }
            Some(database) => {
                for connection in self.openers.get(&database).into_iter().flatten() {
                    if *connection == entry.author {
                        continue;
                    }
                    if let Some(reader) = self.readers.get(connection) {
                        reader.wake.notify_one();
                    }
                }
            }
        }
        self.bytes += entry.remote.len() + entry.ack.len();
        self.entries.push_back(entry);
        self.appended_since_trim += 1;
        if self.appended_since_trim >= TRIM_EVERY || self.bytes > RETAINED_BYTES {
            self.trim();
        }
    }

    /// The frames `connection` has not read yet, oldest first, about `budget` bytes at most (always at least
    /// one when there is any). Its own transactions come back as acks.
    pub fn read(
        &mut self,
        connection: ConnectionId,
        budget: usize,
    ) -> Result<Vec<Bytes>, FellBehind> {
        let first_seq = self.entries.front().map(|entry| entry.seq);
        let Some(reader) = self.readers.get_mut(&connection) else {
            return Ok(Vec::new());
        };
        let Some(first_seq) = first_seq else {
            return Ok(Vec::new());
        };
        if reader.read_seq + 1 < first_seq {
            return Err(FellBehind);
        }
        let start = usize::try_from(reader.read_seq + 1 - first_seq)
            .expect("the feed holds fewer entries than fit in memory");
        let mut frames = Vec::new();
        let mut bytes = 0;
        for entry in self.entries.iter().skip(start) {
            let frame = if entry.author == connection {
                &entry.ack
            } else {
                &entry.remote
            };
            if !frames.is_empty() && bytes + frame.len() > budget {
                break;
            }
            bytes += frame.len();
            frames.push(frame.clone());
            reader.read_seq = entry.seq;
        }
        Ok(frames)
    }

    /// Drops what every reader has read, and past `RETAINED_BYTES` what the slowest have not; they find out
    /// when they next read.
    fn trim(&mut self) {
        self.appended_since_trim = 0;
        let oldest_unread = self
            .readers
            .values()
            .map(|reader| reader.read_seq + 1)
            .min()
            .unwrap_or(u64::MAX);
        while let Some(entry) = self.entries.front() {
            if entry.seq >= oldest_unread && self.bytes <= RETAINED_BYTES {
                break;
            }
            self.bytes -= entry.remote.len() + entry.ack.len();
            self.entries.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt;
    use uuid::Uuid;

    use super::*;

    fn connection() -> ConnectionId {
        ConnectionId(Uuid::now_v7())
    }

    fn entry(seq: u64, author: ConnectionId, rows_of: Option<DatabaseId>) -> Entry {
        Entry {
            seq,
            author,
            rows_of,
            remote: Bytes::from(format!("remote {seq}")),
            ack: Bytes::from(format!("ack {seq}")),
        }
    }

    fn reader(feed: &mut Feed, read_seq: u64) -> (ConnectionId, Arc<Notify>) {
        let id = connection();
        let wake = Arc::new(Notify::new());
        feed.subscribe(
            id,
            Reader {
                read_seq,
                wake: wake.clone(),
            },
        );
        (id, wake)
    }

    fn was_woken(wake: &Notify) -> bool {
        wake.notified().now_or_never().is_some()
    }

    #[test]
    fn row_changes_wake_only_readers_with_the_database_open_but_everyone_reads_them_in_order() {
        let mut feed = Feed::default();
        let database = DatabaseId::generate();
        let (looking, looking_wake) = reader(&mut feed, 0);
        let (elsewhere, elsewhere_wake) = reader(&mut feed, 0);
        feed.open_database(looking, database);
        assert!(
            was_woken(&looking_wake),
            "opening delivers what was held back"
        );

        let author = connection();
        feed.append(entry(1, author, Some(database)));
        assert!(was_woken(&looking_wake));
        assert!(!was_woken(&elsewhere_wake));

        feed.append(entry(2, author, None));
        assert!(was_woken(&elsewhere_wake));
        let read = feed.read(elsewhere, usize::MAX).unwrap();
        assert_eq!(read, vec![Bytes::from("remote 1"), Bytes::from("remote 2")]);
        assert!(feed.read(elsewhere, usize::MAX).unwrap().is_empty());
    }

    #[test]
    fn the_author_reads_its_own_transactions_as_acks() {
        let mut feed = Feed::default();
        let (author, _) = reader(&mut feed, 0);
        feed.append(entry(1, author, None));
        assert_eq!(
            feed.read(author, usize::MAX).unwrap(),
            vec![Bytes::from("ack 1")]
        );
    }

    #[test]
    fn a_read_stops_near_the_budget_but_always_makes_progress() {
        let mut feed = Feed::default();
        let (id, _) = reader(&mut feed, 0);
        let author = connection();
        for seq in 1..=3 {
            feed.append(entry(seq, author, None));
        }
        assert_eq!(feed.read(id, 1).unwrap().len(), 1);
        assert_eq!(feed.read(id, 20).unwrap().len(), 2);
    }

    #[test]
    fn entries_go_once_every_reader_has_read_them_and_a_reader_left_behind_finds_out() {
        let mut feed = Feed::default();
        let (slow, _) = reader(&mut feed, 0);
        let author = connection();
        for seq in 1..=TRIM_EVERY as u64 {
            feed.append(entry(seq, author, None));
        }
        assert_eq!(
            feed.entries.len(),
            TRIM_EVERY,
            "the slow reader still needs them"
        );

        feed.read(slow, usize::MAX).unwrap();
        for seq in TRIM_EVERY as u64 + 1..=2 * TRIM_EVERY as u64 {
            feed.append(entry(seq, author, None));
        }
        assert_eq!(feed.entries.len(), TRIM_EVERY);

        let (late, _) = reader(&mut feed, 1);
        assert!(feed.read(late, usize::MAX).is_err());
    }

    #[test]
    fn small_and_bulk_transactions_take_turns_and_bulk_ones_go_alone() {
        let mut feed = Feed::default();
        let commit = |operations: usize| {
            let (reply, _) = oneshot::channel();
            let operation = scroll_core::operation::Operation::Delete(
                scroll_core::operation::RecordId::Tag(scroll_core::id::TagId::generate()),
            );
            WaitingCommit {
                transaction: Transaction::new(vec![operation; operations]),
                author: UserId(Uuid::now_v7()),
                connection: connection(),
                reply,
            }
        };
        let sizes = |group: Vec<WaitingCommit>| -> Vec<usize> {
            group
                .iter()
                .map(|commit| commit.transaction.operations.len())
                .collect()
        };
        feed.wait_to_commit(commit(2_000));
        assert_eq!(sizes(feed.take_next_group()), vec![2_000]);

        feed.wait_to_commit(commit(2_001));
        feed.wait_to_commit(commit(1));
        feed.wait_to_commit(commit(2_002));
        feed.wait_to_commit(commit(3));
        assert_eq!(
            sizes(feed.take_next_group()),
            vec![1, 3],
            "small ones overtake"
        );
        feed.wait_to_commit(commit(4));
        assert_eq!(
            sizes(feed.take_next_group()),
            vec![2_001],
            "then a bulk one"
        );
        assert_eq!(sizes(feed.take_next_group()), vec![4]);
        assert_eq!(sizes(feed.take_next_group()), vec![2_002]);
        assert!(feed.take_next_group().is_empty());
    }
}
