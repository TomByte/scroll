//! A Model Context Protocol server, so AI tools can read, search and change the workspace open in
//! the app. No GPUI: the app starts it for its workspace file and drains its events.
//!
//! It runs on a thread of its own with its own connection to the workspace file, beside the app's
//! storage thread and the sync engine, as the table view does. Record changes go through
//! `apply_local_transaction` and page content through `record_local_page_update`, so validation
//! and, in a linked workspace, the queue for the server apply as for the app's own edits. Each
//! write is reported as an `McpEvent`; the app refreshes what it shows from them and tells its
//! sync engine.
//!
//! The transport is MCP's Streamable HTTP in its smallest form: `POST /mcp` on 127.0.0.1 takes
//! one JSON-RPC message and answers with JSON. Every request must carry `Authorization: Bearer`
//! with the token from `load_or_create_token`; requests from a browser (any `Origin`, or a
//! `Host` other than this machine) are refused.

mod http;
mod rpc;
mod server;
mod token;
mod tools;

pub use server::{
    Access, DEFAULT_PORT, MCP_PATH, McpConfig, McpEvent, McpEvents, McpServer, StartError,
};
pub use token::{TokenError, generate_token, load_or_create_token};
