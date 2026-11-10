//! The named themes a person can pick: Scroll's own light and dark, and ports of popular
//! editor themes.

use super::Theme;
use super::sources;
use super::tokens::Appearance;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeId {
    ScrollLight,
    ScrollDark,
    AyuLight,
    AyuMirage,
    AyuDark,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    CatppuccinMocha,
    Dracula,
    GruvboxDark,
    GruvboxLight,
    KanagawaWave,
    KanagawaDragon,
    KanagawaLotus,
    Nord,
    RosePine,
    RosePineDawn,
    TokyoNight,
}

impl ThemeId {
    /// Every theme in the order a picker lists them: Scroll's own first, then by family, each
    /// family's variants in the order the family itself lists them.
    pub const ALL: &'static [ThemeId] = &[
        ThemeId::ScrollLight,
        ThemeId::ScrollDark,
        ThemeId::AyuLight,
        ThemeId::AyuMirage,
        ThemeId::AyuDark,
        ThemeId::CatppuccinLatte,
        ThemeId::CatppuccinFrappe,
        ThemeId::CatppuccinMacchiato,
        ThemeId::CatppuccinMocha,
        ThemeId::Dracula,
        ThemeId::GruvboxDark,
        ThemeId::GruvboxLight,
        ThemeId::KanagawaWave,
        ThemeId::KanagawaDragon,
        ThemeId::KanagawaLotus,
        ThemeId::Nord,
        ThemeId::RosePine,
        ThemeId::RosePineDawn,
        ThemeId::TokyoNight,
    ];

    /// The theme a window uses while it follows the system appearance.
    pub fn system_default(appearance: Appearance) -> ThemeId {
        match appearance {
            Appearance::Light => ThemeId::ScrollLight,
            Appearance::Dark => ThemeId::ScrollDark,
        }
    }

    /// What a settings file stores; must never change. Scroll's own themes keep the ids the
    /// old light/dark setting stored, so those settings still load.
    pub fn id(self) -> &'static str {
        match self {
            ThemeId::ScrollLight => "light",
            ThemeId::ScrollDark => "dark",
            ThemeId::AyuLight => "ayu-light",
            ThemeId::AyuMirage => "ayu-mirage",
            ThemeId::AyuDark => "ayu-dark",
            ThemeId::CatppuccinLatte => "catppuccin-latte",
            ThemeId::CatppuccinFrappe => "catppuccin-frappe",
            ThemeId::CatppuccinMacchiato => "catppuccin-macchiato",
            ThemeId::CatppuccinMocha => "catppuccin-mocha",
            ThemeId::Dracula => "dracula",
            ThemeId::GruvboxDark => "gruvbox-dark",
            ThemeId::GruvboxLight => "gruvbox-light",
            ThemeId::KanagawaWave => "kanagawa-wave",
            ThemeId::KanagawaDragon => "kanagawa-dragon",
            ThemeId::KanagawaLotus => "kanagawa-lotus",
            ThemeId::Nord => "nord",
            ThemeId::RosePine => "rose-pine",
            ThemeId::RosePineDawn => "rose-pine-dawn",
            ThemeId::TokyoNight => "tokyo-night",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeId::ScrollLight => "Scroll Light",
            ThemeId::ScrollDark => "Scroll Dark",
            ThemeId::AyuLight => "Ayu Light",
            ThemeId::AyuMirage => "Ayu Mirage",
            ThemeId::AyuDark => "Ayu Dark",
            ThemeId::CatppuccinLatte => "Catppuccin Latte",
            ThemeId::CatppuccinFrappe => "Catppuccin Frappé",
            ThemeId::CatppuccinMacchiato => "Catppuccin Macchiato",
            ThemeId::CatppuccinMocha => "Catppuccin Mocha",
            ThemeId::Dracula => "Dracula",
            ThemeId::GruvboxDark => "Gruvbox Dark",
            ThemeId::GruvboxLight => "Gruvbox Light",
            ThemeId::KanagawaWave => "Kanagawa Wave",
            ThemeId::KanagawaDragon => "Kanagawa Dragon",
            ThemeId::KanagawaLotus => "Kanagawa Lotus",
            ThemeId::Nord => "Nord",
            ThemeId::RosePine => "Rosé Pine",
            ThemeId::RosePineDawn => "Rosé Pine Dawn",
            ThemeId::TokyoNight => "Tokyo Night",
        }
    }

    /// The family a picker groups the theme under, e.g. "Catppuccin".
    pub fn family(self) -> &'static str {
        match self {
            ThemeId::ScrollLight | ThemeId::ScrollDark => "Scroll",
            ThemeId::AyuLight | ThemeId::AyuMirage | ThemeId::AyuDark => "Ayu",
            ThemeId::CatppuccinLatte
            | ThemeId::CatppuccinFrappe
            | ThemeId::CatppuccinMacchiato
            | ThemeId::CatppuccinMocha => "Catppuccin",
            ThemeId::Dracula => "Dracula",
            ThemeId::GruvboxDark | ThemeId::GruvboxLight => "Gruvbox",
            ThemeId::KanagawaWave | ThemeId::KanagawaDragon | ThemeId::KanagawaLotus => "Kanagawa",
            ThemeId::Nord => "Nord",
            ThemeId::RosePine | ThemeId::RosePineDawn => "Rosé Pine",
            ThemeId::TokyoNight => "Tokyo Night",
        }
    }

    pub fn appearance(self) -> Appearance {
        match self {
            ThemeId::ScrollLight
            | ThemeId::AyuLight
            | ThemeId::CatppuccinLatte
            | ThemeId::GruvboxLight
            | ThemeId::KanagawaLotus
            | ThemeId::RosePineDawn => Appearance::Light,
            ThemeId::ScrollDark
            | ThemeId::AyuMirage
            | ThemeId::AyuDark
            | ThemeId::CatppuccinFrappe
            | ThemeId::CatppuccinMacchiato
            | ThemeId::CatppuccinMocha
            | ThemeId::Dracula
            | ThemeId::GruvboxDark
            | ThemeId::KanagawaWave
            | ThemeId::KanagawaDragon
            | ThemeId::Nord
            | ThemeId::RosePine
            | ThemeId::TokyoNight => Appearance::Dark,
        }
    }
}

