//! Serves a workspace file over MCP outside the app, for trying the server with a real client:
//!
//!     cargo run -p scroll-mcp --example serve -- <workspace file> [port]
//!
//! A new token is made for each run and printed to stderr only; the server writes to the file,
//! so point it at a copy. Stop it with Ctrl-C.

use std::path::PathBuf;

use scroll_mcp::{Access, DEFAULT_PORT, MCP_PATH, McpConfig, McpEvent, McpServer, generate_token};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: serve <workspace file> [port]")?,
    );
    let port = arguments
        .next()
        .map_or(Ok(DEFAULT_PORT), |port| port.parse())?;
    let token = generate_token()?;
    let (server, mut events) = McpServer::start(McpConfig {
        workspace_path: path,
        port,
        token: token.clone(),
        access: Access::ReadWrite,
    })?;
    eprintln!("serving http://{}{MCP_PATH}", server.address());
    eprintln!("token: {token}");
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    runtime.block_on(async {
        while let Some(event) = events.next().await {
            match event {
                McpEvent::RecordsChanged(changes) => eprintln!("records changed: {changes:?}"),
                McpEvent::PageContentChanged { page_id, update } => {
                    eprintln!("page {} changed ({} bytes)", page_id.0, update.len())
                }
            }
        }
    });
    Ok(())
}
