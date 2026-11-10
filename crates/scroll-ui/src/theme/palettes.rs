//! The light and dark values of every token.

use gpui::{BoxShadow, Hsla, hsla, point, px};

use super::Theme;
use super::tokens::{
    Appearance, Fonts, Radii, Scale, SemanticColors, Shadows, Sizes, Spacing, SyntaxColors,
    TextSize, TypeScale,
};

impl Theme {
    pub(super) fn light() -> Theme {
        let grey = Scale {
            steps: [
                hsl(220.0, 14.0, 99.2),
                hsl(220.0, 14.0, 97.6),
                hsl(220.0, 12.0, 95.4),
                hsl(220.0, 11.0, 93.0),
                hsl(220.0, 10.0, 90.6),
                hsl(220.0, 10.0, 87.6),
                hsl(220.0, 9.0, 82.6),
                hsl(220.0, 8.0, 74.0),
                hsl(220.0, 6.0, 56.0),
                hsl(220.0, 6.0, 50.0),
                hsl(220.0, 7.0, 40.0),
                hsl(220.0, 14.0, 13.0),
            ],
        };
        let accent = Scale {
            steps: [
                hsl(222.0, 100.0, 99.0),
                hsl(222.0, 100.0, 97.5),
                hsl(222.0, 95.0, 95.0),
                hsl(222.0, 90.0, 92.0),
                hsl(222.0, 85.0, 88.0),
                hsl(222.0, 80.0, 82.0),
                hsl(222.0, 75.0, 74.0),
                hsl(222.0, 72.0, 64.0),
                hsl(222.0, 75.0, 53.0),
                hsl(222.0, 72.0, 48.0),
                hsl(222.0, 70.0, 44.0),
                hsl(222.0, 60.0, 22.0),
            ],
        };
        let ink = |alpha: f32| hsla(220.0 / 360.0, 0.3, 0.15, alpha);
        let colors = SemanticColors {
            surface: hsl(0.0, 0.0, 100.0),
            surface_secondary: grey.step(2),
            surface_elevated: hsl(0.0, 0.0, 100.0),
            border: ink(0.09),
            border_strong: ink(0.16),
            control: hsl(0.0, 0.0, 100.0),
            control_hover: grey.step(2),
            control_pressed: grey.step(3),
            text: grey.step(12),
            text_muted: grey.step(11),
            text_placeholder: grey.step(9),
            text_on_accent: hsl(0.0, 0.0, 100.0),
            icon: grey.step(10),
            hover: ink(0.045),
            pressed: ink(0.08),
            selected: ink(0.065),
            focus_ring: accent.step(9).opacity(0.28),
            accent: accent.step(9),
            accent_hover: accent.step(10),
            accent_pressed: accent.step(11),
            danger: hsl(0.0, 72.0, 51.0),
            text_selection: accent.step(9).opacity(0.2),
            caret: accent.step(9),
            link: accent.step(11),
            code_background: ink(0.06),
            callout_background: ink(0.04),
            scrollbar_thumb: ink(0.22),
            backdrop: hsla(220.0 / 360.0, 0.2, 0.1, 0.18),
            tooltip_background: grey.step(12),
            tooltip_text: grey.step(1),
            collaborators: [
                hsl(24.0, 92.0, 50.0),
                hsl(152.0, 60.0, 38.0),
                hsl(282.0, 60.0, 55.0),
                hsl(340.0, 75.0, 52.0),
            ],
        };
        let shadow_colour = |alpha: f32| hsla(220.0 / 360.0, 0.3, 0.1, alpha);
        Theme {
            appearance: Appearance::Light,
            disabled_opacity: 0.45,
            colors,
            syntax: SyntaxColors {
                keyword: hsl(356.0, 72.0, 47.0),
                string: hsl(218.0, 83.0, 23.0),
                comment: hsl(212.0, 8.0, 47.0),
                constant: hsl(214.0, 94.0, 35.0),
                function: hsl(262.0, 69.0, 59.0),
                type_name: hsl(23.0, 100.0, 29.0),
            },
            shadow: Shadows {
                raised: vec![shadow(shadow_colour(0.05), 1.0, 2.0, 0.0)],
                popover: vec![shadow(shadow_colour(0.10), 4.0, 8.0, 0.0)],
                modal: vec![shadow(shadow_colour(0.14), 8.0, 16.0, 0.0)],
            },
            spacing: spacing(),
            text: type_scale(),
            radius: radii(),
            size: sizes(),
            font: fonts(),
        }
    }

