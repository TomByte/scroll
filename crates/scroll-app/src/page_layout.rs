//! Geometry every kind of page shares, so documents and databases line up, and how wide a
//! document runs.

use gpui::{Pixels, px};
use scroll_settings::{AnySetting, Category, Choice, Setting};

/// The space either side of a page's content, and above its header.
pub const PAGE_MARGIN: Pixels = px(40.0);
/// The widest a document's text runs while its width is constrained.
pub const CONTENT_MAX_WIDTH: Pixels = px(720.0);

/// One choice for every document on this Mac: a width per page would be page data, synced
/// like a title, which is out of proportion for a viewing preference.
pub const PAGE_WIDTH: Setting<PageWidth> = Setting {
    key: "appearance.page_width",
    category: Category::Appearance,
    label: "Page width",
    description: "Full width uses the whole window for documents; databases always do.",
    default: PageWidth::Constrained,
};

pub static SETTINGS: &[&dyn AnySetting] = &[&PAGE_WIDTH];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageWidth {
    Constrained,
    Full,
}

impl PageWidth {
    pub fn toggled(self) -> PageWidth {
        match self {
            PageWidth::Constrained => PageWidth::Full,
            PageWidth::Full => PageWidth::Constrained,
        }
    }

    /// The widest a document's column runs, margins included; `None` fills the window.
    pub fn document_max_width(self) -> Option<Pixels> {
        match self {
            PageWidth::Constrained => Some(CONTENT_MAX_WIDTH + PAGE_MARGIN * 2.0),
            PageWidth::Full => None,
        }
    }
}

impl Choice for PageWidth {
    const OPTIONS: &'static [Self] = &[PageWidth::Constrained, PageWidth::Full];

    fn id(self) -> &'static str {
        match self {
            PageWidth::Constrained => "constrained",
            PageWidth::Full => "full",
        }
    }

    fn label(self) -> &'static str {
        match self {
            PageWidth::Constrained => "Constrained",
            PageWidth::Full => "Full",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constrained_document_keeps_its_text_column_and_a_full_one_fills_the_window() {
        assert_eq!(
            PageWidth::Constrained.document_max_width(),
            Some(CONTENT_MAX_WIDTH + PAGE_MARGIN * 2.0)
        );
        assert_eq!(PageWidth::Full.document_max_width(), None);
        assert_eq!(PageWidth::Constrained.toggled(), PageWidth::Full);
        assert_eq!(PageWidth::Full.toggled(), PageWidth::Constrained);
    }
}
