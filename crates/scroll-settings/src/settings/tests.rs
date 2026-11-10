use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::TestAppContext;

use super::*;
use crate::{Choice, ChoiceOption, Control, Setting};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shade {
    Light,
    Dark,
}

impl Choice for Shade {
    const OPTIONS: &'static [Self] = &[Shade::Light, Shade::Dark];

    fn id(self) -> &'static str {
        match self {
            Shade::Light => "light",
            Shade::Dark => "dark",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Shade::Light => "Light",
            Shade::Dark => "Dark",
        }
    }
}

const SHADE: Setting<Shade> = Setting {
    key: "test.shade",
    category: Category::Appearance,
    label: "Shade",
    description: "How dark things are.",
    default: Shade::Light,
};

const IS_SYNCED: Setting<bool> = Setting {
    key: "test.synced",
    category: Category::Integrations,
    label: "Sync",
    description: "Keeps things in step.",
    default: false,
};

const IS_COMPACT: Setting<bool> = Setting {
    key: "test.compact",
    category: Category::Appearance,
    label: "Compact",
    description: "Tighter rows.",
    default: true,
};

/// A file of its own per test, removed when dropped.
struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        TemporaryFile(std::env::temp_dir().join(format!(
            "scroll-preferences-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn read(&self) -> Map<String, Value> {
        serde_json::from_str(&std::fs::read_to_string(&self.0).unwrap()).unwrap()
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

#[gpui::test]
fn values_read_as_defaults_until_set_and_survive_a_restart(cx: &mut TestAppContext) {
    let file = TemporaryFile::new();
    cx.update(|cx| init(file.0.clone(), cx));
    assert_eq!(cx.update(|cx| SHADE.get(cx)), Shade::Light);
    assert!(!cx.update(|cx| IS_SYNCED.get(cx)));

    cx.update(|cx| {
        SHADE.set(Shade::Dark, cx);
        IS_SYNCED.set(true, cx);
    });
    cx.run_until_parked();
    cx.update(|cx| init(file.0.clone(), cx));

    assert_eq!(cx.update(|cx| SHADE.get(cx)), Shade::Dark);
    assert!(cx.update(|cx| IS_SYNCED.get(cx)));
    assert!(cx.update(|cx| IS_COMPACT.get(cx)));
}

#[gpui::test]
fn values_this_version_cannot_read_fall_back_to_defaults_and_unknown_keys_are_kept(
    cx: &mut TestAppContext,
) {
    let file = TemporaryFile::new();
    std::fs::write(
        &file.0,
        r#"{"test.shade": "purple", "test.synced": 3, "later.feature": {"on": true}}"#,
    )
    .unwrap();
    cx.update(|cx| init(file.0.clone(), cx));

    assert_eq!(cx.update(|cx| SHADE.get(cx)), Shade::Light);
    assert!(!cx.update(|cx| IS_SYNCED.get(cx)));

    cx.update(|cx| IS_SYNCED.set(true, cx));
    cx.run_until_parked();
    let saved = file.read();
    assert_eq!(saved["test.synced"], Value::Bool(true));
    assert_eq!(saved["later.feature"], serde_json::json!({"on": true}));
}

#[gpui::test]
fn a_broken_file_reads_as_defaults_and_is_replaced_on_the_next_save(cx: &mut TestAppContext) {
    let file = TemporaryFile::new();
    std::fs::write(&file.0, "{ not json").unwrap();
    cx.update(|cx| init(file.0.clone(), cx));
    assert_eq!(cx.update(|cx| SHADE.get(cx)), Shade::Light);

    cx.update(|cx| SHADE.set(Shade::Dark, cx));
    cx.run_until_parked();

    assert_eq!(file.read()["test.shade"], Value::String("dark".into()));
}

#[gpui::test]
fn the_page_lists_settings_by_category_and_changes_them_without_knowing_their_types(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| register(&[&IS_SYNCED, &SHADE, &IS_COMPACT], cx));

    let sections = cx.update(|cx| Settings::sections(cx));
    let keys: Vec<(Category, Vec<&str>)> = sections
        .iter()
        .map(|(category, settings)| {
            let keys = settings.iter().map(|setting| setting.key()).collect();
            (*category, keys)
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            (Category::Appearance, vec!["test.shade", "test.compact"]),
            (Category::Integrations, vec!["test.synced"]),
        ]
    );

    let shade = sections[0].1[0];
    assert_eq!(
        cx.update(|cx| shade.control(cx)),
        Control::Choice {
            options: vec![
                ChoiceOption {
                    id: "light",
                    label: "Light"
                },
                ChoiceOption {
                    id: "dark",
                    label: "Dark"
                },
            ],
            selected: 0,
        }
    );
    cx.update(|cx| shade.set_json(&Value::String("dark".into()), cx));
    cx.update(|cx| shade.set_json(&Value::Bool(true), cx));
    assert_eq!(cx.update(|cx| SHADE.get(cx)), Shade::Dark);

    let compact = sections[0].1[1];
    cx.update(|cx| compact.set_json(&Value::Bool(false), cx));
    assert_eq!(
        cx.update(|cx| compact.control(cx)),
        Control::Toggle { is_on: false }
    );
}

#[gpui::test]
fn observers_hear_about_every_change(cx: &mut TestAppContext) {
    let changes = std::rc::Rc::new(std::cell::Cell::new(0));
    let heard = changes.clone();
    let _subscription =
        cx.update(|cx| cx.observe_global::<Settings>(move |_| heard.set(heard.get() + 1)));
    cx.run_until_parked();
    let before = changes.get();

    cx.update(|cx| IS_SYNCED.set(true, cx));
    cx.run_until_parked();

    assert_eq!(changes.get(), before + 1);
}

#[cfg(debug_assertions)]
#[gpui::test]
#[should_panic(expected = "two settings are called test.shade")]
fn registering_a_key_twice_is_a_mistake(cx: &mut TestAppContext) {
    cx.update(|cx| register(&[&SHADE, &SHADE], cx));
}
