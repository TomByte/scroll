//! The settings page, drawn from the registry: a section per category, a row per setting with
//! the control its kind calls for.

use gpui::{
    AnyElement, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, div, prelude::FluentBuilder,
};
use scroll_settings::{AnySetting, Control, Settings};
use scroll_ui::{
    Button, ButtonSize, ButtonVariant, Dialog, Divider, SectionHeader, SegmentedControl, Switch,
    Theme,
};
use serde_json::Value;

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
                .child(setting.label())
                .child(
                    div()
                        .text_size(theme.text.caption.size)
                        .line_height(theme.text.caption.line_height)
                        .text_color(theme.colors.text_muted)
                        .child(setting.description()),
                ),
        )
        .child(control(setting, cx))
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
