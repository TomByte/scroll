//! Page content: Loro updates per page, their catch-up, and compaction into snapshots.

mod compaction;
mod store;

pub use compaction::Compactor;
pub use store::*;
