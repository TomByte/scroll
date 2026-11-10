//! A sample database in a temporary workspace, shown in a table window in the system appearance.
//!
//! `cargo run -p scroll-table --example table -- [ROWS] [--measure]`
//! ROWS is 100 (default) or any count, e.g. 100000. `--measure` scrolls through the table
//! programmatically, prints frame timings and quits. `--fixture PATH` opens a copy of a workspace
//! built by scroll-core's `fixture` example instead, showing its biggest database.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, Focusable, IntoElement, KeyBinding,
    ParentElement, Render, Styled, Subscription, Window, WindowBounds, WindowOptions, actions, div,
    px, size,
};
use scroll_core::id::{DatabaseId, PageId, PropertyId, SelectOptionId, ViewId};
use scroll_core::operation::{Operation, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{
    Condition, Database, Date, FilterCondition, Page, Property, PropertyType, PropertyValue,
    SelectOption, SortDirection, SortKey, View, ViewType,
};
use scroll_core::store::WorkspaceStore;
use scroll_table::{TableEvent, TableView};

actions!(table_example, [Quit]);

const MEASURED_FRAMES: usize = 600;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let row_count = arguments
        .iter()
        .find_map(|argument| argument.parse::<usize>().ok())
        .unwrap_or(100);
    let measures = arguments.iter().any(|argument| argument == "--measure");
    let measures_sorting = arguments
        .iter()
        .any(|argument| argument == "--measure-sort");
    // `--appearance light|dark` pins the window's appearance; otherwise it follows the system.
    let appearance = match arguments
        .iter()
        .position(|argument| argument == "--appearance")
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
    {
        Some("light") => Some(scroll_ui::Appearance::Light),
        Some("dark") => Some(scroll_ui::Appearance::Dark),
        _ => None,
    };
    // `--show NAME` sets up something to look at once rows have loaded: sort, filter, properties,
    // column-menu, select, multi-select, date, number, text or middle.
    let shown = arguments
        .iter()
        .position(|argument| argument == "--show")
        .and_then(|index| arguments.get(index + 1))
        .cloned();

    let fixture = arguments
        .iter()
        .position(|argument| argument == "--fixture")
        .and_then(|index| arguments.get(index + 1))
        .map(PathBuf::from);
    let path = temporary_workspace_path();
    let started = Instant::now();
    let sample = match &fixture {
        Some(fixture) => fixture_sample(fixture, &path),
        None => sample_workspace(&path, row_count),
    };
    println!(
        "prepared the workspace in {:?} at {}",
        started.elapsed(),
        path.display()
    );

    let path_for_window = path.clone();
    Application::new()
        .with_assets(scroll_ui::Assets)
        .run(move |cx: &mut App| {
            scroll_ui::init(cx);
            scroll_table::init(cx);
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.0), px(800.0)),
                    cx,
                ))),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, |window, cx| {
                if let Some(appearance) = appearance {
                    scroll_ui::theme::force_window_appearance(appearance, window, cx);
                }
                cx.new(|cx| TableWindow::new(path_for_window.clone(), sample.clone(), window, cx))
            });
            match opened {
                Ok(handle) if measures_sorting => {
                    handle
                        .update(cx, |root, window, _| root.measure_sorting(window))
                        .ok();
                }
                Ok(handle) if measures => {
                    handle
                        .update(cx, |root, window, cx| root.measure_scrolling(window, cx))
                        .ok();
                }
                Ok(handle) if shown.is_some() => {
                    let name = shown.clone().unwrap_or_default();
                    handle
                        .update(cx, |root, window, _| root.show_when_loaded(name, window))
                        .ok();
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("could not open the table window: {error:#}");
                    cx.quit();
                }
            }
            cx.activate(true);
        });
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        std::fs::remove_file(file).ok();
    }
}

