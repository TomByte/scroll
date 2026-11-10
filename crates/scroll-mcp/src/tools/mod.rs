//! The tools AI clients call: what each is called and takes (`catalog`), and running them against
//! the workspace file. Reads query the store directly; writes go through the same store calls as
//! the app's own edits and report what they changed.

mod blocks;
mod catalog;
mod databases;
mod pages;
mod values;

use std::sync::{Mutex, PoisonError};

use scroll_core::id::{DatabaseId, PageId, ViewId};
use scroll_core::operation::Transaction;
use scroll_core::page_doc::{PageDoc, PageDocError};
use scroll_core::position::PositionError;
use scroll_core::store::{StoreError, WorkspaceStore};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::server::{Access, McpEvent};

pub(crate) use catalog::definitions;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    ListPages,
    GetPage,
    Search,
    QueryDatabase,
    CreatePage,
    UpdatePage,
    AppendBlocks,
    UpdateBlock,
    DeleteBlock,
    CreateRow,
    UpdateRow,
}

impl Tool {
    const ALL: [Tool; 11] = [
        Tool::ListPages,
        Tool::GetPage,
        Tool::Search,
        Tool::QueryDatabase,
        Tool::CreatePage,
        Tool::UpdatePage,
        Tool::AppendBlocks,
        Tool::UpdateBlock,
        Tool::DeleteBlock,
        Tool::CreateRow,
        Tool::UpdateRow,
    ];

    fn name(self) -> &'static str {
        match self {
            Tool::ListPages => "list_pages",
            Tool::GetPage => "get_page",
            Tool::Search => "search",
            Tool::QueryDatabase => "query_database",
            Tool::CreatePage => "create_page",
            Tool::UpdatePage => "update_page",
            Tool::AppendBlocks => "append_blocks",
            Tool::UpdateBlock => "update_block",
            Tool::DeleteBlock => "delete_block",
            Tool::CreateRow => "create_row",
            Tool::UpdateRow => "update_row",
        }
    }

    fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    fn changes_workspace(self) -> bool {
        match self {
            Tool::ListPages | Tool::GetPage | Tool::Search | Tool::QueryDatabase => false,
            Tool::CreatePage
            | Tool::UpdatePage
            | Tool::AppendBlocks
            | Tool::UpdateBlock
            | Tool::DeleteBlock
            | Tool::CreateRow
            | Tool::UpdateRow => true,
        }
    }
}

