use std::ops::Range;

/// A selection as the user made it: `anchor` stays put while `head` follows the caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: usize,
    pub head: usize,
}

impl TextSelection {
    pub fn caret(offset: usize) -> Self {
        TextSelection {
            anchor: offset,
            head: offset,
        }
    }

    /// A selection covering `range`, with the caret at its start when `is_reversed`.
    pub fn from_range(range: Range<usize>, is_reversed: bool) -> Self {
        if is_reversed {
            TextSelection {
                anchor: range.end,
                head: range.start,
            }
        } else {
            TextSelection {
                anchor: range.start,
                head: range.end,
            }
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    pub fn extended_to(self, offset: usize) -> Self {
        TextSelection {
            anchor: self.anchor,
            head: offset,
        }
    }

    /// Clamps both ends to `length`, for when the text shrank underneath the selection.
    pub fn clamped(self, length: usize) -> Self {
        TextSelection {
            anchor: self.anchor.min(length),
            head: self.head.min(length),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_ordered_regardless_of_direction() {
        let backwards = TextSelection { anchor: 7, head: 2 };
        assert_eq!(backwards.range(), 2..7);
        assert!(backwards.is_reversed());
        assert_eq!(TextSelection::from_range(2..7, true), backwards);
    }

    #[test]
    fn extending_keeps_the_anchor() {
        let selection = TextSelection::caret(4).extended_to(1).extended_to(9);
        assert_eq!(selection, TextSelection { anchor: 4, head: 9 });
    }

    #[test]
    fn clamping_pulls_both_ends_in() {
        let selection = TextSelection {
            anchor: 3,
            head: 12,
        }
        .clamped(5);
        assert_eq!(selection.range(), 3..5);
    }
}