/// A copy of a benchmark workspace and its biggest database's "All" view. Estimate is the third
/// property, as in the built-in sample, so `--measure-sort` sorts by it.
fn fixture_sample(fixture: &Path, path: &Path) -> Sample {
    std::fs::copy(fixture, path).expect("the fixture can be copied");
    let store = WorkspaceStore::open(path).expect("the fixture opens");
    let children = store
        .page_children(None, None, 100_000)
        .expect("the fixture's pages read");
    let databases = children.iter().filter_map(|child| {
        store
            .database_hosted_by(child.page.id)
            .expect("databases read")
    });
    let database = databases
        .max_by_key(|database| {
            let views = store.views(database.id).expect("views read");
            store
                .view_row_ids(views[0].id)
                .map_or(0, |ids| ids.rows.len())
        })
        .expect("the fixture has a database");
    let view = store.views(database.id).expect("views read").remove(0);
    let named = |name: &str| {
        database
            .properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| property.id)
            .expect("the fixture's properties are known")
    };
    Sample {
        view: view.id,
        properties: [
            "Status", "Labels", "Estimate", "Due", "Done", "Link", "Notes",
        ]
        .into_iter()
        .map(named)
        .collect(),
    }
}

/// What the sample workspace holds that the example refers to.
#[derive(Clone)]
struct Sample {
    view: ViewId,
    /// Status, Tags, Estimate, Due, Shipped, Link, Notes.
    properties: Vec<PropertyId>,
}

struct TableWindow {
    table: Entity<TableView>,
    sample: Sample,
    _subscriptions: Vec<Subscription>,
}

impl TableWindow {
    fn new(path: PathBuf, sample: Sample, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| TableView::new(path, sample.view, cx));
        window.focus(&table.focus_handle(cx));
        let opened = cx.subscribe(&table, |_, _, event: &TableEvent, _| match event {
            TableEvent::OpenPage(page) => println!("open page {}", page.0),
            TableEvent::Changed => {}
        });
        TableWindow {
            table,
            sample,
            _subscriptions: vec![opened, scroll_ui::theme::follow_system_appearance(window)],
        }
    }

    /// Waits for the first load, then sets up `name` (see `--show` in `main`).
    fn show_when_loaded(&mut self, name: String, window: &mut Window) {
        let table = self.table.clone();
        let sample = self.sample.clone();
        wait_for_rows(table.clone(), window, move |window, cx| {
            show(&name, &table, &sample, window, cx)
        });
    }

    /// Changes the sort every few seconds while revealing rows, and reports frame timings and
    /// how long each re-sort took to arrive.
    fn measure_sorting(&mut self, window: &mut Window) {
        let table = self.table.clone();
        let estimate = self.sample.properties[2];
        wait_for_rows(table.clone(), window, move |window, _| {
            let measurement = Rc::new(RefCell::new(SortMeasurement::default()));
            sort_frame(
                SortRun {
                    table,
                    estimate,
                    measurement,
                },
                Instant::now(),
                window,
            );
        });
    }

    /// Reveals rows further down every frame and reports how long frames took.
    fn measure_scrolling(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let frames = Rc::new(RefCell::new(Vec::with_capacity(MEASURED_FRAMES)));
        schedule_frame(self.table.clone(), frames, Instant::now(), window);
        cx.notify();
    }
}

fn schedule_frame(
    table: Entity<TableView>,
    frames: Rc<RefCell<Vec<Duration>>>,
    previous: Instant,
    window: &mut Window,
) {
    window.on_next_frame(move |window, cx| {
        let now = Instant::now();
        frames.borrow_mut().push(now - previous);
        let frame = frames.borrow().len();
        if frame >= MEASURED_FRAMES {
            report(&frames.borrow());
            cx.quit();
            return;
        }
        table.update(cx, |table, cx| {
            let row = (frame * 37) % table.row_count().max(1);
            table.reveal_row(row, cx);
        });
        schedule_frame(table, frames, now, window);
        window.refresh();
    });
}

fn report(frames: &[Duration]) {
    let mut sorted: Vec<Duration> = frames.iter().skip(10).copied().collect();
    sorted.sort();
    let at = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction) as usize];
    println!(
        "frame intervals over {} frames: median {:?}, p95 {:?}, p99 {:?}, max {:?}",
        sorted.len(),
        at(0.5),
        at(0.95),
        at(0.99),
        sorted.last().copied().unwrap_or_default()
    );
    let long: Vec<(usize, Duration)> = frames
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, frame)| *frame > Duration::from_millis(12))
        .collect();
    println!("frames over 12 ms (index, length): {long:?}");
}

impl Render for TableWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.table.clone())
    }
}

fn temporary_workspace_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "scroll-table-example-{}.scroll",
        uuid::Uuid::now_v7()
    ))
}

