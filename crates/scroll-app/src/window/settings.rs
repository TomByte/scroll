//! The appearance preferences (theme and motion), and this Mac's window frame and sidebar width
//! in the data directory, which are state rather than preferences. What belongs to one workspace is in
//! `workspace::settings`.

use std::path::{Path, PathBuf};

use gpui::App;
use scroll_settings::{AnySetting, Category, Choice, Setting, Settings};
use scroll_ui::motion::{self, MotionPreference};
use serde::{Deserialize, Serialize};

use crate::json_file;

const SETTINGS_FILE: &str = "settings.json";

pub const THEME: Setting<ThemePreference> = Setting {
    key: "appearance.theme",
    category: Category::Appearance,
    label: "Theme",
    description: "Light, dark, or whichever macOS is using.",
    default: ThemePreference::System,
};

pub const MOTION: Setting<MotionChoice> = Setting {
    key: "appearance.motion",
    category: Category::Appearance,
    label: "Animations",
    description: "Reduced shows every change at once, without movement.",
    default: MotionChoice::Full,
};

pub static SETTINGS: &[&dyn AnySetting] = &[&THEME, &MOTION];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

impl Choice for ThemePreference {
    const OPTIONS: &'static [Self] = &[
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ];

    fn id(self) -> &'static str {
        match self {
            ThemePreference::System => "system",
            ThemePreference::Light => "light",
            ThemePreference::Dark => "dark",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "System",
            ThemePreference::Light => "Light",
            ThemePreference::Dark => "Dark",
        }
    }
}

/// The motion setting's own type: `scroll_settings::Choice` cannot be implemented here for the
/// UI kit's `MotionPreference`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionChoice {
    Full,
    Reduced,
}

impl Choice for MotionChoice {
    const OPTIONS: &'static [Self] = &[MotionChoice::Full, MotionChoice::Reduced];

    fn id(self) -> &'static str {
        match self {
            MotionChoice::Full => "full",
            MotionChoice::Reduced => "reduced",
        }
    }

    fn label(self) -> &'static str {
        match self {
            MotionChoice::Full => "Full",
            MotionChoice::Reduced => "Reduced",
        }
    }
}

/// Hands the motion setting to the UI kit now and whenever settings change.
pub fn follow_motion(cx: &mut App) {
    apply_motion(cx);
    cx.observe_global::<Settings>(apply_motion).detach();
}

fn apply_motion(cx: &mut App) {
    let preference = match MOTION.get(cx) {
        MotionChoice::Full => MotionPreference::Full,
        MotionChoice::Reduced => MotionPreference::Reduced,
    };
    if motion::preference(cx) != preference {
        motion::set_preference(preference, cx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub window: Option<WindowFrame>,
    /// The sidebar's width as last dragged; unset until it is first resized.
    pub sidebar_width: Option<f32>,
}

impl AppSettings {
    /// Missing settings fall back to defaults; a broken file is reported and then replaced.
    pub fn load(directory: &Path) -> AppSettings {
        json_file::read(&settings_path(directory))
            .unwrap_or_else(|error| {
                log::warn!("ignoring unreadable settings in {error}");
                None
            })
            .unwrap_or_default()
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        json_file::save(&settings_path(directory), self)
    }
}

fn settings_path(directory: &Path) -> PathBuf {
    directory.join(SETTINGS_FILE)
}

#[cfg(test)]
mod tests {
    use scroll_core::id::PageId;

    use super::*;

    #[test]
    fn settings_round_trip_and_tolerate_missing_fields() {
        let directory =
            std::env::temp_dir().join(format!("scroll-settings-{}", PageId::generate().0));
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(AppSettings::load(&directory), AppSettings::default());

        let settings = AppSettings {
            window: Some(WindowFrame {
                x: 10.0,
                y: 20.0,
                width: 900.0,
                height: 700.0,
            }),
            sidebar_width: Some(320.0),
        };
        settings.save(&directory).unwrap();
        assert_eq!(AppSettings::load(&directory), settings);

        std::fs::write(settings_path(&directory), r#"{"theme":"Light"}"#).unwrap();
        assert_eq!(AppSettings::load(&directory), AppSettings::default());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
