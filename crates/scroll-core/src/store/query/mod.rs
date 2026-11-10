//! Reads that answer a screen: a table view's rows, the sidebar's tree, tags, search.

pub(in crate::store) mod table;
pub(in crate::store) mod tags;
pub(in crate::store) mod tree;

pub use crate::store::search::SearchHit;
pub use table::ViewRowIds;
pub use tags::TagUsage;
pub use tree::ChildPage;
