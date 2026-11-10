//! Scroll's GPUI component kit: design tokens and the components built on them. The kit knows
//! nothing about Scroll's data model.
//!
//! # Adding a component
//!
//! 1. Write one file in the group it belongs to (`controls/`, `lists/`, `overlays/`,
//!    `display/`, or `emoji/` for the emoji picker and its data) and add it to that group's
//!    `mod.rs`. The file holds:
//!    - the component: `new(id, …)` plus builder methods, no boolean arguments (a builder
//!      method such as `.disabled()` instead), every colour and size from `Theme`;
//!    - if it can be clicked, one `Clickable::new(radius)…apply(element, window, cx)` call
//!      on its root (from `foundation/focus.rs`, exported for the app too): that makes it a
//!      Tab stop with the keyboard focus ring, the pointing hand, its hover and pressed
//!      fills, its click (Space and Return press it), and dims it when disabled. Three parts
//!      stay pointer-only on purpose, because a Tab stop each would bury the controls around
//!      them: calendar day cells (the date field beside the calendar is the keyboard path),
//!      emoji picker cells (the arrow keys move through the grid from the search field) and
//!      the tree's disclosure chevron (keyboard expansion belongs on the row's arrow keys,
//!      which the tree does not have yet);
//!    - if it is a popup (a menu, picker or panel), a plain focusable view that emits
//!      `DismissEvent` when it is done: `PopupHost::show` draws it in a `Popover` at its
//!      anchor and closes it on a click outside or Escape, so it takes no position;
//!    - if it has key bindings: actions, a `pub(crate)` key context const in
//!      `foundation/actions.rs` (or beside the component), and a `bind_keys(cx)` called from
//!      `init` below;
//!    - `pub fn story(&mut Window, &mut App) -> AnyView`, showing it in its states;
//!    - a `tests` module with at least `the_story_renders` (via `open_story`) and a test of
//!      anything it does.
//! 2. Add one `pub use` line below.
//! 3. Add one line to `STORIES` in `examples/gallery/main.rs`.

mod controls;
mod display;
pub mod emoji;
mod foundation;
mod lists;
mod overlays;
pub mod text;
pub mod theme;

pub use controls::button::{Button, ButtonSize, ButtonVariant};
pub use controls::checkbox::Checkbox;
pub use controls::combobox::{Combobox, ComboboxEvent, ComboboxOption};
pub use controls::field_error::FieldError;
pub use controls::form_field::FormField;
pub use controls::icon_button::IconButton;
pub use controls::segmented_control::SegmentedControl;
pub use controls::text_input::{InputAccess, TextInput, TextInputEvent};
pub use display::avatar::Avatar;
pub use display::calendar::{Calendar, CalendarDate, CalendarMonth};
pub use display::divider::Divider;
pub use display::drag_preview::DragPreview;
pub use display::drop_indicator::DropIndicator;
pub use display::icon::{Icon, IconName, IconSize};
pub use display::pill::Pill;
pub use display::pill_row::PillRow;
pub use display::resize_handle::ResizeHandle;
pub use display::scrollbar::{ScrollPosition, ScrollSource, Scrollbar};
pub use display::tabs::Tabs;
pub use display::titlebar_drag_area::TitlebarDragArea;
pub use emoji::picker::{EmojiPicker, EmojiPickerEvent};
pub use foundation::actions;
pub use foundation::assets::Assets;
pub use foundation::ellipsis::Ellipsis;
pub use foundation::focus::{Clickable, tab_navigation};
pub use foundation::story::StoryBuilder;
pub use lists::empty_state::EmptyState;
pub use lists::list_item::ListItem;
pub use lists::section_header::SectionHeader;
pub use lists::tree_item::{Disclosure, TreeItem};
pub use overlays::dialog::Dialog;
pub use overlays::error_bubble::ErrorBubble;
pub use overlays::menu::{Menu, MenuItem};
pub use overlays::modal::Modal;
pub use overlays::modal_layer::ModalLayer;
pub use overlays::popover::Popover;
pub use overlays::popup_host::{PopupAnchor, PopupHost, PopupTrigger};
pub use overlays::toast::Toast;
pub use overlays::toast_host::ToastHost;
pub use overlays::tooltip::Tooltip;
pub use text::TextSelection;
pub use theme::{Appearance, TextSize, Theme};

use gpui::{App, ClickEvent, MouseDownEvent, Window};

pub(crate) type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
pub(crate) type MouseDownHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Installs the themes and key bindings. Call once at startup, before opening windows.
pub fn init(cx: &mut App) {
    theme::Themes::install(cx);
    actions::bind_text_editing_keys(actions::TEXT_INPUT_CONTEXT, cx);
    actions::bind_navigation_keys(cx);
    emoji::picker::bind_keys(cx);
}
