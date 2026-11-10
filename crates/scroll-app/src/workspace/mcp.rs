//! What an AI tool changed through the MCP server, which writes the file with its own connection.

use gpui::Context;
use scroll_mcp::McpEvent;

use super::loading::with_parents;
use super::*;

impl Workspace {
    /// Already in the file (and, in a linked workspace, queued for the server): tell the engine,
    /// re-read the records as for changes from elsewhere, and hand page content to an open
    /// editor the way the server's updates arrive.
    pub fn mcp_changed(&mut self, event: McpEvent, cx: &mut Context<Self>) {
        if let Some(engine) = self.sync.as_ref() {
            engine.local_change_queued();
        }
        match event {
            McpEvent::RecordsChanged(changes) => self
                .records_changed_elsewhere(with_parents(changes.records, changes.child_lists), cx),
            McpEvent::PageContentChanged { page_id, update } => cx.emit(WorkspaceEvent::Page(
                page_id,
                PageSyncEvent::Updates(vec![update]),
            )),
        }
    }
}
