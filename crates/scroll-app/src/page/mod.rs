//! One open page: its editable title, then either its block editor or, for a page holding a
//! database, the database's views. A database row's page also shows its property values.
//!
//! The page's document loads in the background; every local edit is saved as it happens. In
//! a local workspace the saved edits are folded into a snapshot shortly after typing stops;
//! in a linked one the sync engine does that when the page closes, and this view imports
//! what others write and shows where they are.

pub mod commands;
mod document;
pub mod icon;
mod presence;
pub mod row_properties;
pub mod tags;

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AnyView, AppContext, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div,
};
use scroll_core::id::{PageId, UserId};
use scroll_sync::ConnectionId;
use scroll_sync::ConnectionStatus;
use scroll_ui::{InputAccess, TextInput, TextInputEvent, Theme};

use crate::database::{DatabaseView, DatabaseViewEvent};
use crate::editor::document::{PageDocument, StableCursor};
use crate::editor::{Collaborator, PageAccess, PageEditor, PageEditorEvent, Selection};
use crate::page::icon::{PAGE_HEADER_GROUP, PageIcon};
use crate::page::row_properties::RowProperties;
use crate::page::tags::PageTags;
use crate::page_layout::PAGE_MARGIN;
use crate::workspace::presence::PresenceState;
use crate::workspace::storage::{self, StorageError};
use crate::workspace::{PageSyncEvent, Workspace, WorkspaceEvent};
use commands::{ChangeIcon, PageCommand};

/// How long after the last edit the page's saved updates are folded into a snapshot, which
/// also refreshes its text in search.
const COMPACTION_DELAY: Duration = Duration::from_millis(1500);
/// How long an empty page in a linked workspace waits for its content before it is shown empty.
const CONTENT_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageViewEvent {
    OpenPage(PageId),
    /// The page was deleted or never existed (an old link or setting).
    Missing,
}

/// Someone else on this page, as their presence last said.
#[derive(Clone, Debug)]
pub struct Person {
    pub user: UserId,
    pub name: SharedString,
    pub color_index: usize,
}

enum Body {
    Loading,
    Document(Entity<PageEditor>),
    Database(Entity<DatabaseView>),
    Failed(String),
}

pub struct PageView {
    page: PageId,
    workspace: Entity<Workspace>,
    title: Entity<TextInput>,
    icon: Entity<PageIcon>,
    tags: Entity<PageTags>,
    properties: Option<Entity<RowProperties>>,
    /// Icon, title, tags and a row's properties, above a document's blocks or a database.
    header: Option<Entity<PageHeader>>,
    body: Body,
    /// Remote updates that arrived before the document finished loading.
    early_updates: Vec<Vec<u8>>,
    /// An empty document held back while its content arrives from the server.
    pending_document: Option<PageDocument>,
    pending_timeout: Task<()>,
    access: PageAccess,
    people: HashMap<ConnectionId, Person>,
    /// Shown on this person's caret on other devices.
    my_name: Option<String>,
    compaction: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PageViewEvent> for PageView {}

/// The title (and a row's properties), shown above the first block and scrolled with the page.
struct PageHeader {
    icon: Entity<PageIcon>,
    title: Entity<TextInput>,
    tags: Entity<PageTags>,
    properties: Option<Entity<RowProperties>>,
}

impl Render for PageHeader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div()
            .group(PAGE_HEADER_GROUP)
            .pt(PAGE_MARGIN)
            .pb(theme.spacing.lg)
            .child(self.icon.clone())
            .child(self.title.clone())
            .child(self.tags.clone())
            .children(
                self.properties
                    .clone()
                    .map(|properties| div().pt(theme.spacing.md).child(properties)),
            )
    }
}

