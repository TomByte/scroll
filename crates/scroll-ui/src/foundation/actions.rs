//! Actions shared by every text-editing surface, plus the navigation actions used by
//! menus, popovers and modals. Key bindings follow standard macOS text conventions.

use gpui::{App, KeyBinding, actions};

actions!(
    text_editing,
    [
        Backspace,
        Delete,
        DeleteWordBackward,
        DeleteWordForward,
        DeleteToLineStart,
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        MoveWordLeft,
        MoveWordRight,
        MoveToLineStart,
        MoveToLineEnd,
        MoveToStart,
        MoveToEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectToLineStart,
        SelectToLineEnd,
        SelectToStart,
        SelectToEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Newline,
        ShowCharacterPalette,
    ]
);

actions!(navigation, [SelectPrevious, SelectNext, Confirm, Cancel]);

actions!(focus, [FocusNext, FocusPrevious]);

// Key-context names. The bindings below are scoped to them, and each component sets its own.
pub(crate) const MENU_CONTEXT: &str = "Menu";
pub(crate) const POPOVER_CONTEXT: &str = "Popover";
pub(crate) const MODAL_CONTEXT: &str = "Modal";
/// Set only by a dialog with a confirm action, so Enter elsewhere in modals is untouched.
pub(crate) const DIALOG_CONTEXT: &str = "Dialog";
pub(crate) const TEXT_INPUT_CONTEXT: &str = "TextInput";

/// Binds the macOS text-editing shortcuts within `context` (a GPUI key context name).
pub fn bind_text_editing_keys(context: &str, cx: &mut App) {
    let context = Some(context);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("ctrl-d", Delete, context),
        KeyBinding::new("alt-backspace", DeleteWordBackward, context),
        KeyBinding::new("alt-delete", DeleteWordForward, context),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, context),
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("alt-left", MoveWordLeft, context),
        KeyBinding::new("alt-right", MoveWordRight, context),
        KeyBinding::new("cmd-left", MoveToLineStart, context),
        KeyBinding::new("cmd-right", MoveToLineEnd, context),
        KeyBinding::new("home", MoveToLineStart, context),
        KeyBinding::new("end", MoveToLineEnd, context),
        KeyBinding::new("ctrl-a", MoveToLineStart, context),
        KeyBinding::new("ctrl-e", MoveToLineEnd, context),
        KeyBinding::new("cmd-up", MoveToStart, context),
        KeyBinding::new("cmd-down", MoveToEnd, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("alt-shift-left", SelectWordLeft, context),
        KeyBinding::new("alt-shift-right", SelectWordRight, context),
        KeyBinding::new("cmd-shift-left", SelectToLineStart, context),
        KeyBinding::new("cmd-shift-right", SelectToLineEnd, context),
        KeyBinding::new("shift-home", SelectToLineStart, context),
        KeyBinding::new("shift-end", SelectToLineEnd, context),
        KeyBinding::new("cmd-shift-up", SelectToStart, context),
        KeyBinding::new("cmd-shift-down", SelectToEnd, context),
        KeyBinding::new("cmd-a", SelectAll, context),
        KeyBinding::new("cmd-c", Copy, context),
        KeyBinding::new("cmd-x", Cut, context),
        KeyBinding::new("cmd-v", Paste, context),
        KeyBinding::new("cmd-z", Undo, context),
        KeyBinding::new("cmd-shift-z", Redo, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
    ]);
}

pub(crate) fn bind_navigation_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrevious, None),
    ]);
    for context in [
        MENU_CONTEXT,
        POPOVER_CONTEXT,
        MODAL_CONTEXT,
        TEXT_INPUT_CONTEXT,
    ] {
        cx.bind_keys([KeyBinding::new("escape", Cancel, Some(context))]);
    }
    let menu = Some(MENU_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, menu),
        KeyBinding::new("down", SelectNext, menu),
        KeyBinding::new("ctrl-p", SelectPrevious, menu),
        KeyBinding::new("ctrl-n", SelectNext, menu),
        KeyBinding::new("enter", Confirm, menu),
        KeyBinding::new("enter", Confirm, Some(DIALOG_CONTEXT)),
    ]);
}
