use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

use crate::display::icon::IconName;

/// Serves the kit's embedded icons. Pass to `Application::with_assets`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let svg = IconName::ALL
            .iter()
            .find(|icon| icon.path() == path)
            .map(|icon| Cow::Borrowed(icon.svg_source().as_bytes()));
        Ok(svg)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(IconName::ALL
            .iter()
            .map(|icon| SharedString::from(icon.path()))
            .filter(|icon_path| icon_path.starts_with(path))
            .collect())
    }
}