    pub(super) fn dark() -> Theme {
        let grey = Scale {
            steps: [
                hsl(220.0, 7.0, 9.0),
                hsl(220.0, 7.0, 11.0),
                hsl(220.0, 7.0, 13.5),
                hsl(220.0, 7.0, 16.0),
                hsl(220.0, 7.0, 19.0),
                hsl(220.0, 7.0, 22.0),
                hsl(220.0, 7.0, 27.0),
                hsl(220.0, 6.0, 35.0),
                hsl(220.0, 5.0, 46.0),
                hsl(220.0, 5.0, 52.0),
                hsl(220.0, 6.0, 68.0),
                hsl(220.0, 10.0, 93.0),
            ],
        };
        let accent = Scale {
            steps: [
                hsl(222.0, 40.0, 10.0),
                hsl(222.0, 45.0, 12.0),
                hsl(222.0, 55.0, 18.0),
                hsl(222.0, 60.0, 23.0),
                hsl(222.0, 60.0, 28.0),
                hsl(222.0, 60.0, 34.0),
                hsl(222.0, 62.0, 42.0),
                hsl(222.0, 68.0, 50.0),
                hsl(222.0, 80.0, 60.0),
                hsl(222.0, 85.0, 65.0),
                hsl(222.0, 90.0, 74.0),
                hsl(222.0, 90.0, 92.0),
            ],
        };
        let light_ink = |alpha: f32| hsla(220.0 / 360.0, 0.25, 0.92, alpha);
        let colors = SemanticColors {
            surface: grey.step(1),
            surface_secondary: grey.step(2),
            surface_elevated: grey.step(3),
            border: light_ink(0.085),
            border_strong: light_ink(0.14),
            control: grey.step(3),
            control_hover: grey.step(4),
            control_pressed: grey.step(5),
            text: grey.step(12),
            text_muted: grey.step(11),
            text_placeholder: grey.step(9),
            text_on_accent: hsl(0.0, 0.0, 100.0),
            icon: grey.step(10),
            hover: light_ink(0.055),
            pressed: light_ink(0.09),
            selected: light_ink(0.075),
            focus_ring: accent.step(9).opacity(0.38),
            accent: accent.step(9),
            accent_hover: accent.step(10),
            accent_pressed: accent.step(8),
            danger: hsl(0.0, 75.0, 63.0),
            text_selection: accent.step(9).opacity(0.32),
            caret: accent.step(10),
            link: accent.step(11),
            code_background: light_ink(0.07),
            callout_background: light_ink(0.05),
            scrollbar_thumb: light_ink(0.22),
            backdrop: hsla(0.0, 0.0, 0.0, 0.5),
            tooltip_background: grey.step(5),
            tooltip_text: grey.step(12),
            collaborators: [
                hsl(28.0, 95.0, 58.0),
                hsl(152.0, 55.0, 50.0),
                hsl(282.0, 70.0, 70.0),
                hsl(340.0, 80.0, 65.0),
            ],
        };
        let shadow_colour = |alpha: f32| hsla(0.0, 0.0, 0.0, alpha);
        Theme {
            appearance: Appearance::Dark,
            disabled_opacity: 0.4,
            colors,
            syntax: SyntaxColors {
                keyword: hsl(4.0, 100.0, 72.0),
                string: hsl(207.0, 100.0, 82.0),
                comment: hsl(212.0, 9.0, 58.0),
                constant: hsl(208.0, 100.0, 74.0),
                function: hsl(270.0, 100.0, 83.0),
                type_name: hsl(28.0, 100.0, 67.0),
            },
            shadow: Shadows {
                raised: vec![shadow(shadow_colour(0.25), 1.0, 2.0, 0.0)],
                popover: vec![shadow(shadow_colour(0.28), 4.0, 8.0, 0.0)],
                modal: vec![shadow(shadow_colour(0.36), 8.0, 16.0, 0.0)],
            },
            spacing: spacing(),
            text: type_scale(),
            radius: radii(),
            size: sizes(),
            font: fonts(),
        }
    }
}

