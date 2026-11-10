//! Drawing Mermaid source as an image, without a browser: `mermaid-rs-renderer` parses it and
//! lays it out as SVG, and resvg (the version gpui draws its icons with) turns that into pixels
//! gpui can show. Both measure and draw labels with the same system fonts, so text fits its
//! boxes. Drawing takes a few milliseconds for a typical diagram, so it runs off the main
//! thread (`diagram_block`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, LazyLock};

use gpui::{Hsla, Pixels, RenderImage, Size, hsla, px, size};
use mermaid_rs_renderer::{ParseError, RenderOptions, render_strict};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use scroll_ui::Theme;
use scroll_ui::theme::Appearance;

/// Pixels per point the diagram is drawn at, so it stays sharp on a Retina display.
const DRAWING_SCALE: f32 = 2.0;
/// The longest side of a drawn diagram, in pixels; a larger one is drawn at a lower scale.
/// A diagram this wide is shown scaled down to the page column anyway.
const MAX_DRAWN_SIDE: f32 = 4096.0;
const LABEL_SIZE: f32 = 14.0;

/// What a new diagram block starts with, so it shows a diagram at once.
pub const EXAMPLE_DIAGRAM: &str =
    "flowchart LR\n    A[Idea] --> B{Ready?}\n    B -->|Yes| C[Publish]\n    B -->|No| A";

/// System fonts, loaded once, the first time a diagram is drawn.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    Arc::new(fonts)
});

#[derive(Debug, thiserror::Error)]
pub enum DiagramError {
    #[error("Nothing to draw yet")]
    Empty,
    #[error(transparent)]
    Syntax(#[from] ParseError),
    #[error("The diagram could not be drawn: {0}")]
    Svg(#[from] usvg::Error),
    #[error("The diagram could not be drawn")]
    RendererFailed,
}

/// The colours a diagram is drawn in, taken from the app's theme so it sits on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiagramStyle {
    background: Hsla,
    node_fill: Hsla,
    node_border: Hsla,
    group_fill: Hsla,
    line: Hsla,
    text: Hsla,
    /// Hue, saturation and lightness of pie slices and other series; the hue turns per slice.
    series: Hsla,
}

impl DiagramStyle {
    pub fn from_theme(theme: &Theme) -> DiagramStyle {
        let colors = &theme.colors;
        let page = colors.surface;
        let series_lightness = match theme.appearance {
            Appearance::Light => 0.72,
            Appearance::Dark => 0.36,
        };
        DiagramStyle {
            background: page,
            node_fill: page.blend(colors.accent.opacity(0.1)),
            node_border: page.blend(colors.accent.opacity(0.6)),
            group_fill: page.blend(colors.code_background),
            line: colors.text_muted,
            text: colors.text,
            series: hsla(colors.accent.h, 0.55, series_lightness, 1.0),
        }
    }

    /// Light-page colours, for tests that need a style without a window.
    #[cfg(test)]
    pub fn for_tests() -> DiagramStyle {
        DiagramStyle {
            background: hsla(0.0, 0.0, 1.0, 1.0),
            node_fill: hsla(0.57, 0.7, 0.95, 1.0),
            node_border: hsla(0.57, 0.7, 0.5, 1.0),
            group_fill: hsla(0.0, 0.0, 0.95, 1.0),
            line: hsla(0.58, 0.1, 0.4, 1.0),
            text: hsla(0.58, 0.3, 0.14, 1.0),
            series: hsla(0.57, 0.55, 0.72, 1.0),
        }
    }

    fn mermaid_theme(&self) -> mermaid_rs_renderer::Theme {
        let background = hex(self.background);
        let node_fill = hex(self.node_fill);
        let node_border = hex(self.node_border);
        let group_fill = hex(self.group_fill);
        let line = hex(self.line);
        let text = hex(self.text);
        mermaid_rs_renderer::Theme {
            font_size: LABEL_SIZE,
            primary_color: node_fill.clone(),
            primary_text_color: text.clone(),
            primary_border_color: node_border.clone(),
            line_color: line.clone(),
            secondary_color: group_fill.clone(),
            tertiary_color: background.clone(),
            edge_label_background: background.clone(),
            cluster_background: group_fill.clone(),
            cluster_border: node_border.clone(),
            background: background.clone(),
            sequence_actor_fill: node_fill.clone(),
            sequence_actor_border: node_border.clone(),
            sequence_actor_line: line.clone(),
            sequence_note_fill: group_fill.clone(),
            sequence_note_border: node_border.clone(),
            sequence_activation_fill: group_fill,
            sequence_activation_border: node_border.clone(),
            text_color: text.clone(),
            pie_colors: std::array::from_fn(|slice| hex(self.series_color(slice))),
            pie_title_text_color: text.clone(),
            pie_section_text_color: text.clone(),
            pie_legend_text_color: text,
            pie_stroke_color: background,
            pie_outer_stroke_color: node_border,
            pie_opacity: 1.0,
            ..mermaid_rs_renderer::Theme::modern()
        }
    }

