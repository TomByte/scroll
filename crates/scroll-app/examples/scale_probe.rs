//! Runs the real app on a large workspace and measures it: launch to first frame and to a
//! shown page, memory, and frame times while a script of steps runs. Build in release, without
//! GPUI's `test-support` feature, for numbers that mean anything.
//!
//! `scale_probe [--welcome] DATA_DIR [STEP]...`
//!
//! The first run builds the workspace in `DATA_DIR/Scale.scroll` (the core fixture at its scale
//! target, or one database of `SCROLL_PROBE_ROWS` rows, plus a "Huge folder" page with 50,000
//! children) and a `workspaces.json` naming it. With `--welcome` it builds nothing, so a new
//! data directory opens on the welcome pages, as a first launch does.
//! Steps are keystrokes (`cmd-p`), `type:<text>`, `wait:<ms>`, `command:<id>`,
//! `phase:<name>` (starts a new measured phase and prints memory) and `quit`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, App, Application, Keystroke, Window};
use scroll_app::app;
use scroll_core::id::PageId;
use scroll_core::operation::{Operation, Record, Transaction};
use scroll_core::position::Position;
use scroll_core::record::Page;
use scroll_core::store::WorkspaceStore;
use scroll_core::store::fixture::{FixtureSize, generate_workspace};

const STEP_INTERVAL: Duration = Duration::from_millis(400);
const HUGE_FOLDER_CHILDREN: usize = 50_000;
/// How long after launch the main thread is watched, and how long a busy spell must be to count.
const WATCH_FOR: Duration = Duration::from_millis(1_500);
const BUSY_SPELL: Duration = Duration::from_millis(8);

/// Frame start times and phase boundaries, filled from the UI thread.
#[derive(Default)]
struct Recording {
    frames: Vec<Instant>,
    phases: Vec<(String, Instant)>,
}

fn recording() -> &'static Mutex<Recording> {
    static RECORDING: OnceLock<Mutex<Recording>> = OnceLock::new();
    RECORDING.get_or_init(Mutex::default)
}

fn main() {
    let started = Instant::now();
    let mut arguments = std::env::args().skip(1).peekable();
    let opens_welcome = arguments
        .next_if(|argument| argument == "--welcome")
        .is_some();
    let Some(data) = arguments.next().map(PathBuf::from) else {
        eprintln!("usage: scale_probe [--welcome] DATA_DIR [STEP]...");
        return;
    };
    let prepared = if opens_welcome {
        std::fs::create_dir_all(&data).map_err(Into::into)
    } else {
        prepare(&data)
    };
    if let Err(error) = prepared {
        eprintln!("could not build the workspace: {error}");
        return;
    }
    let steps: Vec<String> = arguments.collect();
    let launched = Instant::now();
    println!("prepare: {:?}", launched - started);
    // Wall-clock time, so screenshots taken from outside can be placed on the same timeline.
    if let Ok(since_epoch) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        println!("launched at unix {:.3}", since_epoch.as_secs_f64());
    }
    Application::new()
        .with_assets(scroll_ui::Assets)
        .run(move |cx: &mut App| {
            // The app's own "after launch" timings count from here, not from process start.
            println!("timing: app started {:?} after launch", launched.elapsed());
            watch_main_thread(launched, cx);
            scroll_app::logger::install();
            app::start(data.clone(), cx);
            cx.activate(true);
            mark_phase("launch", launched);
            cx.spawn(async move |cx| {
                let window = loop {
                    let found = cx.update(|cx| app::main_window(cx)).ok().flatten();
                    if let Some(window) = found {
                        break window;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                };
                println!("timing: window found {:?} after launch", launched.elapsed());
                let first_frame = cx.update(|cx| {
                    window
                        .update(cx, |_, window, _| record_frames(window))
                        .is_ok()
                });
                if first_frame.is_err() {
                    return;
                }
                print_memory("after launch");
                for step in steps {
                    run_step(&step, window, cx).await;
                }
            })
            .detach();
        });
}

/// Prints each spell after launch in which the main thread could not run a 1 ms timer for longer
/// than `BUSY_SPELL`, to show what holds back the first frame.
fn watch_main_thread(launched: Instant, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = Instant::now();
        while launched.elapsed() < WATCH_FOR {
            cx.background_executor()
                .timer(Duration::from_millis(1))
                .await;
            let now = Instant::now();
            if now - last > BUSY_SPELL {
                println!(
                    "timing: main thread busy {:?} until {:?} after launch",
                    now - last,
                    now - launched
                );
            }
            last = now;
        }
    })
    .detach();
}

fn mark_phase(name: &str, at: Instant) {
    if let Ok(mut recording) = recording().lock() {
        if let Some((_, launched)) = recording.phases.first() {
            println!("phase {name} at {:?} after launch", at - *launched);
        }
        recording.phases.push((name.to_string(), at));
    }
}

/// Records the start of every frame, asking for the next one each time so idle time counts.
fn record_frames(window: &mut Window) {
    window.on_next_frame(|window, _| {
        if let Ok(mut recording) = recording().lock() {
            recording.frames.push(Instant::now());
        }
        window.refresh();
        record_frames(window);
    });
}

