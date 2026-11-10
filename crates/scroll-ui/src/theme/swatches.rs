//! The swatches: a fixed set of colours for things people colour themselves, such as tags and
//! select options. The kit knows them only by index; what each is called is the app's.

use gpui::{Hsla, hsla};

use super::Theme;

/// A swatch's base hue (degrees), saturation and lightness.
type Swatch = (f32, f32, f32);

/// Every swatch. A swatch is an index into this list; indexes wrap around.
const SWATCHES: [Swatch; SWATCH_COUNT] = [
    (220.0, 0.08, 0.55),
    (25.0, 0.35, 0.45),
    (28.0, 0.85, 0.55),
    (45.0, 0.90, 0.50),
    (140.0, 0.50, 0.42),
    (215.0, 0.75, 0.55),
    (265.0, 0.55, 0.60),
    (330.0, 0.65, 0.60),
    (0.0, 0.70, 0.55),
];

pub const SWATCH_COUNT: usize = 9;

/// A pill's background, as a fraction of its swatch's opacity.
const SWATCH_FILL_OPACITY: f32 = 0.16;

impl Theme {
    /// Swatch `index` at full strength, e.g. for a dot beside a tag's name.
    pub fn swatch(&self, index: usize) -> Hsla {
        let (hue, saturation, lightness) = SWATCHES[index % SWATCH_COUNT];
        hsla(hue / 360.0, saturation, lightness, 1.0)
    }

    /// Background and text colours of a pill in swatch `index`.
    pub(crate) fn swatch_fill(&self, index: usize) -> (Hsla, Hsla) {
        (
            self.swatch(index).opacity(SWATCH_FILL_OPACITY),
            self.colors.text,
        )
    }
}
