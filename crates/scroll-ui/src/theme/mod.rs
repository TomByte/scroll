//! Design tokens. Every colour, size and spacing value the kit draws with comes from here.
//!
//! Each window resolves its theme from its own appearance, so a window follows the system
//! light/dark setting unless an appearance has been forced on it.

mod appearance;
mod palettes;
mod swatches;
mod tokens;

use std::sync::Arc;

use gpui::{App, Window};

pub(crate) use appearance::Themes;
pub use appearance::{follow_system_appearance, follow_system_in_window, force_window_appearance};
pub use swatches::SWATCH_COUNT;
pub use tokens::{
    Appearance, Fonts, Radii, Scale, SemanticColors, Shadows, Sizes, Spacing, TextSize, TypeScale,
};

#[derive(Clone, Debug)]
pub struct Theme {
    pub appearance: Appearance,
    /// Applied to whole controls when they are disabled.
    pub disabled_opacity: f32,
    pub colors: SemanticColors,
    pub spacing: Spacing,
    pub text: TypeScale,
    pub radius: Radii,
    pub size: Sizes,
    pub shadow: Shadows,
    pub font: Fonts,
}

impl Theme {
    /// The theme for `window`: its forced appearance if one was set, otherwise the system one.
    pub fn current(window: &Window, cx: &App) -> Arc<Theme> {
        cx.global::<Themes>().for_window(window)
    }
}

impl Theme {
    /// The colour of collaborator `index` (wrapping), for their caret, selection and avatar.
    pub fn collaborator_color(&self, index: usize) -> gpui::Hsla {
        self.colors.collaborators[index % self.colors.collaborators.len()]
    }
}
