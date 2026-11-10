use gpui::Styled;

/// One line of text that ends in "…" when it does not fit.
///
/// Use this instead of GPUI's `truncate()`: in GPUI 0.2.2 a no-wrap text keeps its first,
/// unconstrained measurement, so inside flex layouts the ellipsis never appears and the text is
/// clipped mid-letter. A one-line clamp is re-measured once its width is known.
pub trait Ellipsis: Styled + Sized {
    fn ellipsis(self) -> Self {
        self.overflow_hidden()
            .whitespace_normal()
            .text_ellipsis()
            .line_clamp(1)
    }
}

impl<T: Styled> Ellipsis for T {}
