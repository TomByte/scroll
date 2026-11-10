//! Drawn diagrams kept for the whole run, so a page opened again shows its diagrams in its
//! first frame instead of drawing them again. Every editor shares one cache, keyed by what a
//! diagram is drawn from.
//!
//! The cache owns the images: an editor only borrows one (a clone of its `Arc`) while it shows
//! it. Past `MAX_CACHED_BYTES` the least recently used images that no editor shows are freed
//! from every window. One that is still shown is never freed: painting it again would upload
//! it again, and then nothing would free it.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Global, Pixels, RenderImage, Size};

use super::DiagramRequest;
use crate::editor::document::BlockId;
use crate::editor::mermaid::DrawnDiagram;

/// About a dozen typical diagrams drawn at twice their size.
const MAX_CACHED_BYTES: usize = 48 * 1024 * 1024;

#[derive(Default)]
pub(super) struct DiagramCache {
    /// Least recently used first.
    entries: Vec<(DiagramRequest, DrawnDiagram)>,
    /// The size each diagram block was last shown at, to hold its place while it is drawn
    /// again (after a theme change, say).
    sizes: HashMap<BlockId, Size<Pixels>>,
}

impl Global for DiagramCache {}

impl DiagramCache {
    fn get(&mut self, request: &DiagramRequest) -> Option<DrawnDiagram> {
        let index = self.position(request)?;
        let entry = self.entries.remove(index);
        let drawn = entry.1.clone();
        self.entries.push(entry);
        Some(drawn)
    }

    /// Keeps `drawn` and returns the drawing to show for `request`: the one already kept if
    /// another editor drew the same thing first, so only one copy is ever uploaded.
    fn insert(&mut self, request: DiagramRequest, drawn: DrawnDiagram) -> DrawnDiagram {
        if let Some(kept) = self.get(&request) {
            return kept;
        }
        self.entries.push((request, drawn.clone()));
        drawn
    }

    /// Takes out the least recently used images no editor shows until the rest fit the
    /// budget, and returns them to be freed.
    fn evict_unused(&mut self) -> Vec<Arc<RenderImage>> {
        let mut bytes: usize = self
            .entries
            .iter()
            .map(|(_, drawn)| image_bytes(drawn))
            .sum();
        let mut evicted = Vec::new();
        let mut index = 0;
        while bytes > MAX_CACHED_BYTES && index < self.entries.len() {
            if is_shown(&self.entries[index].1) {
                index += 1;
                continue;
            }
            let (_, drawn) = self.entries.remove(index);
            bytes -= image_bytes(&drawn);
            evicted.push(drawn.image);
        }
        evicted
    }

    fn position(&self, request: &DiagramRequest) -> Option<usize> {
        self.entries.iter().position(|(kept, _)| kept == request)
    }
}

/// The drawing kept for `request`, if any.
pub(super) fn cached(request: &DiagramRequest, cx: &mut App) -> Option<DrawnDiagram> {
    cx.try_global::<DiagramCache>()?;
    cx.global_mut::<DiagramCache>().get(request)
}

/// Keeps a new drawing (see `DiagramCache::insert`), freeing old ones past the budget.
pub(super) fn keep(request: DiagramRequest, drawn: DrawnDiagram, cx: &mut App) -> DrawnDiagram {
    let kept = cx.default_global::<DiagramCache>().insert(request, drawn);
    free_unused(cx);
    kept
}

/// Frees the images no editor shows any more, oldest first, until the cache fits its budget.
pub(super) fn free_unused(cx: &mut App) {
    let Some(cache) = cx.try_global::<DiagramCache>() else {
        return;
    };
    if cache.entries.is_empty() {
        return;
    }
    for image in cx.global_mut::<DiagramCache>().evict_unused() {
        cx.drop_image(image, None);
    }
}

pub(super) fn remember_size(block: BlockId, size: Size<Pixels>, cx: &mut App) {
    cx.default_global::<DiagramCache>()
        .sizes
        .insert(block, size);
}

/// The size `block`'s diagram was last shown at in this run.
pub(super) fn last_size(block: BlockId, cx: &App) -> Option<Size<Pixels>> {
    cx.try_global::<DiagramCache>()?.sizes.get(&block).copied()
}

/// Whether an editor holds the image besides the cache.
fn is_shown(drawn: &DrawnDiagram) -> bool {
    Arc::strong_count(&drawn.image) > 1
}

fn image_bytes(drawn: &DrawnDiagram) -> usize {
    let pixels = drawn.image.size(0);
    pixels.width.0.max(0) as usize * pixels.height.0.max(0) as usize * 4
}

#[cfg(test)]
mod tests {
    use gpui::{px, size};

    use super::*;
    use crate::editor::mermaid::DiagramStyle;

    /// A drawing of `side` by `side` pixels.
    fn drawing(side: u32) -> DrawnDiagram {
        let buffer = image::RgbaImage::new(side, side);
        DrawnDiagram {
            image: Arc::new(RenderImage::new([image::Frame::new(buffer)])),
            size: size(px(side as f32 / 2.0), px(side as f32 / 2.0)),
        }
    }

    fn request(source: &str) -> DiagramRequest {
        DiagramRequest {
            source: source.to_string(),
            style: DiagramStyle::for_tests(),
        }
    }

    /// A drawing a quarter of the budget.
    fn quarter() -> DrawnDiagram {
        let side = ((MAX_CACHED_BYTES / 4 / 4) as f64).sqrt() as u32;
        drawing(side)
    }

    #[test]
    fn the_same_drawing_is_kept_once_and_found_again() {
        let mut cache = DiagramCache::default();
        let first = cache.insert(request("a"), drawing(4));
        let second = cache.insert(request("a"), drawing(4));
        assert!(
            Arc::ptr_eq(&first.image, &second.image),
            "the first is kept"
        );
        assert_eq!(cache.entries.len(), 1);
        let found = cache.get(&request("a")).expect("found");
        assert!(Arc::ptr_eq(&found.image, &first.image));
        assert!(cache.get(&request("b")).is_none());
    }

    #[test]
    fn past_the_budget_the_least_recently_used_unshown_drawings_go() {
        let mut cache = DiagramCache::default();
        let names = ["a", "b", "c", "d", "e", "f"];
        for name in names {
            cache.insert(request(name), quarter());
        }
        // "a" was looked at again, and an editor shows "b".
        cache.get(&request("a"));
        let shown = cache.get(&request("b"));

        let evicted = cache.evict_unused();
        let kept: Vec<&str> = cache
            .entries
            .iter()
            .map(|(request, _)| request.source.as_str())
            .collect();
        assert_eq!(evicted.len(), 2, "two over the budget");
        assert_eq!(kept, ["e", "f", "a", "b"]);
        assert!(
            evicted.iter().all(|image| Arc::strong_count(image) == 1),
            "nothing else holds what is freed"
        );
        drop(shown);
    }

    #[test]
    fn drawings_still_shown_are_kept_even_over_the_budget() {
        let mut cache = DiagramCache::default();
        let shown: Vec<DrawnDiagram> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(|name| cache.insert(request(name), quarter()))
            .collect();
        assert!(cache.evict_unused().is_empty());
        assert_eq!(cache.entries.len(), 5);

        drop(shown);
        assert_eq!(
            cache.evict_unused().len(),
            1,
            "once let go, the oldest goes"
        );
    }
}