impl Theme {
    /// Builds theme `id`. Windows read the shared copy through `Theme::current`; this is for
    /// code that needs a theme no window is using, such as a preview.
    pub fn named(id: ThemeId) -> Theme {
        let source = match id {
            ThemeId::ScrollLight => return Theme::light(),
            ThemeId::ScrollDark => return Theme::dark(),
            ThemeId::AyuLight => &sources::AYU_LIGHT,
            ThemeId::AyuMirage => &sources::AYU_MIRAGE,
            ThemeId::AyuDark => &sources::AYU_DARK,
            ThemeId::CatppuccinLatte => &sources::CATPPUCCIN_LATTE,
            ThemeId::CatppuccinFrappe => &sources::CATPPUCCIN_FRAPPE,
            ThemeId::CatppuccinMacchiato => &sources::CATPPUCCIN_MACCHIATO,
            ThemeId::CatppuccinMocha => &sources::CATPPUCCIN_MOCHA,
            ThemeId::Dracula => &sources::DRACULA,
            ThemeId::GruvboxDark => &sources::GRUVBOX_DARK,
            ThemeId::GruvboxLight => &sources::GRUVBOX_LIGHT,
            ThemeId::KanagawaWave => &sources::KANAGAWA_WAVE,
            ThemeId::KanagawaDragon => &sources::KANAGAWA_DRAGON,
            ThemeId::KanagawaLotus => &sources::KANAGAWA_LOTUS,
            ThemeId::Nord => &sources::NORD,
            ThemeId::RosePine => &sources::ROSE_PINE,
            ThemeId::RosePineDawn => &sources::ROSE_PINE_DAWN,
            ThemeId::TokyoNight => &sources::TOKYO_NIGHT,
        };
        Theme::from_source(source, id.appearance())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use gpui::Hsla;

    use super::*;
    use crate::theme::derivation::contrast_ratio;

    /// Themes whose published body text is deliberately soft, held to 4.5:1 rather than 7:1.
    const SOFT_THEMES: [ThemeId; 2] = [ThemeId::AyuLight, ThemeId::KanagawaLotus];

    #[test]
    fn every_theme_has_its_own_id_and_label() {
        let ids: HashSet<_> = ThemeId::ALL.iter().map(|id| id.id()).collect();
        let labels: HashSet<_> = ThemeId::ALL.iter().map(|id| id.label()).collect();
        assert_eq!(ids.len(), ThemeId::ALL.len());
        assert_eq!(labels.len(), ThemeId::ALL.len());
    }

    #[test]
    fn a_family_is_listed_together() {
        let mut families_seen = Vec::new();
        for id in ThemeId::ALL {
            if families_seen.last() != Some(&id.family()) {
                assert!(!families_seen.contains(&id.family()), "{id:?} is split off");
                families_seen.push(id.family());
            }
        }
    }

    #[test]
    fn each_theme_is_built_for_its_appearance() {
        for &id in ThemeId::ALL {
            assert_eq!(Theme::named(id).appearance, id.appearance(), "{id:?}");
        }
    }

    #[test]
    fn every_theme_keeps_its_text_readable() {
        for &id in ThemeId::ALL {
            let colors = Theme::named(id).colors;
            let body_minimum = if SOFT_THEMES.contains(&id) { 4.5 } else { 7.0 };
            assert_contrast(id, "body text", colors.text, colors.surface, body_minimum);
            for background in [
                colors.surface,
                colors.surface_secondary,
                colors.surface_elevated,
            ] {
                assert_contrast(id, "muted text", colors.text_muted, background, 4.5);
                assert_contrast(id, "link", colors.link, background, 4.5);
            }
            assert_contrast(
                id,
                "primary button label",
                colors.text_on_primary,
                colors.primary,
                4.5,
            );
        }
    }

    fn assert_contrast(id: ThemeId, what: &str, foreground: Hsla, background: Hsla, minimum: f32) {
        let ratio = contrast_ratio(foreground, background);
        assert!(
            ratio >= minimum,
            "{id:?}: {what} contrast {ratio:.2} is under {minimum}"
        );
    }
}
