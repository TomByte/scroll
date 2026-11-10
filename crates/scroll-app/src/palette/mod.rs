//! The command palette (⌘P). Plain text finds pages and commands and tags; `>` lists only
//! commands, `#` only tags, `@` only pages. Commands that need something to act on (a
//! destination, a block kind, a workspace, a tag) ask for it as a second step here.
//!
//! Pages come from the search index on the storage thread, never from a scan of every page on
//! this one, so results stay instant however large the workspace; each keystroke replaces the
//! search in flight.

pub mod commands;
mod sources;

use sources::tagged_page_items;

use std::sync::Arc;

use gpui::{
    App, AppContext, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
};
use scroll_core::id::{PageId, TagId};
use scroll_ui::actions::{MoveDown, MoveUp};
use scroll_ui::{
    Divider, EmptyState, IconName, ListItem, Modal, SectionHeader, TextInput, TextInputEvent, Theme,
};

use crate::commands::{
    Choice, ChoiceValue, Choices, Command, CommandUsage, Run, Situation, Target, fuzzy_score,
    matching_commands,
};
use crate::workspace::Workspace;
use crate::workspace::page_tree::labelled_title;
use crate::workspace::storage::{self, Storage};

/// The tallest the results list grows before it scrolls.
const RESULTS_MAX_HEIGHT: gpui::Pixels = gpui::px(448.0);
/// How long typing must pause before the search index is asked.
const SEARCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(30);
/// How many pages to ask the search index for.
const PAGE_LIMIT: usize = 30;
/// How many recently visited pages to list before anything is typed.
const RECENT_LIMIT: usize = 6;
/// How many results of each kind to show.
const SECTION_LIMIT: usize = 40;
/// How many of a tag's pages to list.
const TAGGED_LIMIT: usize = 500;

pub enum PaletteEvent {
    OpenPage(PageId),
    Run {
        id: &'static str,
        target: Target,
    },
    Chose {
        id: &'static str,
        target: Target,
        value: ChoiceValue,
    },
}

#[derive(Clone)]
enum Item {
    Page {
        id: PageId,
        title: SharedString,
        snippet: Option<SharedString>,
    },
    Command(&'static Command),
    Tag {
        id: TagId,
        name: SharedString,
        count: usize,
    },
    Choice(Choice),
}

impl Item {
    fn section(&self) -> &'static str {
        match self {
            Item::Page { .. } => "Pages",
            Item::Command(_) => "Commands",
            Item::Tag { .. } => "Tags",
            Item::Choice(_) => "",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Everything,
    Commands,
    Tags,
    Pages,
}

/// Splits the prefix that picks a mode off the query.
fn mode_of(query: &str) -> (Mode, &str) {
    let trimmed = query.trim_start();
    match trimmed.chars().next() {
        Some('>') => (Mode::Commands, trimmed[1..].trim()),
        Some('#') => (Mode::Tags, trimmed[1..].trim()),
        Some('@') => (Mode::Pages, trimmed[1..].trim()),
        _ => (Mode::Everything, trimmed.trim()),
    }
}

/// The second step: what a command is waiting for, or which tag's pages are listed.
enum Step {
    Search,
    Choose {
        command: &'static Command,
        target: Target,
    },
    /// A tag's pages, read from the store when the step opens.
    TaggedPages(Vec<scroll_core::record::Page>),
}

pub struct CommandPalette {
    input: Entity<TextInput>,
    commands: &'static [&'static Command],
    workspace: Entity<Workspace>,
    storage: Arc<Storage>,
    situation: Situation,
    usage: CommandUsage,
    recent_pages: Vec<PageId>,
    target: Target,
    step: Step,
    items: Vec<Item>,
    highlighted: usize,
    scroll: ScrollHandle,
    search: Task<()>,
    tag_load: Task<()>,
    /// The answers of a command with a fixed list, read when its step began.
    fixed_choices: Vec<Choice>,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for CommandPalette {}
impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

/// What the palette is opened with.
pub struct PaletteSetup {
    /// The page open in the window, for commands that act on it.
    pub current_page: Option<PageId>,
    pub commands: &'static [&'static Command],
    pub workspace: Entity<Workspace>,
    pub situation: Situation,
    pub usage: CommandUsage,
    pub recent_pages: Vec<PageId>,
    /// Typed in already, e.g. ">" to list commands.
    pub prefix: &'static str,
    /// Set to start at a command's second step, e.g. from a page's context menu.
    pub command: Option<(&'static Command, Target)>,
}

impl CommandPalette {
    pub fn new(setup: PaletteSetup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new(cx).with_text(setup.prefix));
        let subscription =
            cx.subscribe_in(&input, window, |palette, _, event, _, cx| match event {
                TextInputEvent::Changed => palette.query_changed(cx),
                TextInputEvent::Submitted => palette.choose(palette.highlighted, cx),
                TextInputEvent::Cancelled => {}
            });
        let storage = setup.workspace.read(cx).storage().clone();
        let mut palette = CommandPalette {
            input,
            commands: setup.commands,
            workspace: setup.workspace,
            storage,
            situation: setup.situation,
            usage: setup.usage,
            recent_pages: setup.recent_pages,
            target: Target {
                page: None,
                current: setup.current_page,
            },
            step: Step::Search,
            items: Vec::new(),
            highlighted: 0,
            scroll: ScrollHandle::new(),
            search: Task::ready(()),
            tag_load: Task::ready(()),
            fixed_choices: Vec::new(),
            _subscription: subscription,
        };
        palette.set_step(Step::Search, cx);
        match setup.command {
            Some((command, target)) => palette.ask(command, target, cx),
            None => palette.query_changed(cx),
        }
        palette
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.items = match &self.step {
            Step::Search => self.search_items(&query, cx),
            Step::Choose { command, .. } => self.choice_items(command, &query, cx),
            Step::TaggedPages(pages) => tagged_page_items(pages, &query),
        };
        self.highlighted = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
        self.search_pages_in_background(&query, cx);
    }

