//! Which cell is selected, and how the keyboard moves it.

/// Column 0 is the pinned title; columns 1 and up are the visible properties in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellPosition {
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movement {
    Up,
    Down,
    Left,
    Right,
    /// Tab: right, wrapping to the start of the next row.
    Next,
    /// Shift+Tab: left, wrapping to the end of the previous row.
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub rows: usize,
    pub columns: usize,
}

impl CellPosition {
    /// Stays inside the grid; at an edge the selection does not move.
    pub fn moved(self, movement: Movement, grid: GridSize) -> CellPosition {
        if grid.rows == 0 || grid.columns == 0 {
            return CellPosition::default();
        }
        let last_row = grid.rows - 1;
        let last_column = grid.columns - 1;
        let CellPosition { row, column } = self.clamped(grid);
        match movement {
            Movement::Up => CellPosition {
                row: row.saturating_sub(1),
                column,
            },
            Movement::Down => CellPosition {
                row: (row + 1).min(last_row),
                column,
            },
            Movement::Left => CellPosition {
                row,
                column: column.saturating_sub(1),
            },
            Movement::Right => CellPosition {
                row,
                column: (column + 1).min(last_column),
            },
            Movement::Next if column < last_column => CellPosition {
                row,
                column: column + 1,
            },
            Movement::Next if row < last_row => CellPosition {
                row: row + 1,
                column: 0,
            },
            Movement::Previous if column > 0 => CellPosition {
                row,
                column: column - 1,
            },
            Movement::Previous if row > 0 => CellPosition {
                row: row - 1,
                column: last_column,
            },
            Movement::Next | Movement::Previous => CellPosition { row, column },
        }
    }

    /// The nearest position inside the grid, for after rows or columns were removed.
    pub fn clamped(self, grid: GridSize) -> CellPosition {
        CellPosition {
            row: self.row.min(grid.rows.saturating_sub(1)),
            column: self.column.min(grid.columns.saturating_sub(1)),
        }
    }
}

/// A block of cells between the anchor (where the selection started) and the head (the cell the
/// keyboard moves). A single cell when the two are the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellRange {
    pub anchor: CellPosition,
    pub head: CellPosition,
}

impl CellRange {
    pub fn rows(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.row.min(self.head.row)..=self.anchor.row.max(self.head.row)
    }

    pub fn columns(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.column.min(self.head.column)..=self.anchor.column.max(self.head.column)
    }

    pub fn contains(&self, position: CellPosition) -> bool {
        self.rows().contains(&position.row) && self.columns().contains(&position.column)
    }

    pub fn is_single(&self) -> bool {
        self.anchor == self.head
    }

    /// The top-left cell, where a paste into the block starts.
    pub fn top_left(&self) -> CellPosition {
        CellPosition {
            row: *self.rows().start(),
            column: *self.columns().start(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: GridSize = GridSize {
        rows: 3,
        columns: 4,
    };

    fn at(row: usize, column: usize) -> CellPosition {
        CellPosition { row, column }
    }

    #[test]
    fn arrows_move_one_cell_and_stop_at_edges() {
        assert_eq!(at(1, 1).moved(Movement::Up, GRID), at(0, 1));
        assert_eq!(at(0, 1).moved(Movement::Up, GRID), at(0, 1));
        assert_eq!(at(2, 1).moved(Movement::Down, GRID), at(2, 1));
        assert_eq!(at(1, 0).moved(Movement::Left, GRID), at(1, 0));
        assert_eq!(at(1, 3).moved(Movement::Right, GRID), at(1, 3));
        assert_eq!(at(1, 2).moved(Movement::Right, GRID), at(1, 3));
    }

    #[test]
    fn tab_wraps_across_rows_both_ways() {
        assert_eq!(at(0, 3).moved(Movement::Next, GRID), at(1, 0));
        assert_eq!(at(2, 3).moved(Movement::Next, GRID), at(2, 3));
        assert_eq!(at(1, 0).moved(Movement::Previous, GRID), at(0, 3));
        assert_eq!(at(0, 0).moved(Movement::Previous, GRID), at(0, 0));
    }

    #[test]
    fn positions_outside_a_shrunken_grid_are_pulled_back() {
        let small = GridSize {
            rows: 1,
            columns: 2,
        };
        assert_eq!(at(5, 7).clamped(small), at(0, 1));
        assert_eq!(at(5, 7).moved(Movement::Up, small), at(0, 1));
        let empty = GridSize {
            rows: 0,
            columns: 2,
        };
        assert_eq!(at(3, 1).moved(Movement::Down, empty), at(0, 0));
    }

    #[test]
    fn a_range_covers_the_block_between_anchor_and_head_either_way_round() {
        let range = CellRange {
            anchor: at(2, 3),
            head: at(0, 1),
        };
        assert_eq!(range.rows(), 0..=2);
        assert_eq!(range.columns(), 1..=3);
        assert_eq!(range.top_left(), at(0, 1));
        assert!(range.contains(at(1, 2)) && !range.contains(at(1, 0)));
        assert!(!range.is_single());
        assert!(
            CellRange {
                anchor: at(1, 1),
                head: at(1, 1)
            }
            .is_single()
        );
    }
}
