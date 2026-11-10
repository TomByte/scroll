//! Measures the editor on a large page in a real window: keystroke handling time, and the
//! interval between frames while typing and while scrolling continuously.
//!
//! `cargo run --release -p scroll-app --example editor_perf -- [blocks] [caret block index]`

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, AppContext, Application, Bounds, Context, Entity, IntoElement, Keystroke,
    ParentElement, Render, Styled, Window, WindowBounds, WindowHandle, WindowOptions, div, px,
    size,
};
use scroll_app::editor::document::PageDocument;
use scroll_app::editor::rich_text::MarkKind;
use scroll_app::editor::{self, PageEditor, Selection};

const KEYSTROKES: usize = 400;
/// Roughly one keystroke per frame at 120 Hz: far faster than anyone types.
const KEYSTROKE_INTERVAL: Duration = Duration::from_millis(8);
const SCROLL_FRAMES: usize = 600;
const SCROLL_STEP: f32 = 45.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Typing,
    Scrolling,
    Done,
}

struct Measurements {
    phase: Phase,
    keystroke_handling: Vec<Duration>,
    typing_frames: Vec<Duration>,
    scrolling_frames: Vec<Duration>,
    last_frame: Option<Instant>,
}

fn main() {
    let block_count: usize = std::env::args()
        .nth(1)
        .and_then(|count| count.parse().ok())
        .unwrap_or(5000);
    Application::new()
        .with_assets(scroll_ui::Assets)
        .run(move |cx: &mut App| {
            scroll_ui::init(cx);
            editor::init(cx);
            let started = Instant::now();
            let page = build_page(block_count);
            println!(
                "built a {block_count}-block page in {:?}",
                started.elapsed()
            );
            let first = page.block_ids()[std::env::args()
                .nth(2)
                .and_then(|index| index.parse().ok())
                .unwrap_or(0)];
            let bounds = Bounds::centered(None, size(px(1000.0), px(900.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, |_, cx| {
                let editor = cx.new(|cx| PageEditor::new(page, Selection::caret(first, 0), cx));
                cx.new(|_| Harness { editor })
            });
            let Ok(window) = opened else {
                eprintln!("could not open a window");
                return cx.quit();
            };
            cx.activate(true);
            run_benchmark(window, cx);
        });
}

/// Hosts the editor the way the app's page view does, rather than as the window's root.
struct Harness {
    editor: Entity<PageEditor>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.editor.clone())
    }
}

fn run_benchmark(window: WindowHandle<Harness>, cx: &mut App) {
    let measurements = Rc::new(RefCell::new(Measurements {
        phase: Phase::Typing,
        keystroke_handling: Vec::new(),
        typing_frames: Vec::new(),
        scrolling_frames: Vec::new(),
        last_frame: None,
    }));
    let focused = window.update(cx, |harness, window, cx| {
        window.focus(harness.editor.read(cx).focus_handle());
        let measurements = measurements.clone();
        window.on_next_frame(move |window, cx| record_frames(measurements, window, cx));
    });
    if let Err(error) = focused {
        eprintln!("{error}");
        return cx.quit();
    }
    // The untyped handle does not lease the root editor, which the keystroke must update.
    let any_window: AnyWindowHandle = window.into();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(500))
            .await;
        for index in 0..KEYSTROKES {
            let typed = any_window.update(cx, |_, window, cx| {
                let character = if index % 12 == 11 { " " } else { "x" };
                let mut keystroke = Keystroke::parse(if character == " " { "space" } else { "x" })
                    .expect("letters and space are valid keystrokes");
                keystroke.key_char = Some(character.into());
                let started = Instant::now();
                let handled = window.dispatch_keystroke(keystroke, cx);
                measurements
                    .borrow_mut()
                    .keystroke_handling
                    .push(started.elapsed());
                if !handled {
                    eprintln!("keystroke {index} was not handled");
                }
            });
            if typed.is_err() {
                return;
            }
            cx.background_executor().timer(KEYSTROKE_INTERVAL).await;
        }
        let counted = window.update(cx, |harness, _, cx| {
            let updates = harness
                .editor
                .update(cx, |editor, _| editor.take_outgoing_updates());
            println!(
                "{KEYSTROKES} keystrokes produced {} Loro updates ({} bytes)",
                updates.len(),
                updates.iter().map(Vec::len).sum::<usize>()
            );
        });
        if counted.is_err() {
            return;
        }
        let mut measurements = measurements.borrow_mut();
        measurements.phase = Phase::Scrolling;
        measurements.last_frame = None;
    })
    .detach();
}

/// Runs once per frame for the whole benchmark: records the interval since the previous
/// frame, scrolls during the scrolling phase, and asks for the next frame.
fn record_frames(measurements: Rc<RefCell<Measurements>>, window: &mut Window, cx: &mut App) {
    let phase = {
        let mut measurements = measurements.borrow_mut();
        let now = Instant::now();
        let interval = measurements
            .last_frame
            .replace(now)
            .map(|previous| now - previous);
        let phase = measurements.phase;
        if let Some(interval) = interval {
            match phase {
                Phase::Typing => measurements.typing_frames.push(interval),
                Phase::Scrolling => measurements.scrolling_frames.push(interval),
                Phase::Done => {}
            }
        }
        if phase == Phase::Scrolling && measurements.scrolling_frames.len() >= SCROLL_FRAMES {
            measurements.phase = Phase::Done;
        }
        measurements.phase
    };
    match phase {
        Phase::Done => {
            report(&measurements.borrow());
            return cx.quit();
        }
        Phase::Scrolling => {
            if let Some(harness) = window.root::<Harness>().flatten() {
                let editor = harness.read(cx).editor.clone();
                editor.update(cx, |editor, cx| editor.scroll_by(px(SCROLL_STEP), cx));
            }
        }
        Phase::Typing => {}
    }
    window.on_next_frame(move |window, cx| record_frames(measurements, window, cx));
    window.refresh();
}

/// Paragraphs of realistic length with a bold word in every other one.
fn build_page(block_count: usize) -> PageDocument {
    let sentence = "The quick brown fox jumps over the lazy dog while the editor keeps every keystroke inside one frame.";
    let paragraphs: Vec<String> = (0..block_count)
        .map(|index| format!("{index}. {sentence}"))
        .collect();
    let paragraphs: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
    let mut page =
        PageDocument::create(1, &paragraphs).expect("building the test page in memory cannot fail");
    for (index, block) in page.block_ids().into_iter().enumerate().step_by(2) {
        let start = format!("{index}. The ").len();
        page.add_mark(block, start..start + 5, MarkKind::Bold)
            .expect("the range lies inside the sentence");
    }
    page.commit();
    page.take_outgoing_updates();
    page
}

fn report(measurements: &Measurements) {
    summarise(
        "keystroke handling (edit + Loro + refresh)",
        &measurements.keystroke_handling,
    );
    summarise("frame interval while typing", &measurements.typing_frames);
    summarise(
        "frame interval while scrolling",
        &measurements.scrolling_frames,
    );
}

fn summarise(label: &str, samples: &[Duration]) {
    if samples.is_empty() {
        return println!("{label}: no samples");
    }
    let mut sorted = samples.to_vec();
    sorted.sort();
    let percentile = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction) as usize];
    println!(
        "{label}: n={} median={:?} p95={:?} p99={:?} max={:?}",
        sorted.len(),
        percentile(0.5),
        percentile(0.95),
        percentile(0.99),
        sorted[sorted.len() - 1]
    );
}