/// A database with one property of every type and `row_count` rows of varied values.
/// The example's own connection builds the data and is closed; the table opens its own.
fn sample_workspace(path: &Path, row_count: usize) -> Sample {
    let mut store = WorkspaceStore::open(path).expect("a temporary workspace opens");
    let host = page("Projects", None, None);
    let options = |names: &[&str]| -> Vec<SelectOption> {
        names
            .iter()
            .map(|name| SelectOption {
                id: SelectOptionId::generate(),
                name: (*name).into(),
            })
            .collect()
    };
    let property = |name: &str, property_type, options| Property {
        id: PropertyId::generate(),
        name: name.into(),
        property_type,
        options,
    };
    let status = property(
        "Status",
        PropertyType::Select,
        options(&["Not started", "In progress", "Done", "Blocked"]),
    );
    let tags = property(
        "Tags",
        PropertyType::MultiSelect,
        options(&["Design", "Backend", "Frontend", "Research", "Urgent"]),
    );
    let estimate = property("Estimate", PropertyType::Number, Vec::new());
    let due = property("Due", PropertyType::Date, Vec::new());
    let done = property("Shipped", PropertyType::Checkbox, Vec::new());
    let link = property("Link", PropertyType::Url, Vec::new());
    let notes = property("Notes", PropertyType::Text, Vec::new());
    let properties = vec![
        status.clone(),
        tags.clone(),
        estimate.clone(),
        due.clone(),
        done.clone(),
        link.clone(),
        notes.clone(),
    ];
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: properties.clone(),
    };
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All projects".into(),
        view_type: ViewType::Table,
        filter: Vec::new(),
        sort: Vec::new(),
        visible_columns: properties.iter().map(|property| property.id).collect(),
        column_widths: Vec::new(),
    };
    let mut operations = vec![
        Operation::Create(Record::Page(host.clone())),
        Operation::Create(Record::Database(database.clone())),
        Operation::Create(Record::View(view.clone())),
    ];
    let mut position = Position::between(None, None).expect("an empty range has room");
    let words = [
        "Atlas", "Beacon", "Cascade", "Delta", "Ember", "Fjord", "Garnet", "Harbor", "Iris",
        "Juniper",
    ];
    for index in 0..row_count {
        let seed = index.wrapping_mul(2_654_435_761) % 1_000_003;
        let mut row = page(
            &format!(
                "{} {} {}",
                words[seed % 10],
                words[(seed / 10) % 10],
                index + 1
            ),
            Some(host.id),
            Some(database.id),
        );
        row.position = position.clone();
        position = Position::between(Some(&position), None).expect("there is always room after");
        let values = [
            (
                status.id,
                PropertyValue::Select(status.options[seed % 4].id),
            ),
            (
                tags.id,
                PropertyValue::MultiSelect(
                    (0..1 + seed % 3)
                        .map(|offset| tags.options[(seed + offset * 2) % 5].id)
                        .collect(),
                ),
            ),
            (
                estimate.id,
                PropertyValue::Number(((seed % 400) as f64) / 4.0),
            ),
            (
                due.id,
                PropertyValue::Date(Date {
                    year: 2026,
                    month: 1 + (seed % 12) as u8,
                    day: 1 + (seed % 28) as u8,
                }),
            ),
            (done.id, PropertyValue::Checkbox(seed % 3 == 0)),
            (
                link.id,
                PropertyValue::Url(format!("https://example.com/projects/{}", index + 1)),
            ),
            (
                notes.id,
                PropertyValue::Text(format!("Review with {} team", words[(seed / 7) % 10])),
            ),
        ];
        let row_id = row.id;
        operations.push(Operation::Create(Record::Page(row)));
        operations.extend(values.into_iter().map(|(property, value)| {
            Operation::Set(scroll_core::operation::FieldChange::PropertyValue {
                page: row_id,
                property,
                value: Some(value),
            })
        }));
    }
    store
        .apply_local_transaction(&Transaction::new(operations))
        .expect("the sample data is valid");
    Sample {
        view: view.id,
        properties: properties.iter().map(|property| property.id).collect(),
    }
}

fn page(title: &str, parent: Option<PageId>, row_of: Option<DatabaseId>) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: title.into(),
        icon: None,
        position: Position::between(None, None).expect("an empty range has room"),
        row_of,
        is_archived: false,
    }
}

