//! The table's keyboard actions, listed once: each action, its keys, and the command it runs.
//! The list defines the actions, binds their keys in `init`, routes them in `on_actions`, and
//! gives each action its `KEYS` so the app can show the same shortcut.

use gpui::{App, Context, Div, InteractiveElement, KeyBinding};

use crate::KEY_CONTEXT;
use crate::selection::Movement;
use crate::table_view::{Command, TableView};

macro_rules! table_actions {
    (
        kit { $($kit_action:path: [$($kit_key:literal),+] => $kit_command:expr;)+ }
        $($action:ident: [$($key:literal),+] => $command:expr;)+
    ) => {
        gpui::actions!(table, [$($action),+]);

        $(impl $action {
            /// The keys bound to this action in the table, the first being the one to show.
            pub const KEYS: &'static [&'static str] = &[$($key),+];
        })+

        /// Binds the table's keys. Call once at startup, after `scroll_ui::init`.
        pub fn init(cx: &mut App) {
            let context = Some(KEY_CONTEXT);
            let mut bindings = Vec::new();
            $(bindings.extend($action::KEYS.iter().map(|key| KeyBinding::new(key, $action, context)));)+
            $($(bindings.push(KeyBinding::new($kit_key, $kit_action, context));)+)+
            cx.bind_keys(bindings);
        }

        pub(crate) fn on_actions(element: Div, cx: &mut Context<TableView>) -> Div {
            element
                $(.on_action(cx.listener(|table, _: &$action, window, cx| {
                    table.perform($command, window, cx)
                })))+
                $(.on_action(cx.listener(|table, _: &$kit_action, window, cx| {
                    table.perform($kit_command, window, cx)
                })))+
        }
    };
}

table_actions! {
    kit {
        scroll_ui::actions::Cancel: ["escape"] => Command::Cancel;
    }
    MoveUp: ["up"] => Command::Move(Movement::Up);
    MoveDown: ["down"] => Command::Move(Movement::Down);
    MoveLeft: ["left"] => Command::Move(Movement::Left);
    MoveRight: ["right"] => Command::Move(Movement::Right);
    MoveNext: ["tab"] => Command::Move(Movement::Next);
    MovePrevious: ["shift-tab"] => Command::Move(Movement::Previous);
    ExtendUp: ["shift-up"] => Command::Extend(Movement::Up);
    ExtendDown: ["shift-down"] => Command::Extend(Movement::Down);
    ExtendLeft: ["shift-left"] => Command::Extend(Movement::Left);
    ExtendRight: ["shift-right"] => Command::Extend(Movement::Right);
    StartEditing: ["enter"] => Command::StartEditing;
    CopyCell: ["cmd-c"] => Command::CopyCells;
    PasteCells: ["cmd-v"] => Command::PasteCells;
    ClearCell: ["backspace", "delete"] => Command::ClearCells;
    DeleteRows: ["cmd-backspace"] => Command::DeleteRows;
    ToggleRowSelection: ["shift-space"] => Command::ToggleSelectedRow;
    OpenRow: ["cmd-enter"] => Command::OpenRow;
}