impl PageView {
    pub fn new(
        page: PageId,
        workspace: Entity<Workspace>,
        my_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = Theme::current(window, cx);
        let title_text = workspace
            .read(cx)
            .tree()
            .page(page)
            .map(|page| page.title.clone())
            .unwrap_or_default();
        let title = cx.new(|cx| {
            TextInput::new(cx)
                .plain()
                .text_size(theme.text.title)
                .font_weight(FontWeight::BOLD)
                .placeholder("Untitled")
                .wrapping()
                .with_text(title_text)
        });
        let subscriptions = vec![
            cx.subscribe_in(&title, window, Self::title_changed),
            cx.subscribe_in(&workspace, window, Self::workspace_changed),
            cx.on_release(|view, cx| view.close(cx)),
        ];
        let icon = {
            let workspace = workspace.clone();
            cx.new(|cx| PageIcon::new(page, workspace, cx))
        };
        let tags = {
            let workspace = workspace.clone();
            cx.new(|cx| PageTags::new(page, workspace, cx))
        };
        // What kind of page this is (document, database, row) is known once its records load.
        let loading = workspace.update(cx, |workspace, cx| {
            workspace.open_page(page);
            workspace.load_page(page, cx)
        });
        cx.spawn_in(window, async move |view, cx| {
            let exists = loading.await;
            let shown = view.update_in(cx, |view, window, cx| {
                if exists {
                    view.show_kind(window, cx)
                } else {
                    view.show_missing(cx)
                }
            });
            if let Err(error) = shown {
                log::debug!("page closed before its records loaded: {error}");
            }
        })
        .detach();
        let body = Body::Loading;
        let access = if workspace.read(cx).is_read_only() {
            PageAccess::ReadOnly
        } else {
            PageAccess::Edit
        };
        title.update(cx, |title, cx| title.set_access(input_access(access), cx));
        PageView {
            page,
            workspace,
            title,
            icon,
            tags,
            properties: None,
            header: None,
            body,
            early_updates: Vec::new(),
            pending_document: None,
            pending_timeout: Task::ready(()),
            access,
            people: HashMap::new(),
            my_name,
            compaction: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn access(&self) -> PageAccess {
        self.access
    }

    pub fn page(&self) -> PageId {
        self.page
    }

    fn show_missing(&mut self, cx: &mut Context<Self>) {
        self.body = Body::Failed("This page no longer exists.".into());
        cx.emit(PageViewEvent::Missing);
        cx.notify();
    }

    /// Sets the page up as a database, a row with properties, or a document, now that its
    /// records are loaded.
    fn show_kind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_title(window, cx);
        let records = self.workspace.read(cx).records();
        let row_of = records.tree.page(self.page).and_then(|page| page.row_of);
        let hosted_database = records
            .database_hosted_by(self.page)
            .map(|database| database.id);
        let page = self.page;
        self.properties = row_of.map(|database| {
            let workspace = self.workspace.clone();
            cx.new(|cx| RowProperties::new(page, database, workspace, window, cx))
        });
        let Some(database) = hosted_database else {
            return Self::load_document(page, &self.workspace, window, cx);
        };
        self.header(cx);
        let view = cx.new(|cx| DatabaseView::new(database, self.workspace.clone(), window, cx));
        self._subscriptions
            .push(
                cx.subscribe(&view, |_, _, event: &DatabaseViewEvent, cx| match event {
                    DatabaseViewEvent::OpenPage(page) => cx.emit(PageViewEvent::OpenPage(*page)),
                }),
            );
        if self.title.read(cx).text().is_empty() {
            window.focus(&self.title.read(cx).focus_handle(cx));
        } else {
            view.update(cx, |view, cx| view.focus_table(window, cx));
        }
        self.body = Body::Database(view);
        log::debug!(
            "timing: database shown {:?} after launch",
            crate::logger::started().elapsed()
        );
        cx.notify();
    }

    fn header(&mut self, cx: &mut Context<Self>) -> Entity<PageHeader> {
        if let Some(header) = self.header.as_ref() {
            return header.clone();
        }
        let header = cx.new(|_| PageHeader {
            icon: self.icon.clone(),
            title: self.title.clone(),
            tags: self.tags.clone(),
            properties: self.properties.clone(),
        });
        self.header = Some(header.clone());
        header
    }

    fn title_changed(
        &mut self,
        title: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {
                if self.access == PageAccess::ReadOnly {
                    // A viewer's typing is not kept: put the stored title back.
                    return self.follow_title(window, cx);
                }
                let text = title.read(cx).text().to_string();
                let page = self.page;
                self.workspace
                    .update(cx, |workspace, cx| workspace.rename(page, &text, cx));
            }
            TextInputEvent::Submitted => self.focus_content(window, cx),
            TextInputEvent::Cancelled => {}
        }
    }

