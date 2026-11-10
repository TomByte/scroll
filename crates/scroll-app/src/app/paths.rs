//! Where the app keeps its files.

use std::path::PathBuf;

const APP_DIRECTORY: &str = "Scroll";

/// `~/Library/Application Support/Scroll`, created if missing.
pub fn data_directory() -> std::io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    let directory = PathBuf::from(home)
        .join("Library/Application Support")
        .join(APP_DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}
