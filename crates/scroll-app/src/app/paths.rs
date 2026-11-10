//! Where the app keeps its files.

use std::path::PathBuf;

const APP_DIRECTORY: &str = "Scroll";
/// The preferences the settings page shows (`scroll_settings`).
pub const PREFERENCES_FILE: &str = "preferences.json";
/// The secret AI tools present to the MCP server; readable by this user only.
pub const MCP_TOKEN_FILE: &str = "mcp-token";

/// `~/Library/Application Support/Scroll`, created if missing.
pub fn data_directory() -> std::io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    let directory = PathBuf::from(home)
        .join("Library/Application Support")
        .join(APP_DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}
