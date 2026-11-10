use gpui::{
    AnyElement, AnyView, App, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled,
    TextRun, Window, div, font, px,
};

use crate::display::pill::Pill;
use crate::foundation::story::stateless_story;
use crate::theme::Theme;

/// Pills on one line in a known width: as many as fit whole, then a "+N" count for the rest,
/// so a pill is never cut mid-word, e.g. a table cell's select options.
#[derive(IntoElement)]
pub struct PillRow {
    width: Pixels,
    pills: Vec<(SharedString, usize)>,
}

impl PillRow {
    pub fn new(width: Pixels) -> Self {
        PillRow {
            width,
            pills: Vec::new(),
        }
    }

    /// A pill with `label` in swatch `swatch`.
    pub fn pill(mut self, label: impl Into<SharedString>, swatch: usize) -> Self {
        self.pills.push((label.into(), swatch));
        self
    }
}

impl RenderOnce for PillRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let widths: Vec<Pixels> = self
            .pills
            .iter()
            .map(|(label, _)| label_width(label, &theme, window))
            .collect();
        let count_estimate = format!("+{}", self.pills.len());
        let spacing = PillSpacing {
            padding: theme.spacing.sm,
            gap: theme.spacing.xs,
            overflow_width: label_width(&count_estimate, &theme, window) + theme.spacing.sm * 2.0,
        };
        let fitting = pills_that_fit(&widths, self.width, &spacing);
        let hidden = self.pills.len() - fitting;
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .gap(theme.spacing.xs)
            .overflow_hidden()
            .children(
                self.pills
                    .into_iter()
                    .take(fitting)
                    .map(|(label, swatch)| Pill::new(label).swatch(swatch)),
            )
            .children((hidden > 0).then(|| Pill::new(format!("+{hidden}")).neutral()))
    }
}

/// The space each pill adds around its label and between pills, and the width of the count
/// pill shown for the rest.
struct PillSpacing {
    padding: Pixels,
    gap: Pixels,
    overflow_width: Pixels,
}

/// How many of the pills (by label width) to show in `available` width. At least one is always
/// shown, truncated if it alone is too wide.
fn pills_that_fit(label_widths: &[Pixels], available: Pixels, spacing: &PillSpacing) -> usize {
    let mut used = px(0.0);
    for (index, label_width) in label_widths.iter().enumerate() {
        let pill = *label_width + spacing.padding * 2.0;
        let gap = if index == 0 { px(0.0) } else { spacing.gap };
        let is_last = index == label_widths.len() - 1;
        let reserve = if is_last {
            px(0.0)
        } else {
            spacing.gap + spacing.overflow_width
        };
        if used + gap + pill + reserve > available {
            return index.max(1);
        }
        used += gap + pill;
    }
    label_widths.len()
}

/// The width of a pill label, measured with the font and size pills use.
fn label_width(text: &str, theme: &Theme, window: &Window) -> Pixels {
    let run = TextRun {
        len: text.len(),
        font: font(theme.font.ui.clone()),
        color: theme.colors.text,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_string().into(), theme.text.small.size, &[run], None)
        .width
}

impl PillRow {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    let row = |width: Pixels| {
        div().w(width).flex().child(
            PillRow::new(width)
                .pill("Design", 5)
                .pill("Research", 4)
                .pill("Engineering", 2)
                .pill("Marketing", 7),
        )
    };
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.sm)
        .child(row(theme.size.popover_min_width * 2.0))
        .child(row(theme.size.popover_min_width))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACING: PillSpacing = PillSpacing {
        padding: px(6.0),
        gap: px(4.0),
        overflow_width: px(28.0),
    };

    #[test]
    fn pills_that_fit_leave_room_for_the_count() {
        let widths = [px(40.0), px(40.0), px(40.0)];
        // Each pill is 52 wide; all three take 52 * 3 + 8 = 164.
        assert_eq!(pills_that_fit(&widths, px(164.0), &SPACING), 3);
        // Two pills plus the count need 52 + 4 + 52 + 4 + 28 = 140.
        assert_eq!(pills_that_fit(&widths, px(140.0), &SPACING), 2);
        assert_eq!(pills_that_fit(&widths, px(139.0), &SPACING), 1);
        assert_eq!(pills_that_fit(&widths, px(10.0), &SPACING), 1);
        assert_eq!(pills_that_fit(&[], px(10.0), &SPACING), 0);
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(PillRow::story, cx);
    }
}