/// Why a tool did not do what was asked. The message goes to the model, so it says what to fix.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolError {
    #[error("there is no tool called {0}")]
    UnknownTool(String),
    #[error("this workspace is read-only for AI tools")]
    ReadOnly,
    #[error("the arguments do not fit the tool's schema: {0}")]
    Arguments(serde_json::Error),
    #[error("there is no page {}", .0.0)]
    PageNotFound(PageId),
    #[error("there is no database {}", .0.0)]
    DatabaseNotFound(DatabaseId),
    #[error("database {} has no view {}", .database.0, .view.0)]
    ViewNotFound { database: DatabaseId, view: ViewId },
    #[error("page {} is not a database row", .0.0)]
    NotARow(PageId),
    #[error("{0:?} is not a block id; block ids look like \"12@3456789\"")]
    InvalidBlockId(String),
    #[error("{0:?} is not a block kind")]
    UnknownBlockKind(String),
    #[error("the database has no property called {0:?}")]
    UnknownProperty(String),
    #[error("{property:?} has no option called {option:?}; its options are: {options}")]
    UnknownOption {
        property: String,
        option: String,
        options: String,
    },
    #[error("{property:?} takes {expected}, not {value}")]
    WrongValue {
        property: String,
        expected: &'static str,
        value: Value,
    },
    #[error("{property:?} cannot be filtered with {operator}")]
    UnsupportedFilter {
        property: String,
        operator: &'static str,
    },
    #[error("filtering {property:?} with {operator} needs a value")]
    MissingFilterValue {
        property: String,
        operator: &'static str,
    },
    #[error("nothing to change: give at least one of the optional fields")]
    NothingToChange,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    PageDoc(#[from] PageDocError),
    #[error(transparent)]
    Position(#[from] PositionError),
}

/// The workspace file as the tools see it.
pub(crate) struct Workspace {
    store: Mutex<WorkspaceStore>,
    access: Access,
    events: UnboundedSender<McpEvent>,
}

impl Workspace {
    pub(crate) fn new(
        store: WorkspaceStore,
        access: Access,
        events: UnboundedSender<McpEvent>,
    ) -> Self {
        Workspace {
            store: Mutex::new(store),
            access,
            events,
        }
    }

    pub(crate) fn access(&self) -> Access {
        self.access
    }
}

/// Runs the tool called `name` and returns what it found or did, as JSON for the model.
pub(crate) fn call(
    workspace: &Workspace,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    let tool = Tool::from_name(name).ok_or_else(|| ToolError::UnknownTool(name.to_string()))?;
    if tool.changes_workspace() && workspace.access == Access::ReadOnly {
        return Err(ToolError::ReadOnly);
    }
    // Only a panic mid-call poisons the lock, and SQLite rolled back whatever that call left
    // unfinished, so the store is still usable.
    let mut store = workspace
        .store
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut writer = Writer {
        store: &mut store,
        events: &workspace.events,
    };
    match tool {
        Tool::ListPages => pages::list_pages(writer.store, parse(arguments)?),
        Tool::GetPage => pages::get_page(writer.store, parse(arguments)?),
        Tool::Search => pages::search(writer.store, parse(arguments)?),
        Tool::QueryDatabase => databases::query_database(writer.store, parse(arguments)?),
        Tool::CreatePage => pages::create_page(&mut writer, parse(arguments)?),
        Tool::UpdatePage => pages::update_page(&mut writer, parse(arguments)?),
        Tool::AppendBlocks => blocks::append_blocks(&mut writer, parse(arguments)?),
        Tool::UpdateBlock => blocks::update_block(&mut writer, parse(arguments)?),
        Tool::DeleteBlock => blocks::delete_block(&mut writer, parse(arguments)?),
        Tool::CreateRow => databases::create_row(&mut writer, parse(arguments)?),
        Tool::UpdateRow => databases::update_row(&mut writer, parse(arguments)?),
    }
}

/// Arguments left out entirely read as an empty object, so tools whose arguments are all
/// optional can be called without any.
fn parse<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolError> {
    let arguments = match arguments {
        Value::Null => Value::Object(Default::default()),
        arguments => arguments,
    };
    serde_json::from_value(arguments).map_err(ToolError::Arguments)
}

/// The store while a writing tool runs, and where it reports what it changed.
pub(crate) struct Writer<'a> {
    store: &'a mut WorkspaceStore,
    events: &'a UnboundedSender<McpEvent>,
}

impl Writer<'_> {
    /// Validates and applies `transaction` as the app applies its own, queueing it for the server
    /// in a linked workspace.
    fn apply(&mut self, transaction: &Transaction) -> Result<(), ToolError> {
        let changes = self.store.apply_local_transaction(transaction)?;
        self.report(McpEvent::RecordsChanged(changes));
        Ok(())
    }

    /// Runs `edit` on the page's stored document and stores what it changed as one update, as the
    /// editor does. Nothing is stored if `edit` fails part way.
    ///
    /// The document is loaded fresh, so it has a peer id of its own (Loro picks one at random)
    /// and never shares one with the app's live document. The search index is refreshed rather
    /// than the log compacted: the app and the sync engine own compaction.
    fn edit_content<T>(
        &mut self,
        page: PageId,
        edit: impl FnOnce(&PageDoc) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        if self.store.page(page)?.is_none() {
            return Err(ToolError::PageNotFound(page));
        }
        let mut doc = self.store.load_page_doc(page)?;
        let edited = edit(&doc)?;
        if let Some(update) = doc.take_local_update()? {
            self.store.record_local_page_update(page, &update)?;
            self.report(McpEvent::PageContentChanged {
                page_id: page,
                update,
            });
            self.store.refresh_page_search(page)?;
        }
        Ok(edited)
    }

    fn report(&self, event: McpEvent) {
        if self.events.send(event).is_err() {
            log::debug!("the app stopped listening to the MCP server's events");
        }
    }
}

/// The page, or `PageNotFound`.
fn require_page(
    store: &WorkspaceStore,
    page: PageId,
) -> Result<scroll_core::record::Page, ToolError> {
    store.page(page)?.ok_or(ToolError::PageNotFound(page))
}
