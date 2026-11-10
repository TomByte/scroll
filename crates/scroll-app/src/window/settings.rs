//! This Mac's app settings, in the data directory: the window's frame and the theme choice.
//! What belongs to one workspace is in `workspace::settings`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::json_file;

const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
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
    pub theme: ThemePreference,
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
            theme: ThemePreference::Dark,
        };
        settings.save(&directory).unwrap();
        assert_eq!(AppSettings::load(&directory), settings);

        std::fs::write(settings_path(&directory), r#"{"theme":"Light"}"#).unwrap();
        assert_eq!(AppSettings::load(&directory).theme, ThemePreference::Light);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
