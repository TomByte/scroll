//! The kinds of design token a theme holds. Values are in `palettes`.

use gpui::{BoxShadow, Hsla, Pixels, SharedString, WindowAppearance};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
        }
    }
}

/// A 12-step scale. Steps follow one convention for both greys and the accent:
/// 1–2 backgrounds, 3–5 component fills, 6–8 borders, 9–10 solid fills, 11–12 text.
#[derive(Clone, Debug)]
pub struct Scale {
    pub(super) steps: [Hsla; 12],
}

impl Scale {
    /// `step` is 1-based to match the convention above; out-of-range steps clamp.
    pub fn step(&self, step: usize) -> Hsla {
        self.steps[step.clamp(1, 12) - 1]
    }
}

#[derive(Clone, Debug)]
pub struct SemanticColors {
    /// Main content area.
    pub surface: Hsla,
    /// Sidebar and other secondary panes.
    pub surface_secondary: Hsla,
    /// Popovers, menus, modals, tooltips.
    pub surface_elevated: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    /// Fill for bordered controls (secondary buttons, inputs, checkboxes).
    pub control: Hsla,
    pub control_hover: Hsla,
    pub control_pressed: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub text_on_accent: Hsla,
    pub icon: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub selected: Hsla,
    pub focus_ring: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_pressed: Hsla,
    pub danger: Hsla,
    pub text_selection: Hsla,
    pub caret: Hsla,
    pub link: Hsla,
    pub code_background: Hsla,
    pub callout_background: Hsla,
    pub scrollbar_thumb: Hsla,
    pub backdrop: Hsla,
    pub tooltip_background: Hsla,
    pub tooltip_text: Hsla,
    /// Colours for other people's carets and selections, assigned in order.
    pub collaborators: [Hsla; 4],
}

#[derive(Clone, Debug)]
pub struct Spacing {
    pub xxs: Pixels,
    pub xs: Pixels,
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
    pub xl: Pixels,
    pub xxl: Pixels,
    pub xxxl: Pixels,
}

#[derive(Clone, Copy, Debug)]
pub struct TextSize {
    pub size: Pixels,
    pub line_height: Pixels,
}

#[derive(Clone, Debug)]
pub struct TypeScale {
    pub caption: TextSize,
    pub small: TextSize,
    pub ui: TextSize,
    pub body: TextSize,
    /// Block headings in page content.
    pub heading_1: TextSize,
    pub heading_2: TextSize,
    pub heading_3: TextSize,
    pub code_block: TextSize,
    pub title: TextSize,
}

#[derive(Clone, Debug)]
pub struct Radii {
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
    pub xl: Pixels,
}

#[derive(Clone, Debug)]
pub struct Sizes {
    pub control_small: Pixels,
    pub control_medium: Pixels,
    pub control_large: Pixels,
    pub icon_xsmall: Pixels,
    pub icon_small: Pixels,
    pub icon_medium: Pixels,
    pub icon_large: Pixels,
    pub row_height: Pixels,
    pub tree_indent: Pixels,
    pub checkbox: Pixels,
    pub border_width: Pixels,
    pub focus_ring_width: Pixels,
    pub caret_width: Pixels,
    pub popover_min_width: Pixels,
    pub modal_width: Pixels,
    pub modal_top_offset: Pixels,
    pub drop_indicator_width: Pixels,
    pub scrollbar_width: Pixels,
    /// How tall a combobox's option list grows before it scrolls.
    pub option_list_max_height: Pixels,
    /// The narrowest a tab is while its name is being edited.
    pub tab_editor_min_width: Pixels,
    pub drag_preview_max_width: Pixels,
    /// One emoji in the picker's grid, and one heading row.
    pub emoji_cell: Pixels,
    /// The picker's grid shows this much (eight rows of cells) before it scrolls.
    pub emoji_grid_height: Pixels,
    /// The ring that separates overlapping avatars.
    pub avatar_ring_width: Pixels,
}

#[derive(Clone, Debug)]
pub struct Shadows {
    /// Barely-there lift for raised controls such as secondary buttons.
    pub raised: Vec<BoxShadow>,
    pub popover: Vec<BoxShadow>,
    pub modal: Vec<BoxShadow>,
}

#[derive(Clone, Debug)]
pub struct Fonts {
    pub ui: SharedString,
    pub mono: SharedString,
}
