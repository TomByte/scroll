//! Page content: reading a page's blocks and adding, changing and deleting them.
//!
//! Text is read and written plain. Marks (bold, links, …) are not returned; an update to a block's
//! text replaces only the part that differs from the stored text, so marks on the rest of the
//! block survive, and the new part is unformatted.

use scroll_core::id::PageId;
use scroll_core::page_doc::{BlockId, BlockKind, BlockPlacement, PageDoc};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ToolError, Writer};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlockInput {
    kind: Option<String>,
    #[serde(default)]
    text: String,
    checked: Option<bool>,
}

/// Every block in document order, nested ones after their parent with a greater `depth`.
pub(super) fn blocks_to_json(doc: &PageDoc) -> Result<Vec<Value>, ToolError> {
    Ok(doc
        .blocks()?
        .iter()
        .map(|block| {
            let mut found = json!({
                "id": block.id.to_string(),
                "kind": block.kind.name(),
                "text": block.text,
                "depth": block.depth,
            });
            if block.kind == BlockKind::ToDo {
                found["checked"] = json!(block.is_checked);
            }
            found
        })
        .collect())
}

/// Inserts `blocks` in order after `after`, or at the end of the page; returns their ids.
pub(super) fn insert_blocks(
    doc: &PageDoc,
    after: Option<BlockId>,
    blocks: &[BlockInput],
) -> Result<Vec<String>, ToolError> {
    let mut placement = after.map_or(BlockPlacement::LastChildOf(None), BlockPlacement::After);
    let mut ids = Vec::with_capacity(blocks.len());
    for block in blocks {
        let kind = block
            .kind
            .as_deref()
            .map_or(Ok(BlockKind::Paragraph), parse_kind)?;
        let id = doc.insert_block(placement, kind)?;
        if !block.text.is_empty() {
            doc.insert_text(id, 0, &block.text)?;
        }
        if block.checked == Some(true) {
            doc.toggle_to_do(id)?;
        }
        placement = BlockPlacement::After(id);
        ids.push(id.to_string());
    }
    Ok(ids)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AppendBlocks {
    page_id: PageId,
    blocks: Vec<BlockInput>,
    after_block_id: Option<String>,
}

pub(super) fn append_blocks(
    writer: &mut Writer,
    arguments: AppendBlocks,
) -> Result<Value, ToolError> {
    let after = arguments
        .after_block_id
        .as_deref()
        .map(parse_block_id)
        .transpose()?;
    let ids = writer.edit_content(arguments.page_id, |doc| {
        insert_blocks(doc, after, &arguments.blocks)
    })?;
    Ok(json!({"block_ids": ids}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateBlock {
    page_id: PageId,
    block_id: String,
    text: Option<String>,
    kind: Option<String>,
    checked: Option<bool>,
}

pub(super) fn update_block(
    writer: &mut Writer,
    arguments: UpdateBlock,
) -> Result<Value, ToolError> {
    let id = parse_block_id(&arguments.block_id)?;
    let kind = arguments.kind.as_deref().map(parse_kind).transpose()?;
    if kind.is_none() && arguments.text.is_none() && arguments.checked.is_none() {
        return Err(ToolError::NothingToChange);
    }
    writer.edit_content(arguments.page_id, |doc| {
        if let Some(kind) = kind {
            doc.set_block_kind(id, kind)?;
        }
        if let Some(text) = arguments.text.as_deref() {
            replace_text(doc, id, text)?;
        }
        if let Some(checked) = arguments.checked
            && doc.block(id)?.is_checked != checked
        {
            doc.toggle_to_do(id)?;
        }
        Ok(())
    })?;
    Ok(json!({"block_id": arguments.block_id}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteBlock {
    page_id: PageId,
    block_id: String,
}

pub(super) fn delete_block(
    writer: &mut Writer,
    arguments: DeleteBlock,
) -> Result<Value, ToolError> {
    let id = parse_block_id(&arguments.block_id)?;
    writer.edit_content(arguments.page_id, |doc| Ok(doc.delete_block(id)?))?;
    Ok(json!({"deleted": arguments.block_id}))
}

/// Replaces the part of the block's text between the common start and the common end of the old
/// and new text, so marks elsewhere in the block and concurrent edits nearby are kept.
fn replace_text(doc: &PageDoc, id: BlockId, new: &str) -> Result<(), ToolError> {
    let old = doc.block(id)?.text.clone();
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(old, new)| old == new)
        .map(|(old, _)| old.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(old, new)| old == new)
        .map(|(old, _)| old.len_utf8())
        .sum();
    let old_end = old.len() - suffix;
    if prefix < old_end {
        doc.delete_text(id, prefix..old_end)?;
    }
    let inserted = &new[prefix..new.len() - suffix];
    if !inserted.is_empty() {
        doc.insert_text(id, prefix, inserted)?;
    }
    Ok(())
}

fn parse_block_id(text: &str) -> Result<BlockId, ToolError> {
    BlockId::try_from(text).map_err(|_| ToolError::InvalidBlockId(text.to_string()))
}

fn parse_kind(name: &str) -> Result<BlockKind, ToolError> {
    BlockKind::ALL
        .into_iter()
        .find(|kind| kind.name() == name)
        .ok_or_else(|| ToolError::UnknownBlockKind(name.to_string()))
}

#[cfg(test)]
mod tests {
    use scroll_core::page_doc::{Mark, TextSpan};

    use super::*;

    fn block_with_bold_start(text: &str, bold: usize) -> (PageDoc, BlockId) {
        let doc = PageDoc::new();
        let id = doc
            .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
            .unwrap();
        doc.insert_text(id, 0, text).unwrap();
        doc.apply_mark(id, 0..bold, &Mark::Bold).unwrap();
        (doc, id)
    }

    #[test]
    fn replacing_text_changes_only_what_differs_and_keeps_marks_elsewhere() {
        let (doc, id) = block_with_bold_start("Ripe tomatoes and basil", 4);

        replace_text(&doc, id, "Ripe tomatoes, garlic and basil").unwrap();

        let block = doc.block(id).unwrap();
        assert_eq!(block.text, "Ripe tomatoes, garlic and basil");
        assert_eq!(
            block.spans[0],
            TextSpan {
                range: 0..4,
                marks: vec![Mark::Bold]
            }
        );
    }

    #[test]
    fn replacing_text_handles_characters_wider_than_a_byte() {
        let (doc, id) = block_with_bold_start("café ☕ olé", 2);
        replace_text(&doc, id, "café 🍵 olé").unwrap();
        assert_eq!(doc.block(id).unwrap().text, "café 🍵 olé");
        replace_text(&doc, id, "").unwrap();
        assert_eq!(doc.block(id).unwrap().text, "");
        replace_text(&doc, id, "olé olé").unwrap();
        assert_eq!(doc.block(id).unwrap().text, "olé olé");
    }

    #[test]
    fn blocks_are_inserted_in_order_with_their_kinds_and_checks() {
        let doc = PageDoc::new();
        let inputs: Vec<BlockInput> = serde_json::from_value(json!([
            {"kind": "heading_1", "text": "Plan"},
            {"kind": "to_do", "text": "Water", "checked": true},
            {"text": "Notes"},
        ]))
        .unwrap();

        insert_blocks(&doc, None, &inputs).unwrap();

        let blocks = blocks_to_json(&doc).unwrap();
        let summary: Vec<(&str, &str)> = blocks
            .iter()
            .map(|block| {
                (
                    block["kind"].as_str().unwrap(),
                    block["text"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("heading_1", "Plan"),
                ("to_do", "Water"),
                ("paragraph", "Notes")
            ]
        );
        assert_eq!(blocks[1]["checked"], json!(true));
        let wrong_kind: Vec<BlockInput> =
            serde_json::from_value(json!([{"kind": "title"}])).unwrap();
        assert!(matches!(
            insert_blocks(&doc, None, &wrong_kind),
            Err(ToolError::UnknownBlockKind(_))
        ));
    }
}
