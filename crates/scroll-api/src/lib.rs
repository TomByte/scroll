//! Everything that crosses the wire between the app and a Scroll server: the sync protocol over the WebSocket,
//! the JSON shapes, error codes and paths of the HTTP API, and the rules both ends of a browser sign-in follow.
//! The server and the app's sync client both build on these, so the two cannot drift apart.

pub mod accounts;
pub mod browser_sign_in;
pub mod connection;
pub mod errors;
pub mod limits;
pub mod paths;
pub mod protocol;
