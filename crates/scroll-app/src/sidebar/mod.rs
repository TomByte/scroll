//! The page tree: navigation, create, rename in place, drag to reorder or nest, context
//! menu, and the trash. All tree logic lives in `workspace::page_tree`; this view only maps
//! pointer and keyboard input onto it.

mod resize;
mod rows;
mod tags;
mod trash;

use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, UniformListScrollHandle, Window, div, prelude::FluentBuilder,
    uniform_list,
};
use scroll_core::id::{PageId, TagId};
use scroll_core::record::Page;
use scroll_ui::{
    EmptyState, Icon, IconName, IconSize, ListItem, Menu, MenuItem, PopupAnchor, PopupHost,
    ResizeHandle, Scrollbar, SectionHeader, TextInput, TextInputEvent, Theme, TitlebarDragArea,
    Transition,
};

use crate::accounts::switcher::{current_workspace_name, switcher_menu};
use crate::commands::{Command, hint};
use crate::overlays::Overlays;
use crate::page::commands::{NEW_DATABASE, NEW_PAGE};
use crate::palette::commands::OPEN_PALETTE;
use crate::sync_status::empty_text;
use crate::title_bar;
use crate::workspace::page_tree::{PageDrop, TreeRow};
use crate::workspace::storage::CHILDREN_BATCH;
use crate::workspace::{Workspace, WorkspaceEvent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    OpenPage(PageId),
    ExpandedChanged,
    /// Runs command `id` for a page (`None` is the open page), e.g. from the page menu.
    RunCommand(&'static str, Option<PageId>),
    /// The person finished dragging the sidebar to this width.
    Resized(Pixels),
}

#[derive(Clone)]
struct DraggedPage {
    id: PageId,
    title: SharedString,
}

struct Renaming {
    page: PageId,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

pub struct Sidebar {
    /// Every command: the page menu offers some, and the buttons show others' shortcuts.
    commands: &'static [&'static Command],
    overlays: Overlays,
    workspace: Entity<Workspace>,
    expanded: HashSet<PageId>,
    /// The parent last opened by hand, whose rows fade in as they appear.
    revealed: Option<PageId>,
    current_page: Option<PageId>,
    renaming: Option<Renaming>,
    popups: Entity<PopupHost>,
    drop: Option<PageDrop>,
    is_trash_open: bool,
    /// Tags picked in the Tags section; the pages carrying all of them are listed.
    selected_tags: Vec<TagId>,
    /// The pages carrying every selected tag, read from the store (at most `TAGGED_LIMIT`).
    tagged_pages: Vec<Page>,
    /// Archived pages, read when the trash is opened.
    trash: Vec<Page>,
    /// The visible tree rows, rebuilt only when the tree or expansion changes.
    rows: Option<Rc<Vec<TreeRow>>>,
    /// Expanded parents whose children have been asked for.
    requested: HashSet<PageId>,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    width: Pixels,
    resizing: Option<resize::Resizing>,
    _subscription: Subscription,
}

/// The most tagged pages the sidebar lists; the palette's `#` mode finds the rest.
const TAGGED_LIMIT: usize = 200;
/// The tag list scrolls past this, leaving the page tree most of the height.
const TAG_LIST_MAX_HEIGHT: Pixels = gpui::px(248.0);

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Focusable for Sidebar {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Sidebar {
    pub fn new(
        workspace: Entity<Workspace>,
        expanded: HashSet<PageId>,
        commands: &'static [&'static Command],
        overlays: Overlays,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription =
            cx.subscribe(
                &workspace,
                |sidebar, _, event: &WorkspaceEvent, cx| match event {
                    WorkspaceEvent::RecordsChanged(_) | WorkspaceEvent::Loaded => {
                        sidebar.rows = None;
                        // An opened page's ancestors may have just loaded: expand down to it.
                        sidebar.expand_to_current_page(cx);
                        sidebar.load_expanded_children(cx);
                        cx.notify();
                    }
                    _ => {}
                },
            );
        let mut sidebar = Sidebar {
            commands,
            overlays,
            workspace,
            expanded,
            revealed: None,
            current_page: None,
            renaming: None,
            popups: cx.new(|_| PopupHost::new()),
            drop: None,
            is_trash_open: false,
            selected_tags: Vec::new(),
            tagged_pages: Vec::new(),
            trash: Vec::new(),
            rows: None,
            requested: HashSet::new(),
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            width: resize::DEFAULT_WIDTH,
            resizing: None,
            _subscription: subscription,
        };
        sidebar.load_expanded_children(cx);
        // Launch read only the first batch of the top level; the rest follows in batches.
        let is_top_level_partial =
            sidebar.workspace.read(cx).tree().children(None).len() >= CHILDREN_BATCH;
        if is_top_level_partial {
            sidebar
                .workspace
                .update(cx, |workspace, cx| workspace.load_children(None, cx))
                .detach();
        }
        sidebar
    }

    /// Asks for the children of every expanded page that is loaded and has none loaded yet,
    /// so the tree restored from settings fills in one level at a time.
    fn load_expanded_children(&mut self, cx: &mut Context<Self>) {
        let tree = self.workspace.read(cx).tree();
        let wanted: Vec<PageId> = self
            .expanded
            .iter()
            .copied()
            .filter(|page| tree.page(*page).is_some() && !tree.are_children_loaded(Some(*page)))
            .filter(|page| !self.requested.contains(page))
            .collect();
        for page in wanted {
            self.requested.insert(page);
            self.workspace
                .update(cx, |workspace, cx| workspace.load_children(Some(page), cx))
                .detach();
        }
    }

    fn rows(&mut self, cx: &App) -> Rc<Vec<TreeRow>> {
        if let Some(rows) = self.rows.as_ref() {
            return rows.clone();
        }
        let rows = Rc::new(self.workspace.read(cx).tree().visible_rows(&self.expanded));
        self.rows = Some(rows.clone());
        rows
    }

    /// Starts at `width` (as the person last left it), within the sidebar's limits.
    pub fn with_width(mut self, width: Pixels) -> Self {
        self.width = resize::clamp_width(width);
        self
    }

    pub fn width(&self) -> Pixels {
        self.width
    }

    pub fn expanded(&self) -> &HashSet<PageId> {
        &self.expanded
    }

    /// Marks `page` as open and expands its ancestors so it is visible.
    pub fn set_current_page(&mut self, page: Option<PageId>, cx: &mut Context<Self>) {
        self.current_page = page;
        self.expand_to_current_page(cx);
        cx.notify();
    }

    /// Expands the open page's loaded ancestors, so its row shows once its parents load.
    fn expand_to_current_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.current_page else {
            return;
        };
        let ancestors = self.workspace.read(cx).tree().ancestors_and_self(page);
        let mut is_newly_expanded = false;
        for ancestor in ancestors.iter().skip(1) {
            is_newly_expanded |= self.expanded.insert(*ancestor);
        }
        if is_newly_expanded {
            self.rows = None;
            self.load_expanded_children(cx);
            cx.emit(SidebarEvent::ExpandedChanged);
        }
    }

    fn toggle_expanded(&mut self, page: PageId, cx: &mut Context<Self>) {
        self.revealed = None;
        if !self.expanded.remove(&page) {
            self.expanded.insert(page);
            self.revealed = Some(page);
            self.load_expanded_children(cx);
        }
        self.rows = None;
        cx.emit(SidebarEvent::ExpandedChanged);
        cx.notify();
    }

    /// Whether `page` is one of the rows that appeared when `parent` opened.
    fn is_revealed_by(&self, page: PageId, parent: PageId, cx: &App) -> bool {
        let ancestors = self.workspace.read(cx).tree().ancestors_and_self(page);
        ancestors.iter().skip(1).any(|ancestor| *ancestor == parent)
    }

    fn open_switcher(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let overlays = self.overlays.clone();
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| switcher_menu(overlays, cx),
                window,
                cx,
            );
        });
    }

    pub(super) fn start_renaming(
        &mut self,
        page: PageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = self
            .workspace
            .read(cx)
            .tree()
            .page(page)
            .map(|page| page.title.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx)
                .plain()
                .placeholder("Untitled")
                .with_text(title);
            input.select_all_text(cx);
            input
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |sidebar, input, event, window, cx| match event {
                TextInputEvent::Submitted => {
                    let title = input.read(cx).text().to_string();
                    sidebar
                        .workspace
                        .update(cx, |workspace, cx| workspace.rename(page, &title, cx));
                    sidebar.stop_renaming(window, cx);
                }
                TextInputEvent::Cancelled => sidebar.stop_renaming(window, cx),
                TextInputEvent::Changed => {}
            },
        );
        window.focus(&input.focus_handle(cx));
        self.renaming = Some(Renaming {
            page,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn stop_renaming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn open_context_menu(
        &mut self,
        page: PageId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let can_edit = !self.workspace.read(cx).is_read_only();
        let self_entity = cx.entity().downgrade();
        let sidebar = cx.entity().downgrade();
        let action = move |run: fn(&mut Sidebar, PageId, &mut Window, &mut Context<Sidebar>)| {
            let sidebar = sidebar.clone();
            move |window: &mut Window, cx: &mut gpui::App| {
                if let Err(error) = sidebar.update(cx, |sidebar, cx| run(sidebar, page, window, cx))
                {
                    log::warn!("sidebar closed before its menu action ran: {error}");
                }
            }
        };
        // "Open" plus every command that offers itself in the page menu, acting on this page.
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| {
                    let mut menu = Menu::new(cx).item(
                        MenuItem::new(
                            "Open",
                            action(|_, page, _, cx| cx.emit(SidebarEvent::OpenPage(page))),
                        )
                        .icon(IconName::Page),
                    );
                    let usable = |command: &&&'static Command| {
                        command.is_in_page_menu && command.is_allowed(can_edit)
                    };
                    for command in self.commands.iter().filter(usable) {
                        let owner = self_entity.clone();
                        let id = command.id;
                        menu = menu.item(
                            MenuItem::new(command.title, move |_, cx: &mut gpui::App| {
                                if let Err(error) = owner.update(cx, |_, cx| {
                                    cx.emit(SidebarEvent::RunCommand(id, Some(page)))
                                }) {
                                    log::debug!(
                                        "sidebar closed before its menu action ran: {error}"
                                    );
                                }
                            })
                            .icon(command.icon),
                        );
                    }
                    menu
                },
                window,
                cx,
            );
        });
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let can_edit = !self.workspace.read(cx).is_read_only();
        let rows = self.rows(cx);
        let row_count = rows.len();
        let list_theme = theme.clone();
        let reveal = Transition::Reveal.progress("page-tree-reveal", self.revealed, window, cx);
        let revealing = self.revealed.filter(|_| reveal < 1.0);
        let trash = self.render_trash(&theme, cx);
        let tag_rows = self.render_tags(&theme, cx);
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .flex_none()
            .h_full()
            .w(self.width)
            .bg(theme.colors.surface_secondary)
            .border_r(theme.size.border_width)
            .border_color(theme.colors.border)
            .text_size(theme.text.ui.size)
            // Leaves room for the window's traffic lights, which sit over the sidebar; the strip
            // moves the window, as a title bar would.
            .pt(title_bar::height(&theme))
            .child(
                TitlebarDragArea::new()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(title_bar::height(&theme)),
            )
            .px(theme.spacing.sm)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing.xxs)
                    .pb(theme.spacing.md)
                    .child(
                        ListItem::new("workspace-switcher", current_workspace_name(cx))
                            .emphasized()
                            .end_slot(Icon::new(IconName::ChevronDown).size(IconSize::Small))
                            .on_click(cx.listener(
                                |sidebar, event: &gpui::ClickEvent, window, cx| {
                                    sidebar.open_switcher(event.position(), window, cx)
                                },
                            )),
                    )
                    .child(
                        ListItem::new("search", "Search")
                            .icon(IconName::Search)
                            .end_slot(hint(self.commands, OPEN_PALETTE))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SidebarEvent::RunCommand(OPEN_PALETTE, None))
                            })),
                    )
                    .when(can_edit, |actions| {
                        actions
                            .child(
                                ListItem::new("new-page", "New page")
                                    .icon(IconName::Plus)
                                    .end_slot(hint(self.commands, NEW_PAGE))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(SidebarEvent::RunCommand(NEW_PAGE, None))
                                    })),
                            )
                            .child(
                                ListItem::new("new-database", "New database")
                                    .icon(IconName::Table)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(SidebarEvent::RunCommand(NEW_DATABASE, None))
                                    })),
                            )
                    }),
            )
            .child(SectionHeader::new("Pages"))
            .child(
                // Only the rows on screen are built, so a parent with 50,000 children
                // scrolls, expands and renames at the cost of the rows in view.
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(row_count == 0, |tree| {
                        tree.child(
                            EmptyState::new(empty_text(
                                &self.workspace.read(cx).sync_state(),
                                "No pages yet",
                            ))
                            .inline(),
                        )
                    })
                    .child(
                        uniform_list(
                            "page-tree",
                            row_count,
                            cx.processor(move |sidebar, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .filter_map(|index| rows.get(index).copied())
                                    .map(|row| {
                                        let is_revealing = revealing.is_some_and(|parent| {
                                            sidebar.is_revealed_by(row.id, parent, cx)
                                        });
                                        sidebar
                                            .render_row(&row, &list_theme, cx)
                                            .when(is_revealing, |row| row.opacity(reveal))
                                            .into_any_element()
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(self.scroll.clone())
                        .size_full(),
                    )
                    .child(Scrollbar::new("page-tree-scrollbar", self.scroll.clone())),
            )
            .child(
                div()
                    .id("tag-list")
                    .flex_none()
                    .max_h(TAG_LIST_MAX_HEIGHT)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(tag_rows),
            )
            .child(div().py(theme.spacing.sm).child(trash))
            .child(self.popups.clone())
            .child(ResizeHandle::new("sidebar-resize").on_press(cx.listener(
                |sidebar, event: &gpui::MouseDownEvent, _, cx| sidebar.start_resizing(event, cx),
            )))
            .when(self.resizing.is_some(), |sidebar| {
                sidebar.child(self.resize_tracker(cx))
            })
    }
}
