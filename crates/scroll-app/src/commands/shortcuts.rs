//! The person's own shortcuts: any command's default can be replaced or removed on the settings
//! page. Only the differences are kept, in preferences, under each command's id.

use gpui::{App, Keystroke};
use scroll_settings::SettingMap;

use super::{Command, FROM_COMMANDS, Shortcut};

/// A key combination, or `NO_SHORTCUT`, for each command whose shortcut is not its default.
const CHANGED_SHORTCUTS: SettingMap = SettingMap { key: "shortcuts" };

/// Kept for a command whose default the person removed.
const NO_SHORTCUT: &str = "none";

/// Why a key combination cannot become a command's shortcut; the message is shown as it is.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShortcutRefusal {
    #[error("Use ⌘, ⌃ or ⌥ with the key, so it does not take a key that types.")]
    NeedsModifier,
    #[error("{keys} is already used by “{command}”.")]
    UsedByCommand { keys: String, command: &'static str },
    #[error("{keys} is kept for editing text and moving around.")]
    Reserved { keys: String },
}

impl Command {
    /// The shortcut that runs it now: the person's own, or its default if they have not changed
    /// it or what is kept cannot be used.
    pub fn current_shortcut(&self, cx: &App) -> Option<Shortcut> {
        let Some(keys) = CHANGED_SHORTCUTS.get(self.id, cx) else {
            return self.shortcut.clone();
        };
        if keys == NO_SHORTCUT {
            return None;
        }
        match usable_keystroke(keys) {
            Some(keystroke) => Some(Shortcut {
                keys: keystroke.unparse().into(),
                context: self.shortcut_context(),
            }),
            None => self.shortcut.clone(),
        }
    }

    /// Whether the person changed its shortcut from the default.
    pub fn has_changed_shortcut(&self, cx: &App) -> bool {
        CHANGED_SHORTCUTS.get(self.id, cx).is_some()
    }

    /// The standard Edit menu items keep the keys text fields bind for them (⌘C, ⌘Z…).
    pub fn has_changeable_shortcut(&self) -> bool {
        self.os_action.is_none()
    }

    /// Makes `keystroke` its shortcut, unless a command in the same place already uses it, or
    /// it is a key that types or one the app keeps for editing and moving around.
    pub fn change_shortcut(
        &self,
        keystroke: &Keystroke,
        commands: &[&'static Command],
        cx: &mut App,
    ) -> Result<(), ShortcutRefusal> {
        let keystroke = without_typed_text(keystroke);
        if let Some(other) = self.command_using(&keystroke, commands, cx) {
            return Err(ShortcutRefusal::UsedByCommand {
                keys: hint(&keystroke),
                command: other.title,
            });
        }
        if self.default_keystroke().as_ref() == Some(&keystroke) {
            CHANGED_SHORTCUTS.remove(self.id, cx);
            return Ok(());
        }
        if !can_be_shortcut(&keystroke) {
            return Err(ShortcutRefusal::NeedsModifier);
        }
        if is_bound_outside_commands(&keystroke, cx) {
            return Err(ShortcutRefusal::Reserved {
                keys: hint(&keystroke),
            });
        }
        CHANGED_SHORTCUTS.set(self.id, &keystroke.unparse(), cx);
        Ok(())
    }

    pub fn remove_shortcut(&self, cx: &mut App) {
        match self.shortcut {
            Some(_) => CHANGED_SHORTCUTS.set(self.id, NO_SHORTCUT, cx),
            None => CHANGED_SHORTCUTS.remove(self.id, cx),
        }
    }

    /// Puts its default back, unless another command has been given that key meanwhile.
    pub fn reset_shortcut(
        &self,
        commands: &[&'static Command],
        cx: &mut App,
    ) -> Result<(), ShortcutRefusal> {
        match self.default_keystroke() {
            Some(keystroke) => self.change_shortcut(&keystroke, commands, cx),
            None => {
                CHANGED_SHORTCUTS.remove(self.id, cx);
                Ok(())
            }
        }
    }

    /// Where its shortcut applies: its default's key context, so a replacement for an editor
    /// command works in the editor only; everywhere for a command with no default.
    fn shortcut_context(&self) -> Option<&'static str> {
        self.shortcut.as_ref().and_then(|shortcut| shortcut.context)
    }

    fn default_keystroke(&self) -> Option<Keystroke> {
        let shortcut = self.shortcut.as_ref()?;
        Keystroke::parse(&shortcut.keys).ok()
    }

    /// Another command whose current shortcut is `keystroke` somewhere this one's would also
    /// apply.
    fn command_using(
        &self,
        keystroke: &Keystroke,
        commands: &[&'static Command],
        cx: &App,
    ) -> Option<&'static Command> {
        let context = self.shortcut_context();
        commands.iter().copied().find(|other| {
            other.id != self.id
                && other.current_shortcut(cx).is_some_and(|shortcut| {
                    let is_same_place = context.is_none()
                        || shortcut.context.is_none()
                        || shortcut.context == context;
                    is_same_place
                        && Keystroke::parse(&shortcut.keys).ok().as_ref() == Some(keystroke)
                })
        })
    }
}

/// The keystroke kept in `keys`, if it is one this app would bind; anything else (written by
/// hand, or by a later version) is ignored.
fn usable_keystroke(keys: &str) -> Option<Keystroke> {
    Keystroke::parse(keys)
        .ok()
        .filter(|keystroke| !keystroke.key.is_empty() && can_be_shortcut(keystroke))
}

/// With ⌘, ⌃ or ⌥ held, or a function key: anything else types, or moves around, somewhere.
fn can_be_shortcut(keystroke: &Keystroke) -> bool {
    let modifiers = keystroke.modifiers;
    modifiers.platform || modifiers.control || modifiers.alt || is_function_key(&keystroke.key)
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=20).contains(&number))
}