    fn workspace_changed(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspaceEvent::RecordsChanged(_) => self.follow_title(window, cx),
            WorkspaceEvent::Page(page, event) if *page == self.page => {
                self.page_synced(event.clone(), window, cx)
            }
            WorkspaceEvent::ConnectionChanged => self.connection_changed(cx),
            _ => {}
        }
    }

    /// Keeps the title in step with renames made elsewhere, unless it is being edited.
    fn follow_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stored) = self
            .workspace
            .read(cx)
            .tree()
            .page(self.page)
            .map(|page| page.title.clone())
        else {
            return;
        };
        let title = self.title.read(cx);
        let is_editing = title.focus_handle(cx).is_focused(window);
        if !is_editing && title.text() != stored {
            self.title
                .update(cx, |title, cx| title.set_text(stored, cx));
        }
    }

    fn page_synced(&mut self, event: PageSyncEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PageSyncEvent::Updates(updates) => {
                if let Some(editor) = self.editor() {
                    return editor
                        .update(cx, |editor, cx| editor.apply_remote_updates(&updates, cx));
                }
                let Some(page) = self.pending_document.as_ref() else {
                    return self.early_updates.extend(updates);
                };
                for update in &updates {
                    if let Err(error) = page.import(update) {
                        log::error!("could not import page content from the server: {error}");
                    }
                }
                self.show_pending_document(window, cx);
            }
            PageSyncEvent::EditsRefused => {
                // Whatever this device wrote was not kept; show the page as stored.
                self.body = Body::Loading;
                Self::load_document(self.page, &self.workspace, window, cx);
                cx.notify();
            }
            PageSyncEvent::Presence {
                user,
                connection,
                state,
            } => self.show_person(user, connection, &state, cx),
            PageSyncEvent::PresenceLeft { connection } => {
                self.people.remove(&connection);
                if let Some(editor) = self.editor() {
                    editor.update(cx, |editor, cx| {
                        editor.set_collaborator(connection.0.as_u128(), None, cx)
                    });
                }
                cx.notify();
            }
        }
    }

    /// Back online: say where this person is. Gone: forget everyone, who will be re-announced.
    fn connection_changed(&mut self, cx: &mut Context<Self>) {
        let access = if self.workspace.read(cx).is_read_only() {
            PageAccess::ReadOnly
        } else {
            PageAccess::Edit
        };
        if access != self.access {
            self.access = access;
            if let Some(editor) = self.editor() {
                editor.update(cx, |editor, cx| editor.set_access(access, cx));
            }
            self.title
                .update(cx, |title, cx| title.set_access(input_access(access), cx));
        }
        if self.workspace.read(cx).connection() == Some(ConnectionStatus::Online) {
            return self.send_presence(cx);
        }
        let connections: Vec<ConnectionId> = self
            .people
            .drain()
            .map(|(connection, _)| connection)
            .collect();
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| {
                for connection in connections {
                    editor.set_collaborator(connection.0.as_u128(), None, cx);
                }
            });
        }
        cx.notify();
    }

    /// Opens the icon picker under the page's icon, for the keyboard and the palette.
    pub fn change_icon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.icon
            .update(cx, |icon, cx| icon.open_picker_below_icon(window, cx));
    }

    /// Carries out a command meant for the open page.
    pub fn run(&mut self, action: &dyn gpui::Action, window: &mut Window, cx: &mut Context<Self>) {
        match action.as_any().downcast_ref::<ChangeIcon>() {
            Some(ChangeIcon) => self.change_icon(window, cx),
            None => log::warn!("the page has no handler for {}", action.name()),
        }
    }

    pub fn focus_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title.clone();
        window.focus(&title.read(cx).focus_handle(cx));
        title.update(cx, |title, cx| title.select_all_text(cx));
    }

    pub fn database_view(&self) -> Option<Entity<DatabaseView>> {
        match &self.body {
            Body::Database(database) => Some(database.clone()),
            _ => None,
        }
    }

    pub fn focus_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.body {
            Body::Document(editor) => {
                editor.update(cx, |editor, cx| editor.focus_start(window, cx))
            }
            Body::Database(database) => {
                database.update(cx, |database, cx| database.focus_table(window, cx))
            }
            Body::Loading | Body::Failed(_) => {}
        }
    }
}

impl Render for PageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let content = match &self.body {
            Body::Document(editor) => editor.clone().into_any_element(),
            Body::Database(database) => div()
                .flex()
                .flex_col()
                .size_full()
                .child(div().px(PAGE_MARGIN).children(self.header.clone()))
                .child(div().flex_1().min_h_0().child(database.clone()))
                .into_any_element(),
            Body::Failed(error) => div()
                .p(PAGE_MARGIN)
                .text_color(theme.colors.danger)
                .child(format!("This page could not be opened: {error}"))
                .into_any_element(),
            Body::Loading => div().into_any_element(),
        };
        div()
            .size_full()
            .on_action(
                cx.listener(|view, _: &scroll_ui::actions::MoveDown, window, cx| {
                    view.focus_content(window, cx)
                }),
            )
            .child(content)
    }
}

fn input_access(access: PageAccess) -> InputAccess {
    match access {
        PageAccess::Edit => InputAccess::Editable,
        PageAccess::ReadOnly => InputAccess::ReadOnly,
    }
}
