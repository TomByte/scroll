//! Which theme each window uses: Scroll's own light or dark, following the system appearance,
//! unless a theme is forced on the window.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Global, Window, WindowId};

use super::Theme;
use super::catalog::ThemeId;
use super::tokens::Appearance;

/// Global holding every named theme, built once, and any per-window choice of theme.
pub(crate) struct Themes {
    themes: HashMap<ThemeId, Arc<Theme>>,
    forced_themes: HashMap<WindowId, ThemeId>,
}

impl Global for Themes {}

impl Themes {
    pub(crate) fn install(cx: &mut App) {
        cx.set_global(Themes {
            themes: ThemeId::ALL
                .iter()
                .map(|&id| (id, Arc::new(Theme::named(id))))
                .collect(),
            forced_themes: HashMap::new(),
        });
    }

    pub(super) fn for_window(&self, window: &Window) -> Arc<Theme> {
        let id = self
            .forced_themes
            .get(&window.window_handle().window_id())
            .copied()
            .unwrap_or_else(|| ThemeId::system_default(window.appearance().into()));
        self.theme(id)
    }

    pub(crate) fn theme(&self, id: ThemeId) -> Arc<Theme> {
        self.themes[&id].clone()
    }
}

/// Pins `window` to theme `id` regardless of the system setting.
pub fn force_window_theme(id: ThemeId, window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    cx.global_mut::<Themes>()
        .forced_themes
        .insert(window_id, id);
    window.refresh();
}

/// Pins `window` to Scroll's own theme for `appearance` regardless of the system setting.
pub fn force_window_appearance(appearance: Appearance, window: &mut Window, cx: &mut App) {
    force_window_theme(ThemeId::system_default(appearance), window, cx);
}

/// Lets `window` follow the system appearance again after `force_window_theme`.
pub fn follow_system_in_window(window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    cx.global_mut::<Themes>().forced_themes.remove(&window_id);
    window.refresh();
}

/// Repaints `window` when the system switches between light and dark.
/// The returned subscription must be kept alive for as long as the window should follow.
pub fn follow_system_appearance(window: &mut Window) -> gpui::Subscription {
    window.observe_window_appearance(|window, _| window.refresh())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn a_forced_theme_holds_until_the_window_follows_the_system_again(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let surface = |window: &Window, cx: &App| Theme::current(window, cx).colors.surface;
            let system_surface = surface(window, cx);
            force_window_theme(ThemeId::Nord, window, cx);
            assert_eq!(
                surface(window, cx),
                Theme::named(ThemeId::Nord).colors.surface
            );
            follow_system_in_window(window, cx);
            assert_eq!(surface(window, cx), system_surface);
        });
    }
}
