//! A database page's body: tabs for its views, and the selected view's table.

pub mod commands;

use crate::page_layout::PAGE_MARGIN;
use gpui::{
    AppContext, Context, Entity, EventEmitter, Focusable, IntoElement, ParentElement, Pixels,
    Point, Render, Styled, Subscription, Window, div, prelude::FluentBuilder,
};
use scroll_core::id::{DatabaseId, PageId, PropertyId, ViewId};
use scroll_core::operation::{RecordId, Transaction};
use scroll_table::{TableAccess, TableEvent, TableView};
use scroll_ui::{
    Button, ButtonSize, ButtonVariant, Divider, IconButton, IconName, Menu, MenuItem, PopupAnchor,
    PopupHost, Tabs, TextInput, TextInputEvent, Theme,
};

use crate::workspace::{ChangeSource, Workspace, WorkspaceEvent};
use commands::DatabaseCommand;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatabaseViewEvent {
    OpenPage(PageId),
}

struct Renaming {
    view: ViewId,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

pub struct DatabaseView {
    database: DatabaseId,
    workspace: Entity<Workspace>,
    current_view: Option<ViewId>,
    table: Option<Entity<TableView>>,
    renaming: Option<Renaming>,
    popups: Entity<PopupHost>,
    /// Focus the table as soon as it exists.
    wants_focus: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DatabaseViewEvent> for DatabaseView {}

impl DatabaseView {
    pub fn new(
        database: DatabaseId,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe_in(&workspace, window, Self::workspace_changed),
            cx.on_release(|view, cx| {
                let database = view.database;
                view.workspace
                    .update(cx, |workspace, _| workspace.close_database(database));
            }),
        ];
        workspace.update(cx, |workspace, _| workspace.open_database(database));
        let mut view = DatabaseView {
            database,
            workspace,
            current_view: None,
            table: None,
            renaming: None,
            popups: cx.new(|_| PopupHost::new()),
            wants_focus: false,
            _subscriptions: subscriptions,
        };
        // A database created a moment ago may not have reached the table's connection yet.
        let first_view = view.view_ids(cx).first().copied();
        if let Some(first) = first_view {
            view.flush_and_show(first, window, cx);
        }
        view
    }

    pub fn open_sort_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(table) = self.table.clone() {
            table.update(cx, |table, cx| table.open_sort_editor(window, cx));
        }
    }

