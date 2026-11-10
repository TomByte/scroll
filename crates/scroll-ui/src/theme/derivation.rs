//! Turns a published palette's few source colours into a whole theme, deriving the rest the way
//! the brand palettes derive theirs: overlays are the text colour at low opacity, the focus ring
//! and selection the accent.

use gpui::{Hsla, Rgba, hsla, rgb};

use super::Theme;
use super::palettes::{fonts, radii, shadow, sizes, spacing, type_scale};
use super::sources::{SourceColors, SourceSyntax};
use super::tokens::{Appearance, SemanticColors, Shadows, SyntaxColors};

/// WCAG AA for body-size text: what muted text, links and button labels must reach.
const READABLE_CONTRAST: f32 = 4.5;

/// How far `readable_against` moves a colour's lightness per try.
const LIGHTNESS_STEP: f32 = 0.005;

/// How far a primary button's fill moves away from its label on hover, and when pressed.
const PRIMARY_HOVER_SHIFT: f32 = 0.05;
const PRIMARY_PRESSED_SHIFT: f32 = 0.1;

/// Opacities of the text colour (and of the accent, for the focus ring and selection) that
/// make each overlay; the brand palettes' own values for each appearance.
struct Strengths {
    border: f32,
    border_strong: f32,
    hover: f32,
    pressed: f32,
    selected: f32,
    code_background: f32,
    callout_background: f32,
    scrollbar_thumb: f32,
    backdrop: f32,
    focus_ring: f32,
    text_selection: f32,
    /// Raised, popover and modal shadows.
    shadows: [f32; 3],
    disabled_opacity: f32,
}

const LIGHT: Strengths = Strengths {
    border: 0.09,
    border_strong: 0.16,
    hover: 0.045,
    pressed: 0.08,
    selected: 0.065,
    code_background: 0.06,
    callout_background: 0.04,
    scrollbar_thumb: 0.22,
    backdrop: 0.18,
    focus_ring: 0.28,
    text_selection: 0.2,
    shadows: [0.05, 0.10, 0.14],
    disabled_opacity: 0.45,
};

const DARK: Strengths = Strengths {
    border: 0.085,
    border_strong: 0.14,
    hover: 0.055,
    pressed: 0.09,
    selected: 0.075,
    code_background: 0.07,
    callout_background: 0.05,
    scrollbar_thumb: 0.22,
    backdrop: 0.5,
    focus_ring: 0.38,
    text_selection: 0.32,
    shadows: [0.25, 0.28, 0.36],
    disabled_opacity: 0.4,
};

impl Theme {
    pub(super) fn from_source(source: &SourceColors, appearance: Appearance) -> Theme {
        let strengths = match appearance {
            Appearance::Light => &LIGHT,
            Appearance::Dark => &DARK,
        };
        Theme {
            appearance,
            disabled_opacity: strengths.disabled_opacity,
            colors: semantic_colors(source, appearance, strengths),
            syntax: syntax_colors(&source.syntax),
            shadow: shadows(shade(source, appearance), strengths),
            spacing: spacing(),
            text: type_scale(),
            radius: radii(),
            size: sizes(),
            font: fonts(),
        }
    }
}

fn semantic_colors(
    source: &SourceColors,
    appearance: Appearance,
    strengths: &Strengths,
) -> SemanticColors {
    let surface = hex(source.surface);
    let surface_secondary = hex(source.surface_secondary);
    let surface_elevated = hex(source.surface_elevated);
    let backgrounds = [surface, surface_secondary, surface_elevated];
    let text = hex(source.text);
    let text_muted = readable_against(hex(source.text_muted), &backgrounds, READABLE_CONTRAST);
    let text_subtle = hex(source.text_subtle);
    let ink = |alpha: f32| text.opacity(alpha);
    // The accent doubles as the primary button's fill under a label in the page's own colour,
    // as Catppuccin's guide has it, so it is kept readable against the page.
    let accent = readable_against(hex(source.accent), &[surface], READABLE_CONTRAST);
    let primary_hover = shifted_away_from(surface, accent, PRIMARY_HOVER_SHIFT);
    let control_pressed = surface_elevated.blend(ink(strengths.pressed));
    let (tooltip_background, tooltip_text) = match appearance {
        Appearance::Light => (text, surface),
        Appearance::Dark => (control_pressed, text),
    };
    SemanticColors {
        surface,
        surface_secondary,
        surface_elevated,
        border: ink(strengths.border),
        border_strong: ink(strengths.border_strong),
        control: surface_elevated,
        control_hover: surface_elevated.blend(ink(strengths.hover)),
        control_pressed,
        text,
        text_muted,
        text_placeholder: text_subtle,
        text_on_accent: surface,
        icon: text_subtle.blend(text_muted.opacity(0.5)),
        hover: ink(strengths.hover),
        pressed: ink(strengths.pressed),
        selected: ink(strengths.selected),
        focus_ring: accent.opacity(strengths.focus_ring),
        accent,
        accent_hover: primary_hover,
        primary: accent,
        primary_hover,
        primary_pressed: shifted_away_from(surface, accent, PRIMARY_PRESSED_SHIFT),
        text_on_primary: surface,
        danger: hex(source.danger),
        text_selection: accent.opacity(strengths.text_selection),
        caret: accent,
        link: readable_against(hex(source.link), &backgrounds, READABLE_CONTRAST),
        code_background: ink(strengths.code_background),
        callout_background: ink(strengths.callout_background),
        scrollbar_thumb: ink(strengths.scrollbar_thumb),
        backdrop: shade(source, appearance).opacity(strengths.backdrop),
        tooltip_background,
        tooltip_text,
        collaborators: source.collaborators.map(hex),
    }
}

