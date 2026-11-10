//! JSON files the app keeps next to the workspaces: read whole, and replaced atomically.

use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
#[error("{path}: {reason}")]
pub struct UnreadableFile {
    pub path: String,
    pub reason: String,
}

/// The file's contents, or `None` when there is no file yet.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, UnreadableFile> {
    let unreadable = |reason: String| UnreadableFile {
        path: path.display().to_string(),
        reason,
    };
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| unreadable(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unreadable(error.to_string())),
    }
}

/// Writes `value` to `path` so a crash mid-write never leaves a half-written file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    // A name of its own, so saves running at the same time never rename each other's file.
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
    std::fs::write(&temporary, text)?;
    std::fs::rename(temporary, path)
}
