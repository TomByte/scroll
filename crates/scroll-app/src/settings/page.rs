//! The settings page, drawn from the registry: the categories down the side, and a row per
//! setting of the one picked, with the control its kind calls for. Shortcuts come from the
//! command list instead (`shortcuts`).

use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
    px,
};
use scroll_settings::{AnySetting, Category, Choice, Control, Settings};
use scroll_ui::{
    Button, ButtonSize, Clickable, Divider, ListItem, Modal, ModalCloseButton, SegmentedControl,
    Switch, Theme, ThemeId, ThemePreview,
};
use serde_json::Value;

use crate::window::settings::{THEME, ThemePreference};

use super::shortcuts::ShortcutList;
use crate::commands::RunCommand;
use crate::mcp;

/// Wide enough for the categories beside a column of settings.
const PAGE_WIDTH: Pixels = px(760.0);
const PAGE_HEIGHT: Pixels = px(520.0);
const CATEGORIES_WIDTH: Pixels = px(200.0);

pub struct SettingsPage {
    focus_handle: FocusHandle,
    /// The category whose settings show; `None` until one is picked, which shows the first.
    category: Option<Category>,
    shortcuts: Entity<ShortcutList>,
    _settings: Subscription,
}

impl EventEmitter<DismissEvent> for SettingsPage {}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsPage {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        SettingsPage {
            focus_handle: cx.focus_handle(),
            category: None,
            shortcuts: cx.new(|_| ShortcutList::default()),
            // Shows changes made elsewhere too, such as a theme command run meanwhile.
            _settings: cx.observe_global::<Settings>(|_, cx| cx.notify()),
        }
    }

    fn show(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = Some(category);
        self.shortcuts
            .update(cx, |shortcuts, cx| shortcuts.stop_recording(cx));
        cx.notify();
    }

    fn render_categories(
        &self,
        categories: &[Category],
        shown: Option<Category>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = categories.iter().map(|&category| {
            ListItem::new(category.label(), category.label())
                .when(shown == Some(category), ListItem::selected)
                .on_click(cx.listener(move |page, _, _, cx| page.show(category, cx)))
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(theme.spacing.xs)
            .w(CATEGORIES_WIDTH)
            .h_full()
            .p(theme.spacing.md)
            .bg(theme.colors.surface_secondary)
            .border_r(theme.size.border_width)
            .border_color(theme.colors.border)
            .child(
                div()
                    .px(theme.spacing.sm)
                    .pt(theme.spacing.sm)
                    .pb(theme.spacing.md)
                    .text_size(theme.text.heading_3.size)
                    .line_height(theme.text.heading_3.line_height)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .children(rows)
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let sections = Settings::sections(cx);
        let mut categories: Vec<Category> =
            sections.iter().map(|(category, _)| *category).collect();
        categories.push(Category::Shortcuts);
        categories.sort();
        categories.dedup();
        let shown = self.category.or(categories.first().copied());
        let settings = sections
            .into_iter()
            .find(|(category, _)| Some(*category) == shown)
            .map(|(_, settings)| settings)
            .unwrap_or_default();
        let last = settings.len().saturating_sub(1);
        let mut rows = Vec::new();
        for (index, setting) in settings.into_iter().enumerate() {
            rows.push(if setting.key() == THEME.key {
                theme_picker(&theme, window, cx)
            } else {
                row(setting, &theme, cx)
            });
            if index < last {
                rows.push(Divider::horizontal().into_any_element());
            }
        }
        let has_mcp_row = shown == Some(Category::Integrations) && mcp::SERVER_ENABLED.get(cx);
        let content = div()
            .id("settings")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(theme.spacing.lg)
            .px(theme.spacing.xl)
            .pb(theme.spacing.xl)
            .overflow_y_scroll()
            .children(rows)
            .when(has_mcp_row, |content| {
                content
                    .child(Divider::horizontal())
                    .child(mcp_connection_row(&theme))
            })
            .when(shown == Some(Category::Shortcuts), |content| {
                content.child(self.shortcuts.clone())
            });
        let heading = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px(theme.spacing.xl)
            .pt(theme.spacing.lg)
            .pb(theme.spacing.lg)
            .text_size(theme.text.heading_3.size)
            .line_height(theme.text.heading_3.line_height)
            .font_weight(FontWeight::SEMIBOLD)
            .children(shown.map(Category::label))
            .child(ModalCloseButton);
        Modal::new().width(PAGE_WIDTH).child(
            div()
                .track_focus(&self.focus_handle)
                .flex()
                .h(PAGE_HEIGHT)
                .text_size(theme.text.ui.size)
                .child(self.render_categories(&categories, shown, &theme, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(heading)
                        .child(content),
                ),
        )
    }
}

fn row(setting: &'static dyn AnySetting, theme: &Theme, cx: &App) -> AnyElement {
    labelled_row(
        setting.label(),
        setting.description(),
        control(setting, cx),
        theme,
    )
}

/// Not a setting, but it belongs beside the one that turns the server on: the command holds a
/// token nobody can type from memory.
fn mcp_connection_row(theme: &Theme) -> AnyElement {
    let copy = Button::new("copy-mcp-connection", "Copy command")
        .size(ButtonSize::Small)
        .on_click(|_, window, cx| {
            let command = RunCommand {
                id: mcp::COPY_CONNECTION,
                page: None,
            };
            window.dispatch_action(Box::new(command), cx)
        });
    labelled_row(
        "Connect Claude Code",
        "Copies a command to paste into a terminal. It contains this computer's access token.",
        copy.into_any_element(),
        theme,
    )
}

fn labelled_row(
    label: &'static str,
    description: &'static str,
    control: AnyElement,
    theme: &Theme,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(theme.spacing.lg)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(theme.spacing.xs)
                .child(label)
                .child(
                    div()
                        .text_size(theme.text.caption.size)
                        .line_height(theme.text.caption.line_height)
                        .text_color(theme.colors.text_muted)
                        .child(description),
                ),
        )
        .child(control)
        .into_any_element()
}

/// Every theme as a row with a sample of its colours, since their names alone say little.
/// "System" is drawn as the theme the system currently gives.
fn theme_picker(theme: &Theme, window: &mut Window, cx: &mut App) -> AnyElement {
    let chosen = THEME.get(cx);
    let system_theme = ThemeId::system_default(window.appearance().into());
    let mut rows = Vec::new();
    for &option in ThemePreference::OPTIONS {
        let preview = match option {
            ThemePreference::System => ThemePreview::new(system_theme).label("System"),
            ThemePreference::Fixed(id) => ThemePreview::new(id),
        };
        let preview = if option == chosen {
            preview.selected()
        } else {
            preview
        };
        let row = div()
            .id(ElementId::Name(format!("theme-{}", option.id()).into()))
            .w_full()
            .rounded(theme.radius.md)
            .child(preview);
        rows.push(
            Clickable::new(theme.radius.md)
                .hover(theme.colors.hover)
                .on_click(move |_, _, cx| THEME.set(option, cx))
                .apply(row, window, cx),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .child(THEME.label)
        .child(div().flex().flex_col().gap(theme.spacing.xs).children(rows))
        .into_any_element()
}

/// A switch for a toggle, a segmented control for a choice; each sets the value in the form it is
/// stored in.
fn control(setting: &'static dyn AnySetting, cx: &App) -> AnyElement {
    match setting.control(cx) {
        Control::Toggle { is_on } => Switch::new(setting.key())
            .when(is_on, Switch::checked)
            .on_toggle(move |is_on, _, cx| setting.set_json(&Value::Bool(*is_on), cx))
            .into_any_element(),
        Control::Choice { options, selected } => {
            let ids: Vec<&'static str> = options.iter().map(|option| option.id).collect();
            options
                .iter()
                .fold(SegmentedControl::new(setting.key()), |control, option| {
                    control.option(option.label)
                })
                .selected(selected)
                .size(ButtonSize::Small)
                .on_select(move |index, _, cx| {
                    if let Some(id) = ids.get(*index) {
                        setting.set_json(&Value::String((*id).to_string()), cx);
                    }
                })
                .into_any_element()
        }
    }
}