    /// Slices step round the colour wheel from the accent, five twelfths at a time, so
    /// neighbours differ clearly.
    fn series_color(&self, index: usize) -> Hsla {
        let hue = (self.series.h + index as f32 * 5.0 / 12.0).fract();
        Hsla {
            h: hue,
            ..self.series
        }
    }
}

/// A drawn diagram and the size it is shown at before any scaling to fit the page.
#[derive(Clone)]
pub struct DrawnDiagram {
    pub image: Arc<RenderImage>,
    pub size: Size<Pixels>,
}

/// Draws `source` in `style`. Slow enough that it belongs off the main thread.
pub fn draw(source: &str, style: &DiagramStyle) -> Result<DrawnDiagram, DiagramError> {
    let svg = diagram_svg(source, style)?;
    let (pixmap, size) = rasterize(&svg)?;
    Ok(DrawnDiagram {
        image: Arc::new(RenderImage::new([image_frame(pixmap)])),
        size,
    })
}

fn diagram_svg(source: &str, style: &DiagramStyle) -> Result<String, DiagramError> {
    if source.trim().is_empty() {
        return Err(DiagramError::Empty);
    }
    let options = RenderOptions {
        theme: style.mermaid_theme(),
        ..RenderOptions::modern()
    };
    // A young third-party layout engine fed whatever is typed: a panic in it must not take
    // the app down, only fail this one drawing.
    match catch_unwind(AssertUnwindSafe(|| render_strict(source, options))) {
        Ok(svg) => Ok(svg?),
        Err(_) => {
            log::warn!(
                "the Mermaid renderer panicked on a diagram of {} bytes",
                source.len()
            );
            Err(DiagramError::RendererFailed)
        }
    }
}

/// The SVG's pixels at `DRAWING_SCALE` (less for a very large diagram), and its size in points.
fn rasterize(svg: &str) -> Result<(Pixmap, Size<Pixels>), DiagramError> {
    let options = usvg::Options {
        fontdb: FONTS.clone(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)?;
    let natural = tree.size();
    let scale = DRAWING_SCALE.min(MAX_DRAWN_SIDE / natural.width().max(natural.height()));
    let mut pixmap = Pixmap::new(
        (natural.width() * scale).ceil() as u32,
        (natural.height() * scale).ceil() as u32,
    )
    .ok_or(DiagramError::Empty)?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok((pixmap, size(px(natural.width()), px(natural.height()))))
}

/// gpui takes images as unpremultiplied BGRA.
fn image_frame(pixmap: Pixmap) -> image::Frame {
    let (width, height) = (pixmap.width(), pixmap.height());
    let bgra: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.blue(), color.green(), color.red(), color.alpha()]
        })
        .collect();
    let buffer = image::RgbaImage::from_raw(width, height, bgra)
        .expect("a pixmap holds four bytes per pixel");
    image::Frame::new(buffer)
}

