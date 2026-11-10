//! The Shortcuts section of the settings page: every command whose shortcut can change, by
//! category, with its current keys and buttons to record new ones, remove them or put the
//! default back.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, KeystrokeEvent, ParentElement,
    Pixels, Render, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use scroll_ui::{Button, ButtonSize, ButtonVariant, SectionHeader, Theme};

use crate::commands::{Command, ShortcutRefusal};
use crate::window::command_list;

/// Wide enough for "Press a shortcut…" and the longest key combination, so buttons line up.
const KEYS_WIDTH: Pixels = px(132.0);

#[derive(Default)]
pub struct ShortcutList {
    recording: Option<Recording>,
    /// Why the last change to a command's shortcut was refused, shown in its row.
    refusal: Option<(&'static str, ShortcutRefusal)>,
}

/// Waiting for the keys of `command`'s new shortcut. While it lasts every keystroke comes here
/// before any binding sees it, so the keys pressed do not also run what they are bound to.
struct Recording {
    command: &'static Command,
    _keystrokes: Subscription,
}

impl ShortcutList {
    pub fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            cx.notify();
        }
    }

    fn start_recording(&mut self, command: &'static Command, cx: &mut Context<Self>) {
        let list = cx.weak_entity();
        let keystrokes = cx.intercept_keystrokes(move |event, _, cx| {
            // Pressing a modifier alone arrives as a keystroke of its own; wait for the key.
            if is_modifier_alone(event) {
                return;
            }
            cx.stop_propagation();
            let recorded = list.update(cx, |list, cx| list.record(event, cx));
            if let Err(error) = recorded {
                log::warn!("a shortcut was pressed after its settings closed: {error}");
            }
        });
        self.recording = Some(Recording {
            command,
            _keystrokes: keystrokes,
        });
        self.refusal = None;
        cx.notify();
    }

    /// Escape gives up; any other keys become the shortcut, or say why they cannot and wait for
    /// others.
    fn record(&mut self, event: &KeystrokeEvent, cx: &mut Context<Self>) {
        let Some(command) = self.recording.as_ref().map(|recording| recording.command) else {
            return;
        };
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            return self.stop_recording(cx);
        }
        match command.change_shortcut(keystroke, command_list::all(), cx) {
            Ok(()) => {
                self.recording = None;
                self.refusal = None;
            }
            Err(refusal) => self.refusal = Some((command.id, refusal)),
        }
        cx.notify();
    }

    fn toggle_recording(&mut self, command: &'static Command, cx: &mut Context<Self>) {
        if self.is_recording(command) {
            self.stop_recording(cx);
        } else {
            self.start_recording(command, cx);
        }
    }

    fn remove(&mut self, command: &'static Command, cx: &mut Context<Self>) {
        self.recording = None;
        self.refusal = None;
        command.remove_shortcut(cx);
        cx.notify();
    }

    fn reset(&mut self, command: &'static Command, cx: &mut Context<Self>) {
        self.recording = None;
        self.refusal = command
            .reset_shortcut(command_list::all(), cx)
            .err()
            .map(|refusal| (command.id, refusal));
        cx.notify();
    }

    fn is_recording(&self, command: &Command) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|recording| recording.command.id == command.id)
    }

    fn render_row(
        &self,
        command: &'static Command,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_recording = self.is_recording(command);
        let shortcut = command.current_shortcut(cx);
        let has_shortcut = shortcut.is_some();
        let keys = match (&shortcut, is_recording) {
            (_, true) => "Press a shortcut…".to_string(),
            (Some(shortcut), false) => shortcut.hint(),
            (None, false) => "None".to_string(),
        };
        let refusal = self
            .refusal
            .as_ref()
            .filter(|(id, _)| *id == command.id)
            .map(|(_, refusal)| refusal.to_string());
        let id = |part: &str| ElementId::Name(format!("shortcut-{part}-{}", command.id).into());
        let change = Button::new(id("change"), if is_recording { "Cancel" } else { "Change" })
            .size(ButtonSize::Small)
            .on_click(cx.listener(move |list, _, _, cx| list.toggle_recording(command, cx)));
        let remove = Button::new(id("remove"), "Remove")
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Small)
            .when(!has_shortcut, Button::disabled)
            .on_click(cx.listener(move |list, _, _, cx| list.remove(command, cx)));
        let reset = Button::new(id("reset"), "Reset")
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Small)
            .when(!command.has_changed_shortcut(cx), Button::disabled)
            .on_click(cx.listener(move |list, _, _, cx| list.reset(command, cx)));
        div()
            .flex()
            .items_center()
            .gap(theme.spacing.sm)
            .py(theme.spacing.xs)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(command.title)
                    .when_some(refusal, |title, refusal| {
                        title.child(
                            div()
                                .text_size(theme.text.caption.size)
                                .line_height(theme.text.caption.line_height)
                                .text_color(theme.colors.danger)
                                .child(refusal),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .w(KEYS_WIDTH)
                    .when(!has_shortcut || is_recording, |keys| {
                        keys.text_color(theme.colors.text_muted)
                    })
                    .child(keys),
            )
            .child(change)
            .child(remove)
            .child(reset)
            .into_any_element()
    }
}

impl Render for ShortcutList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let commands: Vec<&'static Command> = command_list::all()
            .iter()
            .copied()
            .filter(|command| command.has_changeable_shortcut())
            .collect();
        let mut categories = Vec::new();
        for command in &commands {
            if !categories.contains(&command.category) {
                categories.push(command.category);
            }
        }
        let mut rows = Vec::new();
        for category in categories {
            rows.push(SectionHeader::new(category.label()).into_any_element());
            for command in commands
                .iter()
                .filter(|command| command.category == category)
            {
                rows.push(self.render_row(command, &theme, cx));
            }
        }
        div().id("shortcuts").flex().flex_col().children(rows)
    }
}

fn is_modifier_alone(event: &KeystrokeEvent) -> bool {
    matches!(
        event.keystroke.key.as_str(),
        "shift" | "control" | "alt" | "platform" | "function"
    )
}
