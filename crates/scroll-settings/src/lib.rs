//! Preferences: each one a typed `Setting` declared as a `const` next to the code that uses it,
//! kept in one JSON file, and listed on the settings page under its category.
//!
//! # Adding a setting
//!
//! 1. Declare it where it is used:
//!    ```ignore
//!    pub const LINE_NUMBERS: Setting<bool> = Setting {
//!        key: "appearance.line_numbers",
//!        category: Category::Appearance,
//!        label: "Line numbers",
//!        description: "Numbers the lines of code blocks.",
//!        default: false,
//!    };
//!    ```
//!    A toggle is a `bool`; a choice is a fieldless enum implementing `Choice`. A new section is
//!    one more `Category`.
//! 2. List it in its feature's slice of settings, which the app passes to `register` at startup.
//!    Its row then appears on the settings page.
//! 3. Read it with `LINE_NUMBERS.get(cx)`, change it with `LINE_NUMBERS.set(value, cx)`, and
//!    follow changes with `cx.observe_global::<Settings>(…)`.
//!
//! A preference whose entries are not known in advance, such as a shortcut per command, is a
//! `SettingMap` instead: a string per name, under one key, that the page does not list by
//! itself.

mod settings;

use gpui::App;
use serde_json::{Map, Value};

pub use settings::{Settings, init, register};

/// The settings page's sections, in the order it shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Appearance,
    Integrations,
    /// Drawn by the app from its commands rather than from registered settings.
    Shortcuts,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Integrations => "Integrations",
            Category::Shortcuts => "Shortcuts",
        }
    }
}

/// One preference. `key` is what the file stores it under, so it must never change.
pub struct Setting<T> {
    pub key: &'static str,
    pub category: Category,
    pub label: &'static str,
    pub description: &'static str,
    /// Used while the file has no value for it, or one this version cannot read.
    pub default: T,
}

impl<T: SettingValue> Setting<T> {
    pub fn get(&self, cx: &App) -> T {
        cx.try_global::<Settings>()
            .and_then(|settings| settings.stored(self.key))
            .and_then(T::from_json)
            .unwrap_or(self.default)
    }

    /// Takes effect at once (observers of `Settings` run) and is saved off the main thread.
    pub fn set(&self, value: T, cx: &mut App) {
        Settings::store(self.key, value.to_json(), cx);
    }
}

/// A string per name, kept as one object under `key`; for preferences such as a shortcut per
/// command, where a fixed list of `Setting`s does not fit.
pub struct SettingMap {
    /// What the file stores it under, so it must never change.
    pub key: &'static str,
}

impl SettingMap {
    /// The value stored for `name`; `None` if there is none, or if what the file holds is not a
    /// string in an object (which this version cannot read, so it reads as nothing set).
    pub fn get<'a>(&self, name: &str, cx: &'a App) -> Option<&'a str> {
        cx.try_global::<Settings>()?
            .stored(self.key)?
            .as_object()?
            .get(name)?
            .as_str()
    }

    /// Takes effect at once and is saved off the main thread, as `Setting::set` does.
    pub fn set(&self, name: &str, value: &str, cx: &mut App) {
        let mut entries = self.entries(cx);
        entries.insert(name.to_string(), Value::String(value.to_string()));
        Settings::store(self.key, Value::Object(entries), cx);
    }

    pub fn remove(&self, name: &str, cx: &mut App) {
        let mut entries = self.entries(cx);
        if entries.remove(name).is_some() {
            Settings::store(self.key, Value::Object(entries), cx);
        }
    }

    /// Every stored entry; a value that is not an object is replaced by the next change.
    fn entries(&self, cx: &App) -> Map<String, Value> {
        cx.try_global::<Settings>()
            .and_then(|settings| settings.stored(self.key))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }
}

/// A type a setting can hold, and how it is stored and shown.
pub trait SettingValue: Copy + PartialEq + Sync + 'static {
    fn to_json(self) -> Value;
    /// `None` for a value this type cannot take, which then reads as the default.
    fn from_json(value: &Value) -> Option<Self>;
    fn control(self) -> Control;
}

/// A setting with a fixed list of options, shown as a segmented control.
pub trait Choice: Copy + PartialEq + Sync + 'static {
    /// Every option, in the order the page shows them.
    const OPTIONS: &'static [Self];
    /// What the file stores; must never change.
    fn id(self) -> &'static str;
    fn label(self) -> &'static str;
}

impl SettingValue for bool {
    fn to_json(self) -> Value {
        Value::Bool(self)
    }

    fn from_json(value: &Value) -> Option<Self> {
        value.as_bool()
    }

    fn control(self) -> Control {
        Control::Toggle { is_on: self }
    }
}

impl<T: Choice> SettingValue for T {
    fn to_json(self) -> Value {
        Value::String(self.id().to_string())
    }

    fn from_json(value: &Value) -> Option<Self> {
        let id = value.as_str()?;
        T::OPTIONS.iter().copied().find(|option| option.id() == id)
    }

    fn control(self) -> Control {
        Control::Choice {
            options: T::OPTIONS
                .iter()
                .map(|option| ChoiceOption {
                    id: option.id(),
                    label: option.label(),
                })
                .collect(),
            selected: T::OPTIONS
                .iter()
                .position(|option| *option == self)
                .unwrap_or_default(),
        }
    }
}

/// What the settings page draws for a setting, in its current state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Toggle {
        is_on: bool,
    },
    Choice {
        options: Vec<ChoiceOption>,
        selected: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    pub id: &'static str,
    pub label: &'static str,
}

/// A setting of any type, so the settings page can list and change every registered one.
pub trait AnySetting: Sync {
    fn key(&self) -> &'static str;
    fn category(&self) -> Category;
    fn label(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn control(&self, cx: &App) -> Control;
    /// Sets it from its stored form: a bool for a toggle, an option's id for a choice. A value of
    /// the wrong form is a mistake in the caller, logged and ignored.
    fn set_json(&self, value: &Value, cx: &mut App);
}

impl<T: SettingValue> AnySetting for Setting<T> {
    fn key(&self) -> &'static str {
        self.key
    }

    fn category(&self) -> Category {
        self.category
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn control(&self, cx: &App) -> Control {
        self.get(cx).control()
    }

    fn set_json(&self, value: &Value, cx: &mut App) {
        match T::from_json(value) {
            Some(value) => self.set(value, cx),
            None => log::warn!("{value} is not a value of setting {}", self.key),
        }
    }
}