fn hex(color: Hsla) -> String {
    let rgb = color.to_rgb();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(rgb.r),
        channel(rgb.g),
        channel(rgb.b)
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn style() -> DiagramStyle {
        DiagramStyle::for_tests()
    }

    /// How many pixels of `a` differ from `b`; both are the same size.
    fn differing_pixels(a: &Pixmap, b: &Pixmap) -> usize {
        a.pixels()
            .iter()
            .zip(b.pixels())
            .filter(|(a, b)| a != b)
            .count()
    }

    #[test]
    fn a_flowchart_is_drawn_with_its_labels() {
        let svg = diagram_svg("flowchart LR\n    A[Write] --> B[Publish]", &style()).unwrap();
        let (pixmap, size) = rasterize(&svg).unwrap();
        assert!(size.width > px(100.0) && size.height > px(20.0), "{size:?}");
        assert_eq!(
            pixmap.width(),
            (f32::from(size.width) * DRAWING_SCALE).ceil() as u32
        );
        let background = pixmap.pixels()[0];
        let drawn = pixmap.pixels().iter().filter(|p| **p != background).count();
        assert!(drawn > 1000, "boxes and arrows are drawn: {drawn} pixels");

        // Without system fonts usvg silently drops text, so check labels made pixels: the same
        // SVG with its text taken out must look clearly different.
        assert!(svg.contains(">Write<") && svg.contains(">Publish<"));
        let without_text = svg.replace(">Write<", "><").replace(">Publish<", "><");
        let (unlabelled, _) = rasterize(&without_text).unwrap();
        let label_pixels = differing_pixels(&pixmap, &unlabelled);
        assert!(label_pixels > 200, "labels drew {label_pixels} pixels");
    }

    #[test]
    fn a_drawn_diagram_becomes_a_bgra_image_of_its_drawn_size() {
        let red_page = DiagramStyle {
            background: hsla(0.0, 1.0, 0.5, 1.0),
            ..style()
        };
        let drawn = draw(EXAMPLE_DIAGRAM, &red_page).unwrap();
        let pixels = drawn.image.size(0);
        assert_eq!(
            pixels.width.0 as f32,
            (f32::from(drawn.size.width) * DRAWING_SCALE).ceil()
        );
        let first_pixel = &drawn.image.as_bytes(0).unwrap()[..4];
        assert_eq!(first_pixel, [0, 0, 255, 255], "blue, green, red, alpha");
    }

    #[test]
    fn common_diagram_kinds_draw() {
        let sources = [
            "sequenceDiagram\n    App->>Server: Push\n    Server-->>App: Ack",
            "classDiagram\n    class Page {\n        +String title\n    }",
            "stateDiagram-v2\n    [*] --> Draft\n    Draft --> [*]",
            "erDiagram\n    PAGE ||--|{ BLOCK : contains",
            "pie title Time\n    \"Writing\" : 60\n    \"Editing\" : 40",
            "gantt\n    dateFormat YYYY-MM-DD\n    section Build\n    Editor :a1, 2026-11-01, 7d",
            "mindmap\n  root((Scroll))\n    Pages\n    Sync",
        ];
        for source in sources {
            assert!(draw(source, &style()).is_ok(), "{source}");
        }
    }

    #[test]
    fn bad_source_is_an_error_not_a_panic() {
        assert!(matches!(draw("  \n", &style()), Err(DiagramError::Empty)));
        assert!(matches!(
            draw("not a diagram", &style()),
            Err(DiagramError::Syntax(_))
        ));
        assert!(matches!(
            draw("flowchart TD\n    A -->", &style()),
            Err(DiagramError::Syntax(_))
        ));
    }

    #[test]
    fn a_huge_diagram_is_drawn_smaller() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="100"><rect width="10000" height="100" fill="red"/></svg>"#;
        let (pixmap, size) = rasterize(svg).unwrap();
        assert_eq!(pixmap.width(), MAX_DRAWN_SIDE as u32);
        assert_eq!(size.width, px(10000.0));
    }

    #[test]
    fn theme_colours_reach_the_svg() {
        let svg = diagram_svg(EXAMPLE_DIAGRAM, &style()).unwrap();
        assert!(svg.contains(&hex(style().node_fill)), "node fill");
        assert!(svg.contains(&hex(style().text)), "label colour");
        assert_eq!(hex(hsla(0.0, 0.0, 1.0, 1.0)), "#FFFFFF");
    }

    /// What drawing costs, in a release build:
    /// `cargo test --release -p scroll-app drawing_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn drawing_costs() {
        let started = Instant::now();
        draw(EXAMPLE_DIAGRAM, &style()).unwrap();
        println!("first drawing, loading fonts: {:?}", started.elapsed());
        let median = |source: &str| {
            let mut timings: Vec<Duration> = (0..10)
                .map(|_| {
                    let started = Instant::now();
                    draw(source, &style()).unwrap();
                    started.elapsed()
                })
                .collect();
            timings.sort();
            timings[timings.len() / 2]
        };
        println!("the example: {:?}", median(EXAMPLE_DIAGRAM));
        let large: String = std::iter::once("flowchart TD".to_string())
            .chain((0..100).map(|node| format!("    N{node}[Step {node}] --> N{}", node + 1)))
            .collect::<Vec<_>>()
            .join("\n");
        println!("a 100-node flowchart: {:?}", median(&large));
    }
}