/// The keys alone, as bindings match them: a pressed keystroke also carries the text it types.
fn without_typed_text(keystroke: &Keystroke) -> Keystroke {
    Keystroke {
        key_char: None,
        ..keystroke.clone()
    }
}

/// Bound by something other than a command, such as text fields' ⌘C or the table's arrows.
fn is_bound_outside_commands(keystroke: &Keystroke, cx: &App) -> bool {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap.bindings().any(|binding| {
        binding.meta() != Some(FROM_COMMANDS)
            && binding.match_keystrokes(std::slice::from_ref(keystroke)) == Some(false)
    })
}

fn hint(keystroke: &Keystroke) -> String {
    Shortcut {
        keys: keystroke.unparse().into(),
        context: None,
    }
    .hint()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::{Action, KeyContext, TestAppContext};
    use scroll_core::id::PageId;

    use super::*;
    use crate::commands::{RunCommand, find, hint as command_hint, keep_keys_bound};
    use crate::window::command_list;

    /// Keys bound as at startup, with preferences in a file of their own.
    fn start(cx: &mut TestAppContext) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("scroll-shortcuts-{}.json", PageId::generate().0));
        cx.update(|cx| {
            scroll_settings::init(path.clone(), cx);
            scroll_ui::init(cx);
            scroll_table::init(cx);
            crate::editor::init(cx);
            keep_keys_bound(command_list::all(), |_| {}, cx);
        });
        path
    }

    fn command(id: &str) -> &'static Command {
        find(command_list::all(), id).unwrap()
    }

    fn keys(keys: &str) -> Keystroke {
        Keystroke::parse(keys).unwrap()
    }

    fn change(
        id: &str,
        keys_pressed: &str,
        cx: &mut TestAppContext,
    ) -> Result<(), ShortcutRefusal> {
        let result = cx
            .update(|cx| command(id).change_shortcut(&keys(keys_pressed), command_list::all(), cx));
        cx.run_until_parked();
        result
    }

    fn current_keys(id: &str, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| {
            command(id)
                .current_shortcut(cx)
                .map(|shortcut| shortcut.keys.to_string())
        })
    }

    /// The keys the menu bar shows for `action`: what GPUI's menus read from the keymap.
    fn keys_bound_to(action: &dyn Action, cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| {
            cx.key_bindings()
                .borrow()
                .bindings_for_action(action)
                .map(|binding| {
                    binding
                        .keystrokes()
                        .iter()
                        .map(|keystroke| keystroke.inner().unparse())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
    }

    fn new_page() -> RunCommand {
        RunCommand {
            id: "page.new",
            page: None,
        }
    }

    #[gpui::test]
    fn a_changed_shortcut_is_bound_shown_and_kept_across_a_restart(cx: &mut TestAppContext) {
        let path = start(cx);
        assert_eq!(keys_bound_to(&new_page(), cx), ["cmd-n"]);

        assert_eq!(change("page.new", "cmd-j", cx), Ok(()));

        assert_eq!(current_keys("page.new", cx).as_deref(), Some("cmd-j"));
        assert_eq!(
            cx.update(|cx| command_hint(command_list::all(), "page.new", cx)),
            "⌘J"
        );
        assert_eq!(
            keys_bound_to(&new_page(), cx),
            ["cmd-j"],
            "the old key is unbound"
        );
        let text_keys = keys_bound_to(&scroll_ui::actions::Copy, cx);
        assert!(
            !text_keys.is_empty(),
            "other parts' keys survive the rebinding"
        );

        cx.update(|cx| scroll_settings::init(path.clone(), cx));
        assert_eq!(current_keys("page.new", cx).as_deref(), Some("cmd-j"));
        std::fs::remove_file(path).ok();
    }

    #[gpui::test]
    fn a_shortcut_can_be_removed_and_its_default_put_back(cx: &mut TestAppContext) {
        let path = start(cx);
        cx.update(|cx| command("page.new").remove_shortcut(cx));
        cx.run_until_parked();
        assert_eq!(current_keys("page.new", cx), None);
        assert!(keys_bound_to(&new_page(), cx).is_empty());
        assert_eq!(
            cx.update(|cx| command_hint(command_list::all(), "page.new", cx)),
            ""
        );

        let reset = cx.update(|cx| command("page.new").reset_shortcut(command_list::all(), cx));
        cx.run_until_parked();
        assert_eq!(reset, Ok(()));
        assert_eq!(current_keys("page.new", cx).as_deref(), Some("cmd-n"));
        assert!(!cx.update(|cx| command("page.new").has_changed_shortcut(cx)));
        assert_eq!(keys_bound_to(&new_page(), cx), ["cmd-n"]);

        // A command with no default keeps nothing once its shortcut is removed.
        assert_eq!(change("page.rename", "cmd-alt-r", cx), Ok(()));
        cx.update(|cx| command("page.rename").remove_shortcut(cx));
        assert!(!cx.update(|cx| command("page.rename").has_changed_shortcut(cx)));
        std::fs::remove_file(path).ok();
    }

    #[gpui::test]
    fn keys_in_use_or_that_type_are_refused_and_change_nothing(cx: &mut TestAppContext) {
        let path = start(cx);
        assert_eq!(
            change("go.back", "cmd-n", cx),
            Err(ShortcutRefusal::UsedByCommand {
                keys: "⌘N".into(),
                command: "New page"
            })
        );
        assert_eq!(
            change("page.new", "cmd-b", cx),
            Err(ShortcutRefusal::UsedByCommand {
                keys: "⌘B".into(),
                command: "Bold"
            }),
            "a key used in the editor is taken for a command that works everywhere"
        );
        assert_eq!(
            change("page.new", "j", cx),
            Err(ShortcutRefusal::NeedsModifier)
        );
        assert_eq!(
            change("page.new", "cmd-c", cx),
            Err(ShortcutRefusal::Reserved {
                keys: "⌘C".into()
            })
        );
        assert_eq!(current_keys("page.new", cx).as_deref(), Some("cmd-n"));
        assert_eq!(current_keys("go.back", cx).as_deref(), Some("cmd-["));

        assert_eq!(
            change("database.open_row", "cmd-b", cx),
            Ok(()),
            "the table and the editor never have focus together"
        );
        std::fs::remove_file(path).ok();
    }

    #[gpui::test]
    fn a_default_given_to_another_command_is_not_put_back(cx: &mut TestAppContext) {
        let path = start(cx);
        assert_eq!(change("page.new", "cmd-j", cx), Ok(()));
        assert_eq!(change("go.back", "cmd-n", cx), Ok(()), "cmd-n is free now");

        let reset = cx.update(|cx| command("page.new").reset_shortcut(command_list::all(), cx));
        assert_eq!(
            reset,
            Err(ShortcutRefusal::UsedByCommand {
                keys: "⌘N".into(),
                command: "Go back"
            })
        );
        assert_eq!(
            change("page.new", "cmd-n", cx),
            Err(ShortcutRefusal::UsedByCommand {
                keys: "⌘N".into(),
                command: "Go back"
            }),
            "nor recorded again"
        );
        let go_back = RunCommand {
            id: "go.back",
            page: None,
        };
        assert_eq!(
            keys_bound_to(&go_back, cx),
            ["cmd-n"],
            "switching off cmd-n's old use does not switch off its new one"
        );
        std::fs::remove_file(path).ok();
    }

    #[gpui::test]
    fn a_replaced_default_that_a_component_also_binds_stops_working(cx: &mut TestAppContext) {
        let path = start(cx);
        let table = KeyContext::parse(scroll_table::KEY_CONTEXT).unwrap();
        let opens_row = |pressed: &str, contexts: &[KeyContext], cx: &mut TestAppContext| {
            cx.update(|cx| {
                let (bindings, _) = cx
                    .key_bindings()
                    .borrow()
                    .bindings_for_input(&[keys(pressed)], contexts);
                bindings
                    .first()
                    .is_some_and(|binding| binding.action().partial_eq(&scroll_table::OpenRow))
            })
        };
        assert!(opens_row("cmd-enter", std::slice::from_ref(&table), cx));

        assert_eq!(change("database.open_row", "cmd-alt-o", cx), Ok(()));

        assert!(
            !opens_row("cmd-enter", std::slice::from_ref(&table), cx),
            "the table's own binding of the old key is switched off"
        );
        assert!(opens_row("cmd-alt-o", std::slice::from_ref(&table), cx));
        assert!(
            !opens_row("cmd-alt-o", &[], cx),
            "the new key keeps the default's context"
        );
        assert_eq!(keys_bound_to(&scroll_table::OpenRow, cx), ["alt-cmd-o"]);
        std::fs::remove_file(path).ok();
    }

    #[gpui::test]
    fn shortcuts_this_version_cannot_use_read_as_defaults_and_keep_other_preferences(
        cx: &mut TestAppContext,
    ) {
        let path =
            std::env::temp_dir().join(format!("scroll-shortcuts-{}.json", PageId::generate().0));
        std::fs::write(
            &path,
            r#"{
                "appearance.theme": "dark",
                "shortcuts": {
                    "page.new": "cmd-k cmd-s",
                    "go.back": 5,
                    "go.forward": "j",
                    "app.quit": "cmd-",
                    "no.such.command": "cmd-y"
                }
            }"#,
        )
        .unwrap();
        cx.update(|cx| {
            scroll_settings::init(path.clone(), cx);
            scroll_ui::init(cx);
            keep_keys_bound(command_list::all(), |_| {}, cx);
        });
        for (id, default) in [
            ("page.new", "cmd-n"),
            ("go.back", "cmd-["),
            ("go.forward", "cmd-]"),
            ("app.quit", "cmd-q"),
        ] {
            assert_eq!(current_keys(id, cx).as_deref(), Some(default), "{id}");
        }
        assert_eq!(keys_bound_to(&new_page(), cx), ["cmd-n"]);

        std::fs::write(
            &path,
            r#"{"appearance.theme": "dark", "shortcuts": "garbage"}"#,
        )
        .unwrap();
        cx.update(|cx| scroll_settings::init(path.clone(), cx));
        assert_eq!(current_keys("page.new", cx).as_deref(), Some("cmd-n"));
        assert_eq!(change("page.new", "cmd-j", cx), Ok(()));
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["appearance.theme"], "dark");
        assert_eq!(saved["shortcuts"], serde_json::json!({"page.new": "cmd-j"}));
        std::fs::remove_file(path).ok();
    }
}
