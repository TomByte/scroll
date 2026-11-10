use gpui::{
    AnyView, App, AppContext, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
};

use crate::controls::button::Button;
use crate::controls::text_input::{TextInput, TextInputEvent};
use crate::display::icon::{Icon, IconName, IconSize};
use crate::display::pill::Pill;
use crate::lists::list_item::ListItem;
use crate::overlays::popup_host::{PopupAnchor, PopupHost};
use crate::theme::Theme;

/// One choice in a `Combobox`: a pill in its swatch, or plain text without one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComboboxOption {
    pub label: SharedString,
    pub swatch: Option<usize>,
}

impl ComboboxOption {
    fn pill(&self) -> Pill {
        match self.swatch {
            Some(swatch) => Pill::new(self.label.clone()).swatch(swatch),
            None => Pill::new(self.label.clone()).neutral(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComboboxEvent {
    /// The chosen options (indexes into the options) are now these.
    Changed(Vec<usize>),
    /// The person typed a name no option has and asked for it; the owner adds the option
    /// with `set_options`, then `select_created`.
    CreateRequested(String),
}

/// A searchable list of options, opened by a `PopupHost`: type to filter, click or press Return to
/// choose, and (if `creatable`) Return on a new name asks for that option. With `multiple`
/// several can be chosen and are shown as removable pills; otherwise choosing one closes it.
pub struct Combobox {
    options: Vec<ComboboxOption>,
    chosen: Vec<usize>,
    allows_many: bool,
    is_creatable: bool,
    is_clearable: bool,
    search: Entity<TextInput>,
    _search_subscription: Subscription,
}

impl EventEmitter<ComboboxEvent> for Combobox {}
impl EventEmitter<DismissEvent> for Combobox {}

impl Focusable for Combobox {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
}

impl Combobox {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx).placeholder("Search…"));
        let subscription = cx.subscribe(&search, |combobox, _, event: &TextInputEvent, cx| {
            match event {
                TextInputEvent::Changed => cx.notify(),
                TextInputEvent::Submitted => combobox.choose_from_search(cx),
                // Escape reaches the popover, which dismisses the combobox.
                TextInputEvent::Cancelled => {}
            }
        });
        Combobox {
            options: Vec::new(),
            chosen: Vec::new(),
            allows_many: false,
            is_creatable: false,
            is_clearable: false,
            search,
            _search_subscription: subscription,
        }
    }

    pub fn options(mut self, options: Vec<ComboboxOption>) -> Self {
        self.options = options;
        self
    }

    pub fn multiple(mut self) -> Self {
        self.allows_many = true;
        self
    }

    pub fn creatable(mut self, cx: &mut Context<Self>) -> Self {
        self.is_creatable = true;
        self.search.update(cx, |search, _| {
            search.set_placeholder("Search or create an option…")
        });
        self
    }

    /// Offers "Clear" while something is chosen.
    pub fn clearable(mut self) -> Self {
        self.is_clearable = true;
        self
    }

    pub fn chosen(mut self, chosen: Vec<usize>) -> Self {
        self.chosen = chosen;
        self
    }

    /// Starts with `text` already typed in the search field.
    pub fn search_for(&mut self, text: &str, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.set_text(text, cx));
    }

    /// After the owner changed the options, e.g. added one asked for with `CreateRequested`.
    pub fn set_options(&mut self, options: Vec<ComboboxOption>, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    /// A newly created option joins the choice, as if it had been clicked.
    pub fn select_created(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.chosen.contains(&index) {
            self.toggle(index, cx);
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    fn matching_options(&self, query: &str) -> Vec<usize> {
        let query = query.to_lowercase();
        (0..self.options.len())
            .filter(|index| self.options[*index].label.to_lowercase().contains(&query))
            .collect()
    }

    fn has_exact_match(&self, query: &str) -> bool {
        self.options
            .iter()
            .any(|option| option.label.eq_ignore_ascii_case(query))
    }

    fn choose_from_search(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if query.is_empty() {
            return;
        }
        if self.is_creatable && !self.has_exact_match(&query) {
            cx.emit(ComboboxEvent::CreateRequested(query));
            self.clear_search(cx);
            return;
        }
        if let Some(index) = self.matching_options(&query).first() {
            self.toggle(*index, cx);
        }
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.allows_many {
            self.chosen = vec![index];
            cx.emit(ComboboxEvent::Changed(self.chosen.clone()));
            cx.emit(DismissEvent);
            return;
        }
        match self.chosen.iter().position(|chosen| *chosen == index) {
            Some(position) => {
                self.chosen.remove(position);
            }
            None => self.chosen.push(index),
        }
        self.clear_search(cx);
        cx.emit(ComboboxEvent::Changed(self.chosen.clone()));
        cx.notify();
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        self.chosen.retain(|chosen| *chosen != index);
        cx.emit(ComboboxEvent::Changed(self.chosen.clone()));
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.chosen.clear();
        cx.emit(ComboboxEvent::Changed(Vec::new()));
        cx.emit(DismissEvent);
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.set_text("", cx));
    }

    fn render_chosen(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .gap(theme.spacing.xs)
            .px(theme.spacing.xs)
            .pb(theme.spacing.sm)
            .children(self.chosen.iter().filter_map(|index| {
                let option = self.options.get(*index)?;
                let index = *index;
                Some(option.pill().on_remove(
                    ("chosen", index),
                    cx.listener(move |combobox, _, _, cx| combobox.remove(index, cx)),
                ))
            }))
    }
}

impl Render for Combobox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let query = self.query(cx);
        let offers_create = self.is_creatable && !query.is_empty() && !self.has_exact_match(&query);
        let options: Vec<_> = self
            .matching_options(&query)
            .into_iter()
            .map(|index| {
                let option = &self.options[index];
                let id = ElementId::named_usize("option", index);
                let item = match option.swatch {
                    Some(_) => ListItem::new(id, "").start_slot(option.pill()),
                    None => ListItem::new(id, option.label.clone()),
                };
                item.when(self.chosen.contains(&index), |item| {
                    item.end_slot(Icon::new(IconName::Check).size(IconSize::Small))
                })
                .on_click(cx.listener(move |combobox, _, _, cx| combobox.toggle(index, cx)))
            })
            .collect();
        let chosen =
            (!self.chosen.is_empty() && self.allows_many).then(|| self.render_chosen(&theme, cx));

        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .children(chosen)
            .child(self.search.clone())
            .child(
                div()
                    .id("options")
                    .flex()
                    .flex_col()
                    .max_h(theme.size.option_list_max_height)
                    .overflow_y_scroll()
                    .children(options),
            )
            .when(self.is_clearable && !self.chosen.is_empty(), |panel| {
                panel.child(
                    ListItem::new("clear-choice", "Clear")
                        .on_click(cx.listener(|combobox, _, _, cx| combobox.clear(cx))),
                )
            })
            .when(offers_create, |panel| {
                panel.child(
                    ListItem::new(
                        "create-option",
                        SharedString::from(format!("Create “{query}”")),
                    )
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|combobox, _, _, cx| combobox.choose_from_search(cx))),
                )
            })
    }
}

