//! The preferences file and every registered setting, held in one GPUI global.

use std::path::{Path, PathBuf};

use gpui::{App, Global, Task};
use serde_json::{Map, Value};

use crate::{AnySetting, Category};

/// The stored values (including keys this version does not know, which are kept) and the
/// registered settings. Observe it to follow changes to any setting.
#[derive(Default)]
pub struct Settings {
    /// `None` keeps values in memory only, as in tests that never call `init`.
    path: Option<PathBuf>,
    values: Map<String, Value>,
    registered: Vec<&'static dyn AnySetting>,
    /// The latest save; each waits for the one before, so an older save never lands last.
    saving: Option<Task<()>>,
}

impl Global for Settings {}

/// Reads the preferences file at `path`. Missing values read as defaults; a broken file is
/// reported and replaced on the next save.
pub fn init(path: PathBuf, cx: &mut App) {
    let values = load(&path);
    let settings = cx.default_global::<Settings>();
    settings.path = Some(path);
    settings.values = values;
}

/// Makes `settings` known to the settings page. Each key may be registered once.
pub fn register(settings: &[&'static dyn AnySetting], cx: &mut App) {
    let registered = &mut cx.default_global::<Settings>().registered;
    for setting in settings {
        debug_assert!(
            registered
                .iter()
                .all(|existing| existing.key() != setting.key()),
            "two settings are called {}",
            setting.key()
        );
        registered.push(*setting);
    }
}

impl Settings {
    /// Every registered setting by category, in the order the page shows them: categories in
    /// declaration order, settings in registration order.
    pub fn sections(cx: &App) -> Vec<(Category, Vec<&'static dyn AnySetting>)> {
        let registered = cx
            .try_global::<Settings>()
            .map(|settings| settings.registered.as_slice())
            .unwrap_or_default();
        let mut categories: Vec<Category> = registered
            .iter()
            .map(|setting| setting.category())
            .collect();
        categories.sort();
        categories.dedup();
        categories
            .into_iter()
            .map(|category| {
                let settings = registered
                    .iter()
                    .copied()
                    .filter(|setting| setting.category() == category)
                    .collect();
                (category, settings)
            })
            .collect()
    }

    pub(crate) fn stored(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub(crate) fn store(key: &'static str, value: Value, cx: &mut App) {
        let executor = cx.background_executor().clone();
        let settings = cx.default_global::<Settings>();
        settings.values.insert(key.to_string(), value);
        let Some(path) = settings.path.clone() else {
            return;
        };
        let values = settings.values.clone();
        let previous = settings.saving.take();
        settings.saving = Some(executor.spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            if let Err(error) = save(&path, &values) {
                log::error!("could not save preferences to {}: {error}", path.display());
            }
        }));
    }
}

fn load(path: &Path) -> Map<String, Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Map::new(),
        Err(error) => {
            log::warn!(
                "ignoring unreadable preferences in {}: {error}",
                path.display()
            );
            return Map::new();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        log::warn!(
            "ignoring unreadable preferences in {}: {error}",
            path.display()
        );
        Map::new()
    })
}

/// Replaces the file whole, so a crash mid-write never leaves half of it. Saves never overlap,
/// so one temporary name is enough.
fn save(path: &Path, values: &Map<String, Value>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(values).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests;
