//! Text offsets in UTF-8, UTF-16 and graphemes, and a text selection, for any text editor.

mod offsets;
mod selection;

pub use offsets::*;
pub use selection::TextSelection;