fn wait_for_rows(
    table: Entity<TableView>,
    window: &mut Window,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    window.on_next_frame(move |window, cx| {
        if table.read(cx).is_loading() {
            wait_for_rows(table, window, then);
            window.refresh();
            return;
        }
        then(window, cx);
    });
}

fn show(name: &str, table: &Entity<TableView>, sample: &Sample, window: &mut Window, cx: &mut App) {
    let [status, tags, estimate, due, shipped, _, notes] = sample.properties[..] else {
        return;
    };
    let typed = table.update(cx, |table, cx| match name {
        "sort" => {
            table.change_sort(
                vec![
                    SortKey {
                        property: estimate,
                        direction: SortDirection::Descending,
                    },
                    SortKey {
                        property: status,
                        direction: SortDirection::Ascending,
                    },
                ],
                cx,
            );
            table.open_sort_editor(window, cx);
            ""
        }
        "filter" => {
            let condition = |property, condition| FilterCondition {
                property,
                condition,
            };
            table.change_filter(
                vec![
                    condition(estimate, Condition::NumberGreaterThan(20.0)),
                    condition(notes, Condition::TextContains("team".into())),
                    condition(shipped, Condition::CheckboxIs(false)),
                    condition(
                        due,
                        Condition::DateAfter(Date {
                            year: 2026,
                            month: 3,
                            day: 1,
                        }),
                    ),
                ],
                cx,
            );
            table.open_filter_editor(window, cx);
            ""
        }
        "properties" => {
            table.open_properties_panel(window, cx);
            ""
        }
        "column-menu" => {
            table.open_column_menu(tags, window, cx);
            ""
        }
        "select" => {
            table.edit_cell(2, 1, window, cx);
            ""
        }
        "multi-select" => {
            table.edit_cell(2, 2, window, cx);
            "o p s"
        }
        "date" => {
            table.edit_cell(2, 4, window, cx);
            ""
        }
        "number" => {
            table.edit_cell(2, 3, window, cx);
            "cmd-a a b c enter"
        }
        "text" => {
            table.edit_cell(2, 7, window, cx);
            ""
        }
        "middle" => {
            let middle = table.row_count() / 2;
            table.reveal_row(middle, cx);
            ""
        }
        other => {
            eprintln!("nothing called {other} to show");
            ""
        }
    });
    let keys: Vec<String> = typed.split_whitespace().map(str::to_string).collect();
    window.on_next_frame(move |window, cx| {
        for key in keys {
            if let Ok(keystroke) = gpui::Keystroke::parse(&key) {
                window.dispatch_keystroke(keystroke, cx);
            }
        }
        window.refresh();
    });
}

#[derive(Default)]
struct SortMeasurement {
    frames: Vec<Duration>,
    sort_started: Option<Instant>,
    sort_waits: Vec<Duration>,
}

struct SortRun {
    table: Entity<TableView>,
    estimate: PropertyId,
    measurement: Rc<RefCell<SortMeasurement>>,
}

/// A sort change every this many frames, in the middle of scrolling.
const SORT_EVERY_FRAMES: usize = 240;

fn sort_frame(run: SortRun, previous: Instant, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        let now = Instant::now();
        let is_loading = run.table.read(cx).is_loading();
        let frame = {
            let mut measurement = run.measurement.borrow_mut();
            measurement.frames.push(now - previous);
            if !is_loading && let Some(started) = measurement.sort_started.take() {
                measurement.sort_waits.push(now - started);
            }
            measurement.frames.len()
        };
        if frame >= MEASURED_FRAMES * 2 {
            let measurement = run.measurement.borrow();
            report(&measurement.frames);
            println!("re-sorted rows arrived after: {:?}", measurement.sort_waits);
            cx.quit();
            return;
        }
        run.table.update(cx, |table, cx| {
            if frame % SORT_EVERY_FRAMES == SORT_EVERY_FRAMES / 2 {
                let direction = match (frame / SORT_EVERY_FRAMES) % 2 {
                    0 => SortDirection::Descending,
                    _ => SortDirection::Ascending,
                };
                table.change_sort(
                    vec![SortKey {
                        property: run.estimate,
                        direction,
                    }],
                    cx,
                );
                run.measurement.borrow_mut().sort_started = Some(Instant::now());
            }
            let row = (frame * 37) % table.row_count().max(1);
            table.reveal_row(row, cx);
        });
        sort_frame(run, now, window);
        window.refresh();
    });
}
