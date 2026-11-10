//! Record storage: validating and committing transactions, and catching clients up.
//!
//! The rules come from `scroll_core::validation` (its module comment lists which checks apply to which
//! operation) and field changes use the pure `apply_to` functions in `scroll_core::operation`. This module only
//! looks up what those need, inside the committing transaction, so each operation sees the ones before it.

mod apply;
mod catch_up;
mod classify;
mod commit;
mod write_buffer;

use deadpool_postgres::PoolError;
use scroll_core::validation::ValidationError;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use catch_up::{CatchUpFrom, Pin, stream_workspace_catch_up};
pub use commit::{CommitOutcome, PushedTransaction, commit_group};

use crate::db::DbError;

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("a stored record or transaction could not be encoded or decoded: {0}")]
    Encoding(#[from] postcard::Error),
    #[error("the connection stopped taking the catch-up")]
    CatchUpAbandoned,
}

impl From<tokio_postgres::Error> for RecordError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(error.into())
    }
}

impl From<PoolError> for RecordError {
    fn from(error: PoolError) -> Self {
        Self::Database(error.into())
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, RecordError> {
    Ok(postcard::to_stdvec(value)?)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, RecordError> {
    Ok(postcard::from_bytes(bytes)?)
}
