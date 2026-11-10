//! The settings page, drawn from the registry: a section per category, a row per setting with
//! the control its kind calls for.

use gpui::{
    AnyElement, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, div, prelude::FluentBuilder,
};
use scroll_settings::{AnySetting, Category, Control, Settings};
use scroll_ui::{
    Button, ButtonSize, ButtonVariant, Dialog, Divider, SectionHeader, SegmentedControl, Switch,
    Theme,
};
use serde_json::Value;

use crate::commands::RunCommand;
use crate::mcp;

pub struct SettingsPage {
    focus_handle: FocusHandle,
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
            // Shows changes made elsewhere too, such as a theme command run meanwhile.
            _settings: cx.observe_global::<Settings>(|_, cx| cx.notify()),
        }
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let sections = Settings::sections(cx);
        let section_count = sections.len();
        let sections = sections
            .into_iter()
            .enumerate()
            .map(|(index, (category, settings))| {
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing.sm)
                    .child(SectionHeader::new(category.label()))
                    .children(settings.into_iter().map(|setting| row(setting, &theme, cx)))
                    .when(
                        category == Category::Integrations && mcp::SERVER_ENABLED.get(cx),
                        |section| section.child(mcp_connection_row(&theme)),
                    )
                    .when(index + 1 < section_count, |section| {
                        section.child(Divider::horizontal())
                    })
            });
        Dialog::new("Settings")
            .track_focus(&self.focus_handle)
            .children(sections)
            .action(
                Button::new("done", "Done")
                    .variant(ButtonVariant::Primary)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
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
        .px(theme.spacing.sm)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
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
