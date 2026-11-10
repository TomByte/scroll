//! The emoji table with search, and the emoji picker.

mod data;
pub(crate) mod picker;

pub use data::{Emoji, emoji_named, remember_emoji, search_emoji};
pub(crate) use data::{all_emoji, recent_emoji};
