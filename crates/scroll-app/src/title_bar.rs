//! The window's title bar, which the app draws itself: the toolbar and the top of the sidebar
//! share its height, and the system's close, minimise and zoom buttons sit over it.

use gpui::{Pixels, px};
use scroll_ui::Theme;

/// How far the close, minimise and zoom buttons sit from the window's top left corner.
pub const TRAFFIC_LIGHT_INSET: Pixels = px(14.0);
/// The room those buttons take at the toolbar's start when the sidebar is hidden.
pub const TRAFFIC_LIGHTS_WIDTH: Pixels = px(80.0);

pub fn height(theme: &Theme) -> Pixels {
    theme.spacing.xxxl
}