    fn is_choosing(&self) -> bool {
        matches!(self.step, Step::Choose { .. })
    }

    /// Switches to the command's question, keeping its target.
    fn ask(&mut self, command: &'static Command, target: Target, cx: &mut Context<Self>) {
        if let Run::Choose {
            choices: Choices::Fixed(make),
            ..
        } = &command.run
        {
            self.fixed_choices = make(&target, cx);
        }
        self.set_step(Step::Choose { command, target }, cx);
        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.query_changed(cx);
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index).cloned() else {
            return;
        };
        // Events go out before dismissal: dismissing releases the palette, and events from a
        // released entity are not delivered.
        match item {
            Item::Page { id, .. } => cx.emit(PaletteEvent::OpenPage(id)),
            Item::Command(command) => {
                if matches!(command.run, Run::Choose { .. }) {
                    return self.ask(command, self.target, cx);
                }
                cx.emit(PaletteEvent::Run {
                    id: command.id,
                    target: self.target,
                });
            }
            Item::Tag { id, .. } => {
                self.set_step(Step::TaggedPages(Vec::new()), cx);
                self.input.update(cx, |input, cx| input.set_text("", cx));
                self.load_tagged_pages(id, cx);
                return self.query_changed(cx);
            }
            Item::Choice(choice) => {
                if let Step::Choose { command, target } = self.step {
                    cx.emit(PaletteEvent::Chose {
                        id: command.id,
                        target,
                        value: choice.value,
                    });
                }
            }
        }
        cx.emit(DismissEvent);
    }

    fn move_highlight(&mut self, step: isize, cx: &mut Context<Self>) {
        let last = self.items.len().saturating_sub(1) as isize;
        self.highlighted = (self.highlighted as isize + step).clamp(0, last) as usize;
        self.scroll.scroll_to_item(self.row_of(self.highlighted));
        cx.notify();
    }

    /// The child index of item `index`, counting the section headings before it.
    fn row_of(&self, index: usize) -> usize {
        let mut previous = None;
        let mut headings = 0;
        for item in self.items.iter().take(index + 1) {
            let section = item.section();
            if !section.is_empty() && previous != Some(section) {
                headings += 1;
            }
            previous = Some(section);
        }
        index + headings
    }

