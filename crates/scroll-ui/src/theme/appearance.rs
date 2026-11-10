//! Which theme each window uses: the system appearance, unless one is forced on the window.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Global, Window, WindowId};

use super::Theme;
use super::tokens::Appearance;

/// Global holding both variants and any per-window appearance overrides.
pub(crate) struct Themes {
    light: Arc<Theme>,
    dark: Arc<Theme>,
    forced_appearances: HashMap<WindowId, Appearance>,
}

impl Global for Themes {}

impl Themes {
    pub(crate) fn install(cx: &mut App) {
        cx.set_global(Themes {
            light: Arc::new(Theme::light()),
            dark: Arc::new(Theme::dark()),
            forced_appearances: HashMap::new(),
        });
    }

    pub(super) fn for_window(&self, window: &Window) -> Arc<Theme> {
        let appearance = self
            .forced_appearances
            .get(&window.window_handle().window_id())
            .copied()
            .unwrap_or_else(|| window.appearance().into());
        self.for_appearance(appearance)
    }

    fn for_appearance(&self, appearance: Appearance) -> Arc<Theme> {
        match appearance {
            Appearance::Light => self.light.clone(),
            Appearance::Dark => self.dark.clone(),
        }
    }
}

/// Pins `window` to one appearance regardless of the system setting.
pub fn force_window_appearance(appearance: Appearance, window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    cx.global_mut::<Themes>()
        .forced_appearances
        .insert(window_id, appearance);
    window.refresh();
}

/// Lets `window` follow the system appearance again after `force_window_appearance`.
pub fn follow_system_in_window(window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    cx.global_mut::<Themes>()
        .forced_appearances
        .remove(&window_id);
    window.refresh();
}

/// Repaints `window` when the system switches between light and dark.
/// The returned subscription must be kept alive for as long as the window should follow.
pub fn follow_system_appearance(window: &mut Window) -> gpui::Subscription {
    window.observe_window_appearance(|window, _| window.refresh())
}
