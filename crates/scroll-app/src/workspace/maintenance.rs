//! Keeping the workspace file small and fast: optimising it after it opens and reclaiming free space.

use gpui::Context;
use scroll_core::store::{StoreError, WorkspaceStore};
use std::time::Instant;

use super::*;

impl Workspace {
    /// Refreshes the store's query statistics on a thread and connection of its own, a moment
    /// after the window is up: on a large file never analysed this reads every table (seconds),
    /// which must neither delay the first frame nor queue in front of the window's reads. It
    /// waits while a first download is still arriving, so nothing else writes to the file then.
    pub fn optimize_soon(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.optimizing = cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(OPTIMIZE_DELAY).await;
            let is_downloading = workspace
                .read_with(cx, |workspace, _| {
                    workspace.downloaded_parts.is_some() || workspace.is_finishing_download
                })
                .unwrap_or(true);
            if is_downloading {
                return;
            }
            let optimizing = std::thread::Builder::new()
                .name("store optimize".into())
                .spawn(move || {
                    let started = Instant::now();
                    match WorkspaceStore::open(&path).and_then(|store| store.optimize()) {
                        Ok(()) => log::debug!("timing: store optimized in {:?}", started.elapsed()),
                        Err(error) => {
                            log::warn!("could not refresh the store's statistics: {error}")
                        }
                    }
                    // The app has just opened and is idle: a good moment to shrink the file.
                    reclaim_free_space(&path);
                });
            if let Err(error) = optimizing {
                log::error!("could not start the optimize thread: {error}");
            }
        });
    }

    /// Returns the space a drained upload queue left behind, on a thread of its own.
    pub(super) fn reclaim_space_in_background(&self) {
        let path = self.path.clone();
        let reclaiming = std::thread::Builder::new()
            .name("store reclaim".into())
            .spawn(move || reclaim_free_space(&path));
        if let Err(error) = reclaiming {
            log::error!("could not start the thread that shrinks the file: {error}");
        }
    }
}

/// Shrinks the file until no unused pages remain. Files from before the store could do this
/// say so, which is not a problem worth reporting.
pub(super) fn reclaim_free_space(path: &std::path::Path) {
    let mut store = match WorkspaceStore::open(path) {
        Ok(store) => store,
        Err(error) => return log::warn!("could not open the file to shrink it: {error}"),
    };
    loop {
        match store.reclaim_free_space(RECLAIM_PAGES) {
            Ok(0) | Err(StoreError::CannotReclaimSpace) => return,
            Ok(_) => {}
            Err(error) => return log::warn!("could not shrink the workspace file: {error}"),
        }
    }
}