async fn run_step(step: &str, window: AnyWindowHandle, cx: &mut gpui::AsyncApp) {
    if let Some(milliseconds) = step.strip_prefix("wait:").and_then(|ms| ms.parse().ok()) {
        cx.background_executor()
            .timer(Duration::from_millis(milliseconds))
            .await;
        return;
    }
    if let Some(name) = step.strip_prefix("phase:") {
        mark_phase(name, Instant::now());
        print_memory(name);
        return;
    }
    if step == "quit" {
        report();
        let _ = cx.update(|cx| cx.quit());
        return;
    }
    let dispatched = cx.update(|cx| {
        window.update(cx, |_, window, cx| {
            if let Some(id) = step.strip_prefix("command:") {
                match app::command_action(id) {
                    Some(action) => window.dispatch_action(action, cx),
                    None => eprintln!("no command {id}"),
                }
                return;
            }
            for keystroke in keystrokes_for(step) {
                window.dispatch_keystroke(keystroke, cx);
            }
        })
    });
    if dispatched.is_err() {
        eprintln!("could not run {step:?}");
    }
    cx.background_executor().timer(STEP_INTERVAL).await;
}

fn keystrokes_for(step: &str) -> Vec<Keystroke> {
    if let Some(text) = step.strip_prefix("type:") {
        return text
            .chars()
            .filter_map(|character| {
                let key = if character == ' ' {
                    "space".to_string()
                } else {
                    character.to_string()
                };
                let mut keystroke = Keystroke::parse(&key).ok()?;
                keystroke.key_char = Some(character.to_string());
                Some(keystroke)
            })
            .collect();
    }
    step.split(' ')
        .filter_map(|key| match Keystroke::parse(key) {
            Ok(keystroke) => Some(keystroke),
            Err(error) => {
                eprintln!("skipping unknown keystroke {key:?}: {error}");
                None
            }
        })
        .collect()
}

/// Prints, per phase, the first frame's delay and the frame interval spread.
fn report() {
    let Ok(recording) = recording().lock() else {
        return;
    };
    let mut boundaries = recording.phases.clone();
    boundaries.push(("end".into(), Instant::now()));
    for pair in boundaries.windows(2) {
        let ((name, start), (_, end)) = (&pair[0], &pair[1]);
        let frames: Vec<Instant> = recording
            .frames
            .iter()
            .copied()
            .filter(|frame| frame >= start && frame < end)
            .collect();
        let Some(first) = frames.first() else {
            println!("{name}: no frames");
            continue;
        };
        let mut intervals: Vec<f64> = frames
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
            .collect();
        intervals.sort_by(f64::total_cmp);
        let percentile = |p: f64| {
            intervals
                .get(((intervals.len() as f64 - 1.0) * p) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        let long = intervals
            .iter()
            .filter(|interval| **interval > 16.7)
            .count();
        println!(
            "{name}: first frame after {:.1} ms; {} frames; interval median {:.2} ms, p95 {:.2} ms, max {:.2} ms; {} over 16.7 ms",
            (*first - *start).as_secs_f64() * 1000.0,
            frames.len(),
            percentile(0.5),
            percentile(0.95),
            intervals.last().copied().unwrap_or(0.0),
            long
        );
    }
}

fn print_memory(label: &str) {
    let pid = std::process::id().to_string();
    match std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
    {
        Ok(output) => {
            let kilobytes = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(0);
            println!("memory {label}: {} MB resident", kilobytes / 1024);
        }
        Err(error) => eprintln!("could not read memory: {error}"),
    }
}

/// Builds the workspace and registry on first use.
fn prepare(data: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data)?;
    let path = data.join("Scale.scroll");
    if !path.exists() {
        let building = Instant::now();
        // SCROLL_PROBE_ROWS=N builds one database of N rows instead of the full target.
        let size = match std::env::var("SCROLL_PROBE_ROWS")
            .ok()
            .and_then(|rows| rows.parse().ok())
        {
            Some(rows) => FixtureSize::one_database(rows),
            None => FixtureSize::target(),
        };
        generate_workspace(&path, &size, 1)?;
        println!("fixture built in {:?}", building.elapsed());
        add_huge_folder(&path)?;
    }
    let registry = data.join("workspaces.json");
    if !registry.exists() {
        std::fs::write(
            registry,
            r#"{"workspaces":[{"name":"Scale","file":"Scale.scroll","server":null}],"current":0}"#,
        )?;
    }
    Ok(())
}

/// A top-level page with 50,000 children, which the fixture's tree does not have.
fn add_huge_folder(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let building = Instant::now();
    let mut store = WorkspaceStore::open(path)?;
    let folder = page("Huge folder", None, Position::between(None, None)?);
    let folder_id = folder.id;
    store.apply_local_transaction(&Transaction::new(vec![Operation::Create(Record::Page(
        folder,
    ))]))?;
    let mut position = Position::between(None, None)?;
    let mut operations = Vec::new();
    for index in 0..HUGE_FOLDER_CHILDREN {
        operations.push(Operation::Create(Record::Page(page(
            &format!("Item {index:05}"),
            Some(folder_id),
            position.clone(),
        ))));
        position = Position::between(Some(&position), None)?;
        if operations.len() == 1_000 {
            store.apply_local_transaction(&Transaction::new(std::mem::take(&mut operations)))?;
        }
    }
    if !operations.is_empty() {
        store.apply_local_transaction(&Transaction::new(operations))?;
    }
    println!("huge folder built in {:?}", building.elapsed());
    Ok(())
}

fn page(title: &str, parent: Option<PageId>, position: Position) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: title.to_string(),
        icon: None,
        position,
        row_of: None,
        is_archived: false,
    }
}