/// What shadows and the modal backdrop darken with: light themes use their own ink, as the
/// brand's navy-tinted shadows do; on a dark page only black reads as a shadow.
fn shade(source: &SourceColors, appearance: Appearance) -> Hsla {
    match appearance {
        Appearance::Light => hex(source.text),
        Appearance::Dark => hsla(0.0, 0.0, 0.0, 1.0),
    }
}

fn syntax_colors(source: &SourceSyntax) -> SyntaxColors {
    SyntaxColors {
        keyword: hex(source.keyword),
        string: hex(source.string),
        comment: hex(source.comment),
        constant: hex(source.constant),
        function: hex(source.function),
        type_name: hex(source.type_name),
    }
}

fn shadows(shade: Hsla, strengths: &Strengths) -> Shadows {
    let [raised, popover, modal] = strengths.shadows;
    Shadows {
        raised: vec![shadow(shade.opacity(raised), 1.0, 2.0, 0.0)],
        popover: vec![shadow(shade.opacity(popover), 4.0, 8.0, 0.0)],
        modal: vec![shadow(shade.opacity(modal), 8.0, 16.0, 0.0)],
    }
}

/// `color` lightened or darkened, away from the first of `backgrounds`, just far enough to
/// reach `minimum` contrast against all of them; unchanged if it already does.
fn readable_against(color: Hsla, backgrounds: &[Hsla], minimum: f32) -> Hsla {
    let direction = direction_away_from(backgrounds[0], color);
    let mut readable = color;
    while lowest_contrast(readable, backgrounds) < minimum {
        let lightness = readable.l + direction * LIGHTNESS_STEP;
        if !(0.0..=1.0).contains(&lightness) {
            break;
        }
        readable.l = lightness;
    }
    readable
}

/// `color` with its lightness moved `amount` further from `background`.
fn shifted_away_from(background: Hsla, color: Hsla, amount: f32) -> Hsla {
    let lightness = color.l + direction_away_from(background, color) * amount;
    Hsla {
        l: lightness.clamp(0.0, 1.0),
        ..color
    }
}

/// +1 to lighten `color` away from `background`, -1 to darken it.
fn direction_away_from(background: Hsla, color: Hsla) -> f32 {
    if luminance(color) >= luminance(background) {
        1.0
    } else {
        -1.0
    }
}

fn lowest_contrast(color: Hsla, backgrounds: &[Hsla]) -> f32 {
    backgrounds
        .iter()
        .map(|background| contrast_ratio(color, *background))
        .fold(f32::INFINITY, f32::min)
}

/// The WCAG contrast ratio between two opaque colours, from 1 (identical) to 21.
pub(super) fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// WCAG relative luminance.
fn luminance(color: Hsla) -> f32 {
    let Rgba { r, g, b, .. } = color.into();
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_ratio_matches_wcag_at_the_extremes() {
        let black = hex(0x000000);
        let white = hex(0xffffff);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 0.01);
    }

    #[test]
    fn a_faint_colour_is_moved_until_it_reads_and_a_readable_one_is_left_alone() {
        let page = hex(0xffffff);
        let faint = hex(0xaaaaaa);
        let nudged = readable_against(faint, &[page], READABLE_CONTRAST);
        assert!(contrast_ratio(nudged, page) >= READABLE_CONTRAST);
        assert!(nudged.l < faint.l);
        let readable = hex(0x333333);
        assert_eq!(
            readable_against(readable, &[page], READABLE_CONTRAST),
            readable
        );
    }
}
