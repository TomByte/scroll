//! Committing the transactions pushed to a workspace, a group at a time, and logging them in sequence.

use deadpool_postgres::{Pool, Transaction as DbTransaction};
use scroll_core::id::{DatabaseId, UserId, WorkspaceId};
use scroll_core::operation::Transaction;
use scroll_core::validation::ValidationError;
use uuid::Uuid;

use super::apply::WorkspaceRecords;
use super::{RecordError, encode};
use crate::db::db_seq;

pub struct PushedTransaction<'a> {
    pub author: UserId,
    pub transaction: &'a Transaction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommitOutcome {
    Committed {
        seq: u64,
        /// The database whose rows are all the transaction changed, if that is all it changed.
        rows_of: Option<DatabaseId>,
    },
    /// A resend of a transaction committed earlier; nothing was written.
    AlreadyCommitted { seq: u64 },
    /// Nothing was written.
    Invalid(ValidationError),
}

/// Validates and applies each transaction's operations, and logs each under the workspace's next sequence
/// number, all in one database transaction: the group shares one wait for the workspace lock and one flush to
/// disk. Each transaction is still all or nothing: one that breaks a rule uses no sequence number, and the
/// others commit. Locking the workspace row makes concurrent commits (from other server processes too) take
/// turns, so the sequence stays gapless.
pub async fn commit_group(
    pool: &Pool,
    workspace_id: WorkspaceId,
    group: &[PushedTransaction<'_>],
) -> Result<Vec<CommitOutcome>, RecordError> {
    let mut client = pool.get().await?;
    // Nearly every group is all valid, and committing it without a savepoint per transaction lets the group's
    // property values go in one statement: a handful of round trips however large the group. A group with an
    // invalid transaction is rolled back and committed again, rolling back to a savepoint just the invalid ones.
    let attempt =
        commit_attempt(&mut client, workspace_id, group, OnInvalid::RollBackGroup).await?;
    match attempt {
        Attempt::Committed(outcomes) => Ok(outcomes),
        Attempt::HadInvalid(error) if group.len() == 1 => Ok(vec![CommitOutcome::Invalid(error)]),
        Attempt::HadInvalid(_) => {
            let retried = commit_attempt(
                &mut client,
                workspace_id,
                group,
                OnInvalid::RollBackTransaction,
            )
            .await?;
            match retried {
                Attempt::Committed(outcomes) => Ok(outcomes),
                Attempt::HadInvalid(_) => unreachable!(
                    "with a savepoint each, an invalid transaction is rolled back on its own"
                ),
            }
        }
    }
}

/// What a commit attempt does when a transaction in the group breaks a rule.
#[derive(Clone, Copy)]
enum OnInvalid {
    /// Gives up on the whole group.
    RollBackGroup,
    /// Rolls back that transaction alone, to a savepoint made before it.
    RollBackTransaction,
}

enum Attempt {
    Committed(Vec<CommitOutcome>),
    /// Nothing was written.
    HadInvalid(ValidationError),
}

async fn commit_attempt(
    client: &mut deadpool_postgres::Object,
    workspace_id: WorkspaceId,
    group: &[PushedTransaction<'_>],
    on_invalid: OnInvalid,
) -> Result<Attempt, RecordError> {
    let tx = client.transaction().await?;
    let records = WorkspaceRecords::new(&tx, workspace_id);
    let (last_seq, mut committed) = records.lock_workspace(group).await?;
    let mut log = LogRows::default();
    let mut outcomes = Vec::with_capacity(group.len());
    let mut savepoint = Savepoint::new(&tx);
    for pushed in group {
        let id = pushed.transaction.id.0;
        if let Some(&seq) = committed.get(&id) {
            outcomes.push(CommitOutcome::AlreadyCommitted { seq });
            continue;
        }
        if let OnInvalid::RollBackTransaction = on_invalid {
            savepoint.start().await?;
        }
        let applied = match on_invalid {
            OnInvalid::RollBackGroup => records.apply_transaction(pushed.transaction).await,
            // Written inside the savepoint, so a page id found taken on writing counts against this
            // transaction, and the next one's rollback cannot undo its writes.
            OnInvalid::RollBackTransaction => {
                match records.apply_transaction(pushed.transaction).await {
                    Ok(rows_of) => records.write_unwritten().await.map(|()| rows_of),
                    Err(error) => Err(error),
                }
            }
        };
        match (applied, on_invalid) {
            (Ok(rows_of), _) => {
                let seq = last_seq + log.len() as u64 + 1;
                log.push(seq, pushed)?;
                committed.insert(id, seq);
                outcomes.push(CommitOutcome::Committed { seq, rows_of });
            }
            // Dropping the database transaction rolls it back.
            (Err(RecordError::Invalid(error)), OnInvalid::RollBackGroup) => {
                return Ok(Attempt::HadInvalid(error));
            }
            (Err(RecordError::Invalid(error)), OnInvalid::RollBackTransaction) => {
                savepoint.roll_back().await?;
                records.forget_unwritten();
                outcomes.push(CommitOutcome::Invalid(error));
            }
            (Err(error), _) => return Err(error),
        }
    }
    match records.write_unwritten().await {
        Ok(()) => {}
        // A page id found taken only now; the retry with savepoints tells which transaction it was.
        Err(RecordError::Invalid(error)) => return Ok(Attempt::HadInvalid(error)),
        Err(error) => return Err(error),
    }
    if !log.is_empty() {
        log.write(&tx, workspace_id).await?;
    }
    tx.commit().await?;
    Ok(Attempt::Committed(outcomes))
}

/// A savepoint before each transaction of a group, so an invalid one can be undone alone. Each is released by
/// starting the next under the same name, in one round trip.
struct Savepoint<'a> {
    tx: &'a DbTransaction<'a>,
    is_open: bool,
}

impl<'a> Savepoint<'a> {
    fn new(tx: &'a DbTransaction<'a>) -> Self {
        Self { tx, is_open: false }
    }

    async fn start(&mut self) -> Result<(), tokio_postgres::Error> {
        let sql = if self.is_open {
            "RELEASE SAVEPOINT pushed; SAVEPOINT pushed"
        } else {
            "SAVEPOINT pushed"
        };
        self.tx.batch_execute(sql).await?;
        self.is_open = true;
        Ok(())
    }

    async fn roll_back(&mut self) -> Result<(), tokio_postgres::Error> {
        self.tx.batch_execute("ROLLBACK TO SAVEPOINT pushed").await
    }
}

/// The log rows of a group's committed transactions, written together.
#[derive(Default)]
struct LogRows {
    seqs: Vec<i64>,
    transaction_ids: Vec<Uuid>,
    authors: Vec<Uuid>,
    transactions: Vec<Vec<u8>>,
}

impl LogRows {
    fn len(&self) -> usize {
        self.seqs.len()
    }

    fn is_empty(&self) -> bool {
        self.seqs.is_empty()
    }

    fn push(&mut self, seq: u64, pushed: &PushedTransaction<'_>) -> Result<(), RecordError> {
        self.seqs.push(db_seq(seq));
        self.transaction_ids.push(pushed.transaction.id.0);
        self.authors.push(pushed.author.0);
        self.transactions.push(encode(pushed.transaction)?);
        Ok(())
    }

    async fn write(
        &self,
        tx: &DbTransaction<'_>,
        workspace_id: WorkspaceId,
    ) -> Result<(), tokio_postgres::Error> {
        let last_seq = self.seqs.last().copied().unwrap_or_default();
        let statement = tx
            .prepare_cached(
                "WITH logged AS (
                    INSERT INTO record_transactions (workspace_id, seq, transaction_id, author_id, transaction)
                    SELECT $1, * FROM UNNEST($2::bigint[], $3::uuid[], $4::uuid[], $5::bytea[])
                )
                UPDATE workspaces SET last_record_seq = $6 WHERE id = $1",
            )
            .await?;
        tx.execute(
            &statement,
            &[
                &workspace_id.0,
                &self.seqs,
                &self.transaction_ids,
                &self.authors,
                &self.transactions,
                &last_seq,
            ],
        )
        .await?;
        Ok(())
    }
}
