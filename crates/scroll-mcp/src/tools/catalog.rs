//! What `tools/list` tells the model: each tool's description and the JSON Schema of its
//! arguments. Keep these in step with the argument structs beside each tool.

use scroll_core::page_doc::BlockKind;
use serde_json::{Value, json};

use super::Tool;
use super::databases::Operator;
use crate::server::Access;

/// The tools offered with `access`: a read-only server lists none that write.
pub(crate) fn definitions(access: Access) -> Vec<Value> {
    Tool::ALL
        .into_iter()
        .filter(|tool| access == Access::ReadWrite || !tool.changes_workspace())
        .map(|tool| {
            json!({
                "name": tool.name(),
                "description": tool.description(),
                "inputSchema": tool.input_schema(),
                "annotations": {"readOnlyHint": !tool.changes_workspace()},
            })
        })
        .collect()
}

impl Tool {
    fn description(self) -> &'static str {
        match self {
            Tool::ListPages => {
                "Lists the pages directly under a page, or the workspace's top-level pages, in \
                 sidebar order. Database rows and archived pages are not listed; use \
                 query_database for rows. Pass next_after as after to get the next batch."
            }
            Tool::GetPage => {
                "Reads one page whole: title, icon, tags and its blocks in document order (nested \
                 blocks follow their parent with a greater depth; text is plain, without \
                 formatting). A page hosting a database also returns its properties and views; a \
                 database row returns its values by property name."
            }
            Tool::Search => {
                "Full-text search over page titles, page text and tag names, best match first. \
                 Every word must match, as a word prefix. Archived pages are left out; database \
                 rows are found too."
            }
            Tool::QueryDatabase => {
                "Returns rows of a database, filtered and sorted as one of its views (the first \
                 if none is named). A filter or sort given here replaces the view's own; all \
                 filter conditions must hold. Values are keyed by property name. Page through \
                 large results with offset; total is the number of rows matching."
            }
            Tool::CreatePage => {
                "Creates a page as the last child of parent_id (or at the top level), optionally \
                 with blocks of content. Returns the new page's id and its blocks' ids."
            }
            Tool::UpdatePage => {
                "Changes a page's title or icon (an emoji; an empty string removes it), or moves \
                 it to the trash (archived: true) and back (archived: false)."
            }
            Tool::AppendBlocks => {
                "Adds blocks to a page's content: at the end of the page, or after the block \
                 after_block_id (at its level). Returns the new blocks' ids."
            }
            Tool::UpdateBlock => {
                "Changes one block's text, kind or checked state (to_do blocks only). Only the \
                 part of the text that differs is replaced, so formatting on the rest of the \
                 block is kept; the new text itself is unformatted."
            }
            Tool::DeleteBlock => "Deletes a block and every block nested under it.",
            Tool::CreateRow => {
                "Adds a row to a database, after its last row, with a title and values by \
                 property name. Select options must already exist; they are matched by name."
            }
            Tool::UpdateRow => {
                "Changes a database row's title or values, by property name. A null value clears \
                 it. Properties not given are left as they are."
            }
        }
    }

    fn input_schema(self) -> Value {
        let page_id = json!({"type": "string", "description": "A page id (a UUID)."});
        let block_id = json!({
            "type": "string",
            "description": "A block id from get_page, such as \"12@3456789\"."
        });
        match self {
            Tool::ListPages => object(
                json!({
                    "parent_id": {
                        "type": "string",
                        "description": "The page whose children to list; leave out for the top level."
                    },
                    "after": {
                        "type": "string",
                        "description": "next_after from the previous call, to continue the list."
                    },
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100},
                }),
                &[],
            ),
            Tool::GetPage => object(json!({"page_id": page_id}), &["page_id"]),
            Tool::Search => object(
                json!({
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
                }),
                &["query"],
            ),
            Tool::QueryDatabase => object(
                json!({
                    "database_id": {
                        "type": "string",
                        "description": "A database id, from get_page on the page hosting it."
                    },
                    "view_id": {"type": "string"},
                    "filter": {"type": "array", "items": filter_schema()},
                    "sort": {
                        "type": "array",
                        "items": object(
                            json!({
                                "property": {"type": "string"},
                                "direction": {
                                    "enum": ["ascending", "descending"],
                                    "default": "ascending"
                                },
                            }),
                            &["property"],
                        ),
                    },
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "offset": {"type": "integer", "minimum": 0, "default": 0},
                }),
                &["database_id"],
            ),
            Tool::CreatePage => object(
                json!({
                    "parent_id": {
                        "type": "string",
                        "description": "The page to create it under; leave out for the top level."
                    },
                    "title": {"type": "string"},
                    "icon": {"type": "string", "description": "An emoji."},
                    "blocks": blocks_schema(),
                }),
                &["title"],
            ),
            Tool::UpdatePage => object(
                json!({
                    "page_id": page_id,
                    "title": {"type": "string"},
                    "icon": {"type": "string", "description": "An emoji; empty removes it."},
                    "archived": {"type": "boolean"},
                }),
                &["page_id"],
            ),
            Tool::AppendBlocks => object(
                json!({
                    "page_id": page_id,
                    "blocks": blocks_schema(),
                    "after_block_id": block_id,
                }),
                &["page_id", "blocks"],
            ),
            Tool::UpdateBlock => object(
                json!({
                    "page_id": page_id,
                    "block_id": block_id,
                    "text": {"type": "string"},
                    "kind": kind_schema(),
                    "checked": {"type": "boolean"},
                }),
                &["page_id", "block_id"],
            ),
            Tool::DeleteBlock => object(
                json!({"page_id": page_id, "block_id": block_id}),
                &["page_id", "block_id"],
            ),
            Tool::CreateRow => object(
                json!({
                    "database_id": {"type": "string"},
                    "title": {"type": "string"},
                    "values": values_schema(),
                }),
                &["database_id"],
            ),
            Tool::UpdateRow => object(
                json!({
                    "page_id": {"type": "string", "description": "The row's page id."},
                    "title": {"type": "string"},
                    "values": values_schema(),
                }),
                &["page_id"],
            ),
        }
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn kind_schema() -> Value {
    let kinds: Vec<&str> = BlockKind::ALL.into_iter().map(BlockKind::name).collect();
    json!({"enum": kinds, "default": "paragraph"})
}

fn blocks_schema() -> Value {
    json!({
        "type": "array",
        "items": object(
            json!({
                "kind": kind_schema(),
                "text": {"type": "string", "default": ""},
                "checked": {"type": "boolean", "description": "For to_do blocks."},
            }),
            &[],
        ),
    })
}

fn values_schema() -> Value {
    json!({
        "type": "object",
        "description": "Values by property name: text and url take a string, number a number, \
                        checkbox true or false, date \"YYYY-MM-DD\", select an option name, \
                        multi_select a list of option names; null clears a value.",
        "additionalProperties": true,
    })
}

fn filter_schema() -> Value {
    let operators: Vec<&str> = Operator::ALL.into_iter().map(Operator::name).collect();
    object(
        json!({
            "property": {"type": "string", "description": "A property name."},
            "operator": {
                "enum": operators,
                "description": "is_empty and is_not_empty work on every type and take no value; \
                                equals on text, url, number, select, checkbox and date; contains \
                                on text, url and multi_select (one option); greater_than and \
                                less_than on number; before and after on date."
            },
            "value": {"description": "Written as for that property's values."},
        }),
        &["property", "operator"],
    )
}
