//! What a brand-new workspace starts with: a welcome page that shows every kind of block,
//! and a few pages to show nesting.

use std::sync::Arc;

use crate::workspace::records::WorkspaceRecords;
use crate::workspace::storage::{self, Storage};
use scroll_core::id::PageId;
use scroll_core::operation::Transaction;

use crate::editor::rich_text::MarkKind;
use crate::workspace::page_tree::PageTree;
use scroll_core::position::PositionError;

use crate::editor::document::{BlockId, BlockKind, PageDocument, PageError, PageResult, Placement};

/// A page to create, and the Loro updates holding its first content.
pub struct SeedPage {
    pub transaction: Transaction,
    pub page: PageId,
    pub content: Vec<Vec<u8>>,
}

/// One block of seed content: kind, nesting depth and text. Text between `**` is bold.
type SeedBlock = (BlockKind, usize, &'static str);

const WELCOME: &[SeedBlock] = &[
    (
        BlockKind::Paragraph,
        0,
        "Welcome to Scroll. Everything here is a block: click anywhere and start typing.",
    ),
    (BlockKind::Heading2, 0, "The basics"),
    (BlockKind::ToDo, 0, "Type **/** for a menu of block types"),
    (
        BlockKind::ToDo,
        0,
        "Start a line with **#**, **-**, **1.**, **[]**, **>** or three backticks",
    ),
    (BlockKind::ToDo, 0, "Open this page"),
    (BlockKind::Heading1, 0, "Shortcuts"),
    (
        BlockKind::BulletedListItem,
        0,
        "**⌘P** searches pages and runs commands",
    ),
    (BlockKind::Heading3, 0, "Formatting"),
    (
        BlockKind::BulletedListItem,
        0,
        "**⌘B**, **⌘I**, **⌘E** and **⇧⌘X** for bold, italic, code and strikethrough",
    ),
    (
        BlockKind::BulletedListItem,
        1,
        "Select text first, or toggle and keep typing",
    ),
    (
        BlockKind::BulletedListItem,
        0,
        "**Tab** and **⇧Tab** nest and un-nest blocks",
    ),
    (
        BlockKind::NumberedListItem,
        0,
        "Press **Esc** to select a block, then **⇧⌘↑** or **⇧⌘↓** to move it",
    ),
    (
        BlockKind::NumberedListItem,
        0,
        "Or drag the handle that appears to the left of a block",
    ),
    (
        BlockKind::NumberedListItem,
        0,
        "Go back and forward with **⌘[** and **⌘]**",
    ),
    (
        BlockKind::Quote,
        0,
        "Simple things should be simple, complex things should be possible.",
    ),
    (
        BlockKind::Callout,
        0,
        "Your workspace lives in one file on this Mac.",
    ),
    (
        BlockKind::Code,
        0,
        "fn main() {\n    println!(\"hello, scroll\");\n}",
    ),
    (BlockKind::Divider, 0, ""),
    (
        BlockKind::Paragraph,
        0,
        "Pages hold pages too: open Projects in the sidebar.",
    ),
];

const ROADMAP: &[SeedBlock] = &[
    (BlockKind::Heading2, 0, "Now"),
    (BlockKind::ToDo, 0, "Ship the local app"),
    (BlockKind::Heading2, 0, "Next"),
    (BlockKind::ToDo, 0, "Databases and table views"),
    (BlockKind::ToDo, 0, "Sync with a Scroll server"),
];

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error(transparent)]
    Page(#[from] PageError),
    #[error(transparent)]
    Position(#[from] PositionError),
    #[error(transparent)]
    Tree(#[from] crate::workspace::page_tree::MoveError),
}

pub fn welcome_pages() -> Result<Vec<SeedPage>, SeedError> {
    let mut seeds = Seeds::default();
    seeds.add(None, "Getting started", WELCOME)?;
    let projects = seeds.add(
        None,
        "Projects",
        &[(BlockKind::Paragraph, 0, "Each project gets its own page.")],
    )?;
    seeds.add(Some(projects), "Roadmap", ROADMAP)?;
    seeds.add(
        Some(projects),
        "Design notes",
        &[(BlockKind::Paragraph, 0, "Calm, precise, fast.")],
    )?;
    seeds.add(None, "Reading list", &[(BlockKind::ToDo, 0, "")])?;
    Ok(seeds.pages)
}

#[derive(Default)]
struct Seeds {
    tree: PageTree,
    pages: Vec<SeedPage>,
}

impl Seeds {
    fn add(
        &mut self,
        parent: Option<PageId>,
        title: &str,
        content: &[SeedBlock],
    ) -> Result<PageId, SeedError> {
        let (page, transaction) = self.tree.create_page(parent, title)?;
        self.tree.apply(&transaction);
        self.pages.push(SeedPage {
            transaction,
            page,
            content: seed_content(content)?,
        });
        Ok(page)
    }
}

fn seed_content(blocks: &[SeedBlock]) -> PageResult<Vec<Vec<u8>>> {
    let mut page = PageDocument::empty();
    let mut parents: Vec<BlockId> = Vec::new();
    for &(kind, depth, text) in blocks {
        parents.truncate(depth);
        let block = page.insert_block(Placement::LastChildOf(parents.last().copied()), kind)?;
        write_with_bold(&page, block, text)?;
        if text == "Open this page" {
            page.set_checked(block, true)?;
        }
        parents.push(block);
    }
    page.ensure_block()?;
    page.commit();
    Ok(page.take_outgoing_updates())
}

/// Writes `text`, turning `**…**` spans into bold. The whole text goes in first, so bold
/// (which grows at its end) cannot spread onto the text after it.
fn write_with_bold(page: &PageDocument, block: BlockId, text: &str) -> PageResult<()> {
    let plain: String = text.split("**").collect();
    page.insert_text(block, 0, &plain)?;
    let mut offset = 0;
    for (index, piece) in text.split("**").enumerate() {
        let is_bold = index % 2 == 1;
        if is_bold && !piece.is_empty() {
            page.add_mark(block, offset..offset + piece.len(), MarkKind::Bold)?;
        }
        offset += piece.len();
    }
    Ok(())
}

/// Writes the welcome pages and returns the records they make.
pub fn seed(storage: &Arc<Storage>) -> WorkspaceRecords {
    let seeds = match welcome_pages() {
        Ok(seeds) => seeds,
        Err(error) => {
            log::error!("could not build the welcome pages: {error}");
            return WorkspaceRecords::default();
        }
    };
    let mut records = WorkspaceRecords::default();
    for seed in seeds {
        records.apply(&seed.transaction);
        let page = seed.page;
        storage.write("create a welcome page", move |store| {
            storage::apply(store, &seed.transaction)?;
            storage::record_updates(store, page, &seed.content)?;
            storage::compact(store, page)
        });
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_welcome_page_has_every_block_kind() {
        let pages = welcome_pages().unwrap();
        assert_eq!(pages.len(), 5);
        let content: Vec<&[u8]> = pages[0].content.iter().map(Vec::as_slice).collect();
        let welcome = PageDocument::load(None, content).unwrap();
        let kinds: std::collections::HashSet<BlockKind> =
            welcome.blocks().iter().map(|block| block.kind).collect();
        assert_eq!(kinds.len(), 11);
        let bold_line = welcome.block_content(welcome.block_ids()[2]).unwrap();
        assert_eq!(bold_line.text, "Type / for a menu of block types");
        assert!(bold_line.is_marked_throughout(&(5..6), MarkKind::Bold));
        assert!(
            !bold_line.is_marked_throughout(&(6..10), MarkKind::Bold),
            "bold stops at its closing marks"
        );
    }
}
