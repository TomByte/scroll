//! The tag colours: what each stored colour is called, which kit swatch draws it, and the
//! order new tags take them in.

use scroll_core::record::TagColor;
use scroll_ui::theme::SWATCH_COUNT;

/// Every tag colour with its name, in the order menus list them. A colour's position here is
/// the kit swatch it is drawn with.
pub const TAG_COLORS: [(TagColor, &str); SWATCH_COUNT] = [
    (TagColor::Gray, "Grey"),
    (TagColor::Brown, "Brown"),
    (TagColor::Orange, "Orange"),
    (TagColor::Yellow, "Yellow"),
    (TagColor::Green, "Green"),
    (TagColor::Blue, "Blue"),
    (TagColor::Purple, "Purple"),
    (TagColor::Pink, "Pink"),
    (TagColor::Red, "Red"),
];

/// The colours new tags take in turn, so neighbours differ.
const NEW_TAG_ORDER: [TagColor; SWATCH_COUNT] = [
    TagColor::Blue,
    TagColor::Green,
    TagColor::Orange,
    TagColor::Purple,
    TagColor::Pink,
    TagColor::Yellow,
    TagColor::Red,
    TagColor::Brown,
    TagColor::Gray,
];

/// The kit swatch a stored tag colour is drawn with.
pub fn swatch_of(color: TagColor) -> usize {
    // Every colour is listed; `every_tag_color_has_a_swatch` checks it.
    TAG_COLORS
        .iter()
        .position(|(listed, _)| *listed == color)
        .unwrap_or(0)
}

pub fn next_tag_color(existing_tags: usize) -> TagColor {
    NEW_TAG_ORDER[existing_tags % NEW_TAG_ORDER.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tag_color_has_a_swatch() {
        for (index, (color, _)) in TAG_COLORS.into_iter().enumerate() {
            assert_eq!(swatch_of(color), index);
        }
    }

    #[test]
    fn new_tags_use_every_colour_once_before_repeating() {
        let mut order: Vec<usize> = NEW_TAG_ORDER.into_iter().map(swatch_of).collect();
        order.sort_unstable();
        assert_eq!(order, (0..SWATCH_COUNT).collect::<Vec<_>>());
    }
}
