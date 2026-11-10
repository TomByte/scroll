//! What this Mac remembers about one workspace: the page open last, the sidebar pages left
//! expanded and whether its Tags section is open. Kept next to the workspace's file, as `<file>.settings.json`.

use std::path::{Path, PathBuf};

use scroll_core::id::PageId;
use serde::{Deserialize, Serialize};

use crate::json_file;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceSettings {
    pub last_page: Option<PageId>,
    pub expanded_pages: Vec<PageId>,
    /// Whether the sidebar's Tags section is open.
    pub are_tags_shown: bool,
}

impl WorkspaceSettings {
    /// The settings of the workspace in `workspace_file`; defaults when there are none yet or
    /// the file is broken (reported).
    pub fn load(workspace_file: &Path) -> WorkspaceSettings {
        json_file::read(&settings_path(workspace_file))
            .unwrap_or_else(|error| {
                log::warn!("ignoring unreadable workspace settings in {error}");
                None
            })
            .unwrap_or_default()
    }

    pub fn save(&self, workspace_file: &Path) -> std::io::Result<()> {
        json_file::save(&settings_path(workspace_file), self)
    }
}

fn settings_path(workspace_file: &Path) -> PathBuf {
    workspace_file.with_extension("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_settings_round_trip_beside_their_workspace() {
        let directory =
            std::env::temp_dir().join(format!("scroll-settings-{}", PageId::generate().0));
        std::fs::create_dir_all(&directory).unwrap();
        let workspace_file = directory.join("Team.scroll");
        assert_eq!(
            WorkspaceSettings::load(&workspace_file),
            WorkspaceSettings::default()
        );

        let settings = WorkspaceSettings {
            last_page: Some(PageId::generate()),
            expanded_pages: vec![PageId::generate()],
            are_tags_shown: true,
        };
        settings.save(&workspace_file).unwrap();
        assert_eq!(WorkspaceSettings::load(&workspace_file), settings);
        assert!(directory.join("Team.settings.json").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
