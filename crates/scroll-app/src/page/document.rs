//! The open page's document: reading it from storage (waiting a moment for content still on
//! its way from the server), showing it in the editor, and saving and compacting its edits.

use super::*;

impl PageView {
    pub(super) fn load_document(
        page: PageId,
        workspace: &Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let loading = workspace.read(cx).storage().read(cx, move |store| {
            storage::load_page(store, page).map(PageDocument::new)
        });
        cx.spawn_in(window, async move |view, cx| {
            let loaded = loading.await;
            let shown = view.update_in(cx, |view, window, cx| {
                view.show_document(loaded, window, cx)
            });
            if let Err(error) = shown {
                log::warn!("page closed before it finished loading: {error}");
            }
        })
        .detach();
    }

    pub(super) fn show_document(
        &mut self,
        loaded: Result<PageDocument, StorageError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = match loaded {
            Ok(page) => page,
            Err(error) => {
                log::error!("could not open page {:?}: {error}", self.page);
                self.body = Body::Failed(error.to_string());
                self.mark_ready(cx);
                return cx.notify();
            }
        };
        let workspace = self.workspace.read(cx);
        let is_reachable = matches!(
            workspace.connection(),
            Some(ConnectionStatus::Online | ConnectionStatus::Connecting)
        );
        if page.blocks().is_empty() && is_reachable {
            // The content is probably still on its way; adding a first block now would leave an
            // empty line above it on every device.
            self.pending_document = Some(page);
            self.pending_timeout = cx.spawn_in(window, async move |view, cx| {
                cx.background_executor().timer(CONTENT_WAIT).await;
                let shown = view.update_in(cx, |view, window, cx| {
                    view.show_pending_document(window, cx)
                });
                if let Err(error) = shown {
                    log::debug!("page closed while waiting for its content: {error}");
                }
            });
            return;
        }
        self.show_editor(page, window, cx);
    }

    pub(super) fn show_pending_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.pending_document.take() {
            self.show_editor(page, window, cx);
        }
    }

    pub(super) fn show_editor(
        &mut self,
        page: PageDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first_block = match page.ensure_block() {
            Ok(block) => block,
            Err(error) => {
                self.body = Body::Failed(error.to_string());
                self.mark_ready(cx);
                return cx.notify();
            }
        };
        let header: AnyView = self.header(cx).into();
        let access = self.access;
        let early_updates = std::mem::take(&mut self.early_updates);
        let editor = cx.new(|cx| {
            let mut editor = PageEditor::new(page, Selection::caret(first_block, 0), cx);
            editor.set_header(header, cx);
            editor.set_access(access, cx);
            editor.apply_remote_updates(&early_updates, cx);
            editor
        });
        self._subscriptions
            .push(cx.subscribe(&editor, |view, _, event, cx| match event {
                PageEditorEvent::Edited => view.save_edits(cx),
                PageEditorEvent::CaretMoved => view.send_presence(cx),
                PageEditorEvent::CreateDatabase => view.create_database(cx),
            }));
        let theme = Theme::current(window, cx);
        editor.update(cx, |editor, cx| {
            editor.draw_first_screen_diagrams(&theme, cx);
            editor.build_first_screen_grammars(cx);
        });
        self.mark_ready_when(&editor, is_first_screen_ready, cx);
        self.body = Body::Document(editor);
        self.focus_on_open(window, cx);
        log::debug!(
            "timing: page content shown {:?} after launch",
            crate::logger::started().elapsed()
        );
        // A page that had no blocks just got its first one; save it.
        self.save_edits(cx);
        cx.notify();
    }

    /// Whether the page shows a document with its first screen ready, for tests.
    #[cfg(test)]
    pub(crate) fn is_first_screen_ready(&self, cx: &gpui::App) -> bool {
        self.editor()
            .is_some_and(|editor| is_first_screen_ready(editor.read(cx)))
    }

    pub(super) fn editor(&self) -> Option<&Entity<PageEditor>> {
        match &self.body {
            Body::Document(editor) => Some(editor),
            _ => None,
        }
    }

    pub(super) fn save_edits(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else {
            return;
        };
        let updates = editor.update(cx, |editor, _| editor.take_outgoing_updates());
        if updates.is_empty() {
            return;
        }
        let page = self.page;
        let workspace = self.workspace.read(cx);
        workspace.write("save page content", move |store| {
            storage::record_updates(store, page, &updates)
        });
        if workspace.is_linked() {
            return;
        }
        self.compaction = cx.spawn(async move |view, cx| {
            cx.background_executor().timer(COMPACTION_DELAY).await;
            if let Err(error) = view.update(cx, |view, cx| view.compact_now(cx)) {
                log::warn!("page closed before compaction: {error}");
            }
        });
    }

    /// Folds a local workspace's saved edits into a snapshot. In a linked workspace the sync
    /// engine owns the stored log and compacts it when the page closes.
    pub fn compact_now(&mut self, cx: &mut gpui::App) {
        let workspace = self.workspace.read(cx);
        if workspace.is_linked() || !matches!(self.body, Body::Document(_)) {
            return;
        }
        let page = self.page;
        workspace.write("compact page content", move |store| {
            storage::compact(store, page)
        });
    }

    pub(super) fn close(&mut self, cx: &mut gpui::App) {
        self.compact_now(cx);
        let page = self.page;
        self.workspace
            .update(cx, |workspace, _| workspace.close_page(page));
    }

    pub(super) fn create_database(&mut self, cx: &mut Context<Self>) {
        let command = PageCommand::NewDatabase {
            parent: Some(self.page),
        };
        let creating = commands::run(&command, &self.workspace, cx);
        cx.spawn(async move |view, cx| {
            if let Some(page) = creating.await
                && let Err(error) = view.update(cx, |_, cx| cx.emit(PageViewEvent::OpenPage(page)))
            {
                log::debug!("page closed before its new database opened: {error}");
            }
        })
        .detach();
    }
}

/// Whether what the page's first screen shows is ready: its diagrams drawn and its code
/// grammars built, so they neither pop in nor hold up its first frames.
fn is_first_screen_ready(editor: &PageEditor) -> bool {
    editor.are_first_screen_diagrams_drawn() && editor.are_first_screen_grammars_built()
}
