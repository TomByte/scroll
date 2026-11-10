//! Live sync over the WebSocket: connections, page rooms, each workspace's feed of committed transactions, and
//! the group commit that fills it.

mod commit_leader;
mod connection;
mod feed;
pub mod hub;
mod page_topic;
mod workspace_topic;

pub use connection::upgrade;
