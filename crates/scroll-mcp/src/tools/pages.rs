//! Pages as records: listing, reading whole, searching, creating and changing their fields.

use scroll_core::id::PageId;
use scroll_core::operation::{FieldChange, Operation, PageField, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page};
use scroll_core::store::WorkspaceStore;
use serde::Deserialize;
use serde_json::{Value, json};

use super::blocks::{BlockInput, blocks_to_json, insert_blocks};
use super::databases::database_to_json;
use super::values::values_to_json;
use super::{ToolError, Writer, require_page};

const DEFAULT_PAGE_LIST: usize = 100;
const MAXIMUM_PAGE_LIST: usize = 500;
const DEFAULT_SEARCH_RESULTS: usize = 20;
const MAXIMUM_SEARCH_RESULTS: usize = 100;
/// How many siblings are read at a time while looking for the last one.
const SIBLING_BATCH: usize = 2_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListPages {
    parent_id: Option<PageId>,
    after: Option<PageId>,
    limit: Option<usize>,
}

pub(super) fn list_pages(store: &WorkspaceStore, arguments: ListPages) -> Result<Value, ToolError> {
    let limit = arguments
        .limit
        .unwrap_or(DEFAULT_PAGE_LIST)
        .clamp(1, MAXIMUM_PAGE_LIST);
    let children = store.page_children(arguments.parent_id, arguments.after, limit)?;
    let next_after = (children.len() == limit)
        .then(|| children.last().map(|child| child.page.id.0))
        .flatten();
    let pages: Vec<Value> = children
        .iter()
        .map(|child| {
            json!({
                "id": child.page.id.0,
                "title": child.page.title,
                "icon": child.page.icon,
                "child_count": child.child_count,
                "database_id": child.hosted_database.map(|database| database.0),
            })
        })
        .collect();
    Ok(json!({"pages": pages, "next_after": next_after}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetPage {
    page_id: PageId,
}

pub(super) fn get_page(store: &WorkspaceStore, arguments: GetPage) -> Result<Value, ToolError> {
    let page = require_page(store, arguments.page_id)?;
    let tags: Vec<String> = store
        .page_tags(page.id)?
        .into_iter()
        .map(|tag| tag.name)
        .collect();
    let blocks = blocks_to_json(&store.load_page_doc(page.id)?)?;
    let mut found = json!({
        "id": page.id.0,
        "title": page.title,
        "icon": page.icon,
        "parent_id": page.parent.map(|parent| parent.0),
        "is_archived": page.is_archived,
        "tags": tags,
        "blocks": blocks,
    });
    if let Some(database) = store.database_hosted_by(page.id)? {
        found["database"] = database_to_json(store, &database)?;
    }
    if let Some(database) = page
        .row_of
        .map(|id| store.database(id))
        .transpose()?
        .flatten()
    {
        found["row"] = row_to_json(store, &page, &database)?;
    }
    Ok(found)
}

fn row_to_json(
    store: &WorkspaceStore,
    page: &Page,
    database: &Database,
) -> Result<Value, ToolError> {
    let values = store.property_values(page.id)?;
    Ok(json!({
        "database_id": database.id.0,
        "values": values_to_json(database, &values),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Search {
    query: String,
    limit: Option<usize>,
}

pub(super) fn search(store: &WorkspaceStore, arguments: Search) -> Result<Value, ToolError> {
    let limit = arguments
        .limit
        .unwrap_or(DEFAULT_SEARCH_RESULTS)
        .clamp(1, MAXIMUM_SEARCH_RESULTS);
    let results: Vec<Value> = store
        .search(&arguments.query, limit)?
        .into_iter()
        .map(|hit| json!({"page_id": hit.page_id.0, "title": hit.title, "snippet": hit.snippet}))
        .collect();
    Ok(json!({"results": results}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreatePage {
    parent_id: Option<PageId>,
    title: String,
    icon: Option<String>,
    #[serde(default)]
    blocks: Vec<BlockInput>,
}

pub(super) fn create_page(writer: &mut Writer, arguments: CreatePage) -> Result<Value, ToolError> {
    let last_sibling = last_child_position(writer.store, arguments.parent_id)?;
    let page = Page {
        id: PageId::generate(),
        parent: arguments.parent_id,
        title: arguments.title,
        icon: arguments.icon.filter(|icon| !icon.is_empty()),
        position: Position::between(last_sibling.as_ref(), None)?,
        row_of: None,
        is_archived: false,
    };
    let id = page.id;
    writer.apply(&Transaction::new(vec![Operation::Create(Record::Page(
        page,
    ))]))?;
    let block_ids = if arguments.blocks.is_empty() {
        Vec::new()
    } else {
        writer.edit_content(id, |doc| insert_blocks(doc, None, &arguments.blocks))?
    };
    Ok(json!({"page_id": id.0, "block_ids": block_ids}))
}

/// The position of the last page under `parent`, to place a new page after it as the app does.
fn last_child_position(
    store: &WorkspaceStore,
    parent: Option<PageId>,
) -> Result<Option<Position>, ToolError> {
    let mut last = None;
    loop {
        let after = last.as_ref().map(|page: &Page| page.id);
        let batch = store.page_children(parent, after, SIBLING_BATCH)?;
        let is_full = batch.len() == SIBLING_BATCH;
        if let Some(child) = batch.into_iter().last() {
            last = Some(child.page);
        }
        if !is_full {
            return Ok(last.map(|page| page.position));
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdatePage {
    page_id: PageId,
    title: Option<String>,
    icon: Option<String>,
    archived: Option<bool>,
}

pub(super) fn update_page(writer: &mut Writer, arguments: UpdatePage) -> Result<Value, ToolError> {
    let id = arguments.page_id;
    require_page(writer.store, id)?;
    let fields = [
        arguments.title.map(PageField::Title),
        arguments
            .icon
            .map(|icon| PageField::Icon(Some(icon).filter(|icon| !icon.is_empty()))),
        arguments.archived.map(PageField::IsArchived),
    ];
    let operations: Vec<Operation> = fields
        .into_iter()
        .flatten()
        .map(|field| Operation::Set(FieldChange::Page { id, field }))
        .collect();
    if operations.is_empty() {
        return Err(ToolError::NothingToChange);
    }
    writer.apply(&Transaction::new(operations))?;
    Ok(json!({"page_id": id.0}))
}