const STORY_OPTIONS: [&str; 4] = ["Design", "Research", "Engineering", "Marketing"];

fn story_options() -> Vec<ComboboxOption> {
    STORY_OPTIONS
        .into_iter()
        .enumerate()
        .map(|(index, label)| ComboboxOption {
            label: label.into(),
            swatch: Some(index + 2),
        })
        .collect()
}

impl Combobox {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let popups = cx.new(|_| PopupHost::new());
        cx.new(|_| ComboboxStory {
            popups,
            chosen: vec![0, 2],
        })
        .into()
    }
}

struct ComboboxStory {
    popups: Entity<PopupHost>,
    chosen: Vec<usize>,
}

impl ComboboxStory {
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = self.chosen.clone();
        let combobox = self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Below("tags".into()),
                |_, cx| {
                    Combobox::new(cx)
                        .options(story_options())
                        .multiple()
                        .creatable(cx)
                        .chosen(chosen)
                },
                window,
                cx,
            )
        });
        cx.subscribe(&combobox, |story, _, event: &ComboboxEvent, cx| {
            if let ComboboxEvent::Changed(chosen) = event {
                story.chosen = chosen.clone();
                cx.notify();
            }
        })
        .detach();
    }
}

impl Render for ComboboxStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let trigger = self.popups.read(cx).trigger("tags");
        let chosen = self
            .chosen
            .iter()
            .map(|index| STORY_OPTIONS[*index])
            .collect::<Vec<_>>()
            .join(", ");
        div()
            .flex()
            .items_center()
            .gap(theme.spacing.md)
            .child(
                div().relative().child(trigger).child(
                    Button::new("choose-tags", "Choose tags")
                        .on_click(cx.listener(|story, _, window, cx| story.open(window, cx))),
                ),
            )
            .child(div().text_color(theme.colors.text_muted).child(chosen))
            .child(self.popups.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::TestAppContext;

    use super::*;

    fn open(
        cx: &mut TestAppContext,
    ) -> (
        Entity<Combobox>,
        Rc<RefCell<Vec<ComboboxEvent>>>,
        &mut gpui::VisualTestContext,
    ) {
        cx.update(crate::init);
        let (combobox, cx) = cx.add_window_view(|_, cx| {
            Combobox::new(cx)
                .options(story_options())
                .multiple()
                .creatable(cx)
                .chosen(vec![0, 2])
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&combobox, move |_, event: &ComboboxEvent, _| {
                recorded.borrow_mut().push(event.clone())
            })
            .detach()
        });
        let focus = combobox.read_with(cx, |combobox, cx| combobox.focus_handle(cx));
        cx.update(|window, _| window.focus(&focus));
        (combobox, events, cx)
    }

    #[gpui::test]
    fn return_on_a_matching_name_toggles_it(cx: &mut TestAppContext) {
        let (_, events, cx) = open(cx);
        cx.simulate_input("research");
        cx.simulate_keystrokes("enter");
        assert_eq!(*events.borrow(), [ComboboxEvent::Changed(vec![0, 2, 1])]);
    }

    #[gpui::test]
    fn return_on_a_new_name_asks_for_it(cx: &mut TestAppContext) {
        let (_, events, cx) = open(cx);
        cx.simulate_input("Sales");
        cx.simulate_keystrokes("enter");
        assert_eq!(
            *events.borrow(),
            [ComboboxEvent::CreateRequested("Sales".into())]
        );
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut TestAppContext) {
        crate::foundation::story::testing::open_story(Combobox::story, cx);
    }

    #[gpui::test]
    fn clear_chooses_nothing(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (combobox, cx) = cx.add_window_view(|_, cx| {
            Combobox::new(cx)
                .options(story_options())
                .clearable()
                .chosen(vec![1])
        });
        let changed = Rc::new(RefCell::new(None));
        let recorded = changed.clone();
        cx.update(|_, cx| {
            cx.subscribe(&combobox, move |_, event: &ComboboxEvent, _| {
                *recorded.borrow_mut() = Some(event.clone())
            })
            .detach()
        });
        combobox.update(cx, |combobox, cx| combobox.clear(cx));
        assert_eq!(*changed.borrow(), Some(ComboboxEvent::Changed(Vec::new())));
    }
}