    pub fn open_filter_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(table) = self.table.clone() {
            table.update(cx, |table, cx| table.open_filter_editor(window, cx));
        }
    }

    pub fn open_properties_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(table) = self.table.clone() {
            table.update(cx, |table, cx| table.open_properties_panel(window, cx));
        }
    }

    /// The property's menu: rename, sort, hide, change type, delete.
    pub fn open_column_menu(
        &mut self,
        property: PropertyId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(table) = self.table.clone() {
            table.update(cx, |table, cx| table.open_column_menu(property, window, cx));
        }
    }

    /// Carries out a database command.
    pub fn run(&mut self, action: &dyn gpui::Action, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = action.as_any().downcast_ref::<DatabaseCommand>() else {
            return log::warn!("the database has no handler for {}", action.name());
        };
        match *command {
            DatabaseCommand::AddView => self.add_view(window, cx),
            DatabaseCommand::AddRow => self.add_row(window, cx),
            DatabaseCommand::Sort => self.open_sort_editor(window, cx),
            DatabaseCommand::Filter => self.open_filter_editor(window, cx),
            DatabaseCommand::Properties => self.open_properties_panel(window, cx),
            DatabaseCommand::EditProperty(property) => self.open_column_menu(property, window, cx),
            DatabaseCommand::SwitchView(view) => self.show_view(view, window, cx),
            DatabaseCommand::RenameView(view) => self.start_renaming(view, window, cx),
            DatabaseCommand::DeleteView(view) => self.delete_view_unless_last(view, cx),
        }
    }

    /// Deletes `view` unless it is the last one; a database always keeps a view.
    fn delete_view_unless_last(&mut self, view: ViewId, cx: &mut Context<Self>) {
        if self.view_ids(cx).len() > 1 {
            self.delete_view(view, cx);
        }
    }

    pub fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(table) = self.table.clone() {
            table.update(cx, |table, cx| table.add_row(window, cx));
        }
    }

    #[cfg(test)]
    pub(crate) fn table(&self) -> Option<&Entity<TableView>> {
        self.table.as_ref()
    }

    pub fn has_table(&self) -> bool {
        self.table.is_some()
    }

    fn table_access(&self, cx: &gpui::App) -> TableAccess {
        if self.workspace.read(cx).is_read_only() {
            TableAccess::ReadOnly
        } else {
            TableAccess::Editable
        }
    }

    /// Moves keyboard focus into the table, now or once it has opened.
    pub fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.table.as_ref() {
            Some(table) => window.focus(&table.focus_handle(cx)),
            None => self.wants_focus = true,
        }
    }

    fn view_ids(&self, cx: &gpui::App) -> Vec<ViewId> {
        self.workspace
            .read(cx)
            .records()
            .views_of(self.database)
            .iter()
            .map(|view| view.id)
            .collect()
    }

    fn show_view(&mut self, view: ViewId, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_view == Some(view) && self.table.is_some() {
            return;
        }
        self.current_view = Some(view);
        let path = self.workspace.read(cx).path().clone();
        let table = cx.new(|cx| TableView::new(path, view, cx));
        let access = self.table_access(cx);
        table.update(cx, |table, cx| table.set_access(access, window, cx));
        let workspace = self.workspace.clone();
        self._subscriptions
            .push(
                cx.subscribe(&table, move |_, _, event: &TableEvent, cx| match event {
                    TableEvent::OpenPage(page) => cx.emit(DatabaseViewEvent::OpenPage(*page)),
                    // The table writes through its own connections; the rest of the app catches up.
                    TableEvent::Changed => {
                        workspace.update(cx, |workspace, cx| workspace.table_changed(cx))
                    }
                }),
            );
        if self.wants_focus {
            self.wants_focus = false;
            window.focus(&table.focus_handle(cx));
        }
        self.table = Some(table);
        cx.notify();
    }

    /// Records changed (here or on the server): keep tabs current and refresh the table.
    fn workspace_changed(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, WorkspaceEvent::ConnectionChanged) {
            // The person's role may have changed: a viewer's table stops accepting edits.
            let access = self.table_access(cx);
            if let Some(table) = self.table.clone() {
                table.update(cx, |table, cx| table.set_access(access, window, cx));
            }
            return;
        }
        let WorkspaceEvent::RecordsChanged(source) = event else {
            return;
        };
        // The table already shows its own changes; anything else may change its rows.
        if *source != ChangeSource::Table {
            self.refresh_table(cx);
        }
        let views = self.view_ids(cx);
        let is_current_gone = self
            .current_view
            .is_none_or(|current| !views.contains(&current));
        if is_current_gone {
            self.table = None;
            if let Some(first) = views.first().copied() {
                self.show_view(first, window, cx);
            }
            return;
        }
        if self.table.is_none()
            && let Some(current) = self.current_view
        {
            self.show_view(current, window, cx);
        }
        cx.notify();
    }

    /// Refreshes the table once changes queued so far have reached the file it reads.
    fn refresh_table(&mut self, cx: &mut Context<Self>) {
        let Some(table) = self.table.as_ref().map(|table| table.downgrade()) else {
            return;
        };
        let flushed = self.workspace.read(cx).flushed(cx);
        cx.spawn(async move |_, cx| {
            if let Err(error) = flushed.await {
                log::error!("changes did not reach the file before the table refreshed: {error}");
            }
            if let Err(error) = table.update(cx, |table, cx| table.refresh(cx)) {
                log::debug!("table closed before it refreshed: {error}");
            }
        })
        .detach();
    }

    pub fn add_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let name = format!("Table {}", self.view_ids(cx).len() + 1);
        let database = self.database;
        let created = self
            .workspace
            .update(cx, |workspace, cx| workspace.add_view(database, &name, cx));
        if let Some(view) = created {
            self.flush_and_show(view, window, cx);
        }
    }

    /// The table reads through its own connection, so it opens a new view only once the
    /// write creating it has landed.
    fn flush_and_show(&mut self, view: ViewId, window: &mut Window, cx: &mut Context<Self>) {
        self.current_view = Some(view);
        self.table = None;
        let flushed = self.workspace.read(cx).flushed(cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = flushed.await {
                log::error!("could not save the new view: {error}");
            }
            let shown = this.update_in(cx, |this, window, cx| {
                this.current_view = None;
                this.show_view(view, window, cx);
            });
            if let Err(error) = shown {
                log::debug!("database closed before its new view opened: {error}");
            }
        })
        .detach();
        cx.notify();
    }

    fn open_tab_menu(
        &mut self,
        view: ViewId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let this = cx.entity().downgrade();
        let can_delete = self.view_ids(cx).len() > 1;
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| {
                    let rename = {
                        let this = this.clone();
                        move |window: &mut Window, cx: &mut gpui::App| {
                            if let Err(error) =
                                this.update(cx, |this, cx| this.start_renaming(view, window, cx))
                            {
                                log::debug!("database closed before renaming: {error}");
                            }
                        }
                    };
                    let delete = move |_: &mut Window, cx: &mut gpui::App| {
                        if let Err(error) = this.update(cx, |this, cx| this.delete_view(view, cx)) {
                            log::debug!("database closed before deleting a view: {error}");
                        }
                    };
                    let menu = Menu::new(cx)
                        .item(MenuItem::new("Rename view", rename).icon(IconName::Pencil));
                    if can_delete {
                        menu.item(MenuItem::new("Delete view", delete).icon(IconName::Trash))
                    } else {
                        menu
                    }
                },
                window,
                cx,
            );
        });
    }

    fn start_renaming(&mut self, view: ViewId, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let name = self
            .workspace
            .read(cx)
            .records()
            .views_of(self.database)
            .iter()
            .find(|candidate| candidate.id == view)
            .map(|view| view.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx).plain().with_text(name);
            input.select_all_text(cx);
            input
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                move |this, input, event, _, cx| match event {
                    TextInputEvent::Submitted => {
                        let name = input.read(cx).text().trim().to_string();
                        if !name.is_empty() {
                            this.workspace.update(cx, |workspace, cx| {
                                workspace.commit(Transaction::rename_view(view, &name), cx)
                            });
                        }
                        this.renaming = None;
                        cx.notify();
                    }
                    TextInputEvent::Cancelled => {
                        this.renaming = None;
                        cx.notify();
                    }
                    TextInputEvent::Changed => {}
                },
            );
        window.focus(&input.focus_handle(cx));
        self.renaming = Some(Renaming {
            view,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn delete_view(&mut self, view: ViewId, cx: &mut Context<Self>) {
        self.workspace.update(cx, |workspace, cx| {
            workspace.commit(Transaction::delete(RecordId::View(view)), cx)
        });
    }
}

impl DatabaseView {
    fn render_tabs(&self, views: &[(ViewId, String)], cx: &mut Context<Self>) -> Tabs {
        let ids: Vec<ViewId> = views.iter().map(|(view, _)| *view).collect();
        let tabs = views
            .iter()
            .fold(Tabs::new("views"), |tabs, (_, name)| tabs.tab(name.clone()));
        let tabs = match ids.iter().position(|view| Some(*view) == self.current_view) {
            Some(index) => tabs.selected(index),
            None => tabs,
        };
        let renaming = self.renaming.as_ref().and_then(|renaming| {
            let index = ids.iter().position(|view| *view == renaming.view)?;
            Some((index, renaming.input.clone()))
        });
        let (select_ids, rename_ids, menu_ids) = (ids.clone(), ids.clone(), ids);
        tabs.when_some(renaming, |tabs, (index, input)| tabs.editing(index, input))
            .on_select(cx.listener(move |this, index: &usize, window, cx| {
                this.show_view(select_ids[*index], window, cx)
            }))
            .on_double_click(cx.listener(move |this, index: &usize, window, cx| {
                this.start_renaming(rename_ids[*index], window, cx)
            }))
            .on_secondary_mouse_down(cx.listener(
                move |this, (index, event): &(usize, gpui::MouseDownEvent), window, cx| {
                    this.open_tab_menu(menu_ids[*index], event.position, window, cx)
                },
            ))
    }
}

impl Render for DatabaseView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let views: Vec<(ViewId, String)> = self
            .workspace
            .read(cx)
            .records()
            .views_of(self.database)
            .iter()
            .map(|view| (view.id, view.name.clone()))
            .collect();
        // The table names its view in its own toolbar, so tabs appear only once there is a
        // second view to switch to.
        let has_several_views = views.len() > 1;
        let tabs = has_several_views.then(|| self.render_tabs(&views, cx));
        let body = match &self.table {
            Some(table) => div()
                .relative()
                .size_full()
                .child(table.clone())
                .into_any_element(),
            None => div().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.spacing.xxs)
                    .px(PAGE_MARGIN)
                    .pb(theme.spacing.sm)
                    .children(tabs)
                    .when(!self.workspace.read(cx).is_read_only(), |tabs| {
                        tabs.child(if has_several_views {
                            IconButton::new("add-view", IconName::Plus)
                                .size(ButtonSize::Small)
                                .tooltip("Add a view")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_view(window, cx)),
                                )
                                .into_any_element()
                        } else {
                            Button::new("add-view", "Add view")
                                .icon(IconName::Plus)
                                .size(ButtonSize::Small)
                                .variant(ButtonVariant::Ghost)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_view(window, cx)),
                                )
                                .into_any_element()
                        })
                    }),
            )
            .child(Divider::horizontal())
            .child(div().flex_1().min_h_0().child(body))
            .child(self.popups.clone())
    }
}