fn spacing() -> Spacing {
    Spacing {
        xxs: px(2.0),
        xs: px(4.0),
        sm: px(6.0),
        md: px(8.0),
        lg: px(12.0),
        xl: px(16.0),
        xxl: px(24.0),
        xxxl: px(40.0),
    }
}

fn type_scale() -> TypeScale {
    TypeScale {
        caption: text_size(11.0, 14.0),
        small: text_size(12.0, 16.0),
        ui: text_size(13.0, 18.0),
        body: text_size(15.0, 24.0),
        heading_1: text_size(28.0, 36.0),
        heading_2: text_size(22.0, 30.0),
        heading_3: text_size(18.0, 26.0),
        code_block: text_size(13.5, 21.0),
        title: text_size(32.0, 40.0),
    }
}

fn radii() -> Radii {
    Radii {
        sm: px(4.0),
        md: px(6.0),
        lg: px(8.0),
        xl: px(12.0),
    }
}

fn sizes() -> Sizes {
    Sizes {
        control_small: px(24.0),
        control_medium: px(28.0),
        control_large: px(32.0),
        icon_xsmall: px(12.0),
        icon_small: px(14.0),
        icon_medium: px(16.0),
        icon_large: px(20.0),
        row_height: px(28.0),
        tree_indent: px(14.0),
        checkbox: px(14.0),
        switch_width: px(26.0),
        switch_height: px(16.0),
        border_width: px(1.0),
        focus_ring_width: px(3.0),
        caret_width: px(1.5),
        popover_min_width: px(200.0),
        modal_width: px(560.0),
        modal_top_offset: px(96.0),
        drop_indicator_width: px(2.0),
        scrollbar_width: px(6.0),
        option_list_max_height: px(240.0),
        tab_editor_min_width: px(150.0),
        drag_preview_max_width: px(300.0),
        emoji_cell: px(32.0),
        emoji_grid_height: px(256.0),
        avatar_ring_width: px(2.0),
    }
}

fn fonts() -> Fonts {
    Fonts {
        ui: ".SystemUIFont".into(),
        mono: "Menlo".into(),
    }
}

fn hsl(hue_degrees: f32, saturation_percent: f32, lightness_percent: f32) -> Hsla {
    hsla(
        hue_degrees / 360.0,
        saturation_percent / 100.0,
        lightness_percent / 100.0,
        1.0,
    )
}

fn text_size(size: f32, line_height: f32) -> TextSize {
    TextSize {
        size: px(size),
        line_height: px(line_height),
    }
}

/// GPUI takes `blur` as the Gaussian's standard deviation, about half what CSS calls the blur
/// radius, so the shadow fades out over roughly three times `blur`.
fn shadow(color: Hsla, offset_y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.0), px(offset_y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_panels_cast_one_soft_shadow_with_no_spread() {
        for theme in [Theme::light(), Theme::dark()] {
            for shadows in [&theme.shadow.popover, &theme.shadow.modal] {
                assert_eq!(shadows.len(), 1, "{:?}: one layer", theme.appearance);
                assert!(shadows[0].spread_radius >= px(0.0));
                assert!(shadows[0].blur_radius <= px(16.0));
            }
        }
    }
}
