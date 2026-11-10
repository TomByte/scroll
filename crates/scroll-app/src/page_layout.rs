//! Geometry every kind of page shares, so documents and databases line up.

use gpui::{Pixels, px};

/// The space either side of a page's content, and above its header.
pub const PAGE_MARGIN: Pixels = px(40.0);