    fn hint(&self) -> String {
        match &self.step {
            Step::Search => "> commands  ·  # tags  ·  @ pages  ·  ↵ open  ·  esc close".into(),
            Step::Choose { command, .. } => match &command.run {
                Run::Choose { prompt, .. } => format!("{prompt}  ·  ↵ choose  ·  esc cancel"),
                _ => String::new(),
            },
            Step::TaggedPages(..) => "Pages with this tag  ·  ↵ open  ·  esc close".into(),
        }
    }

    /// Moves to `step`; the field's hint says what to type in it.
    fn set_step(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        let placeholder = self.placeholder();
        self.input
            .update(cx, |input, _| input.set_placeholder(placeholder));
    }

    fn placeholder(&self) -> &'static str {
        match &self.step {
            Step::Search => "Search pages, or type > for commands",
            Step::Choose { .. } => "Type to filter",
            Step::TaggedPages(..) => "Filter pages",
        }
    }

    fn render_item(&self, index: usize, item: &Item, cx: &mut Context<Self>) -> ListItem {
        let id = ElementId::Integer(index as u64);
        let row = match item {
            Item::Page { title, snippet, .. } => {
                let label = match snippet {
                    Some(snippet) => format!("{title}  —  {snippet}"),
                    None => title.to_string(),
                };
                ListItem::new(id, label).icon(IconName::Page)
            }
            Item::Command(command) => {
                let row = ListItem::new(id, command.title).icon(command.icon);
                match command.shortcut {
                    Some(shortcut) => row.end_slot(shortcut.hint()),
                    None => row.end_slot(command.category.label()),
                }
            }
            Item::Tag { name, count, .. } => ListItem::new(id, name.clone())
                .icon(IconName::Hash)
                .end_slot(count.to_string()),
            Item::Choice(choice) => {
                let row = ListItem::new(id, choice.label.clone());
                match choice.emoji.clone() {
                    Some(emoji) => row.emoji(emoji),
                    None => row,
                }
            }
        };
        row.emphasized()
            .when(index == self.highlighted, ListItem::highlighted)
            .on_click(cx.listener(move |palette, _, _, cx| palette.choose(index, cx)))
    }
}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut previous_section = None;
        let items = self.items.clone();
        for (index, item) in items.iter().enumerate() {
            let section = item.section();
            if !section.is_empty() && previous_section != Some(section) {
                rows.push(SectionHeader::new(section).into_any_element());
            }
            previous_section = Some(section);
            rows.push(self.render_item(index, item, cx).into_any_element());
        }
        let is_empty = rows.is_empty();
        div()
            .on_action(cx.listener(|palette, _: &MoveUp, _, cx| palette.move_highlight(-1, cx)))
            .on_action(cx.listener(|palette, _: &MoveDown, _, cx| palette.move_highlight(1, cx)))
            .child(
                Modal::new()
                    .child(div().p(theme.spacing.md).child(self.input.clone()))
                    .child(Divider::horizontal())
                    .child(
                        div()
                            .id("palette-results")
                            .track_scroll(&self.scroll)
                            .max_h(RESULTS_MAX_HEIGHT)
                            .overflow_y_scroll()
                            .p(theme.spacing.xs)
                            .children(rows)
                            .when(is_empty, |results| {
                                results.child(EmptyState::new("Nothing matches").inline())
                            }),
                    )
                    .child(Divider::horizontal())
                    .child(
                        div()
                            .px(theme.spacing.lg)
                            .py(theme.spacing.sm)
                            .text_size(theme.text.caption.size)
                            .text_color(theme.colors.text_placeholder)
                            .child(self.hint()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_pick_a_mode() {
        assert!(mode_of("> new") == (Mode::Commands, "new"));
        assert!(mode_of("#work") == (Mode::Tags, "work"));
        assert!(mode_of("@ plan") == (Mode::Pages, "plan"));
        assert!(mode_of("plan") == (Mode::Everything, "plan"));
    }
}
