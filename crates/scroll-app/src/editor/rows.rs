//! Which blocks each row of the page's list shows. Most blocks are a row each. A table is one
//! row holding the table block and everything nested under it (its rows and their cells), since
//! the editor draws those as one grid.

use std::ops::Range;

use crate::editor::document::{BlockInfo, BlockKind};

#[derive(Default)]
pub(super) struct Rows {
    /// The blocks each row shows, as indices in document order.
    spans: Vec<Range<usize>>,
    row_of_block: Vec<usize>,
}

impl Rows {
    pub(super) fn of(blocks: &[BlockInfo]) -> Rows {
        let mut rows = Rows {
            spans: Vec::with_capacity(blocks.len()),
            row_of_block: Vec::with_capacity(blocks.len()),
        };
        // The depth of the table the walk is inside, if any.
        let mut table_depth: Option<usize> = None;
        for (index, block) in blocks.iter().enumerate() {
            let is_in_table = table_depth.is_some_and(|depth| block.depth > depth);
            match rows.spans.last_mut() {
                Some(span) if is_in_table => span.end = index + 1,
                _ => {
                    rows.spans.push(index..index + 1);
                    table_depth = (block.kind == BlockKind::Table).then_some(block.depth);
                }
            }
            rows.row_of_block.push(rows.spans.len() - 1);
        }
        rows
    }

    pub(super) fn count(&self) -> usize {
        self.spans.len()
    }

    /// The blocks row `row` shows; the first is the block the row is drawn for.
    pub(super) fn span(&self, row: usize) -> Range<usize> {
        self.spans[row].clone()
    }

    pub(super) fn row_of(&self, block: usize) -> usize {
        self.row_of_block[block]
    }

    /// What each row shows, to compare with another `Rows` of `blocks` as they were or will be.
    pub(super) fn contents<'a>(&self, blocks: &'a [BlockInfo]) -> Vec<&'a [BlockInfo]> {
        self.spans
            .iter()
            .map(|span| &blocks[span.clone()])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::{PageDocument, Placement};

    /// A page of blocks of `kinds` at the given depths, each nested under the block above it
    /// one level up.
    fn infos(kinds: &[(BlockKind, usize)]) -> Vec<BlockInfo> {
        let mut page = PageDocument::create(1, &[]).unwrap();
        let mut parents = Vec::new();
        for &(kind, depth) in kinds {
            parents.truncate(depth);
            let block = page
                .insert_block(Placement::LastChildOf(parents.last().copied()), kind)
                .unwrap();
            parents.push(block);
        }
        page.commit();
        page.blocks().to_vec()
    }

    #[test]
    fn a_table_is_one_row_with_everything_under_it() {
        use BlockKind::*;
        let blocks = infos(&[
            (Paragraph, 0),
            (Table, 0),
            (TableRow, 1),
            (TableCell, 2),
            (TableCell, 2),
            (TableRow, 1),
            (TableCell, 2),
            (Paragraph, 0),
            (Paragraph, 1),
        ]);
        let rows = Rows::of(&blocks);
        assert_eq!(rows.count(), 4);
        assert_eq!(rows.span(1), 1..7);
        assert_eq!(rows.span(2), 7..8);
        assert_eq!(rows.row_of(4), 1, "a cell is shown in its table's row");
        assert_eq!(
            rows.row_of(8),
            3,
            "an ordinary nested block has its own row"
        );
    }

    #[test]
    fn tables_next_to_each_other_stay_separate_rows() {
        use BlockKind::*;
        let blocks = infos(&[(Table, 0), (TableRow, 1), (Table, 0), (TableRow, 1)]);
        let rows = Rows::of(&blocks);
        assert_eq!((rows.span(0), rows.span(1)), (0..2, 2..4));
    }

    #[test]
    fn an_empty_page_has_no_rows() {
        assert_eq!(Rows::of(&[]).count(), 0);
    }
}
