//! Runs the real app on a showcase workspace and plays a scripted tour of it, for the README's
//! demo GIF. Nothing is faked: the tour sends the keystrokes and commands a person would.
//!
//! `demo DATA_DIR` builds the workspace in `DATA_DIR` (which must not exist yet), opens the
//! window at `WINDOW` (points, from the top left of the main display) in Scroll Dark, plays
//! `TOUR` and quits.
//!
//! To regenerate the GIF after UI changes, from the repository root:
//!
//! ```sh
//! scripts/record-demo.sh          # writes assets/demo/scroll-demo.gif
//! ```
//!
//! It builds this example in release, records the window's rectangle with `screencapture -v`
//! while the tour plays, and turns the recording into a GIF with ffmpeg and gifski
//! (`brew install ffmpeg gifski imagemagick`). Keep the mouse pointer away from that rectangle
//! while it records. To run the tour without recording:
//!
//! ```sh
//! CARGO_TARGET_DIR=target-ui cargo run --release -p scroll-app --example demo -- "$(mktemp -d)/data"
//! ```

mod seed;

use std::path::Path;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, Application, AsyncApp, Keystroke};
use scroll_app::app;
use scroll_settings::Settings;
use serde_json::{Value, json};

/// Where the window opens, in points: x, y, width, height. `scripts/record-demo.sh` records
/// this rectangle, so change both together.
const WINDOW: [f32; 4] = [284.0, 160.0, 1160.0, 740.0];
const SIDEBAR_WIDTH: f32 = 268.0;
const WORKSPACE_FILE: &str = "Fernway.scroll";
/// The pause after each keystroke step, so every change is seen.
const STEP_PAUSE: Duration = Duration::from_millis(300);
/// The pause between the keys of one step, such as `down down`.
const KEY_PAUSE: Duration = Duration::from_millis(130);
/// The pause between typed characters, about a quick typist's pace.
const TYPING_PAUSE: Duration = Duration::from_millis(100);

/// Keystrokes (`cmd-p`, `down down`), `type:<text>` (typed a character at a time),
/// `wait:<ms>`, `command:<id>`, `theme:<id>` (as picked on the settings page) and `quit`.
const TOUR: &[&str] = &[
    // The team's home page: blocks, to-dos, a callout, a quote, tags, the page tree.
    "wait:3000",
    // Find the architecture page from the palette: its Mermaid diagram and highlighted Rust.
    "cmd-p",
    "wait:400",
    "type:sync",
    "wait:700",
    "enter",
    "wait:3300",
    // The roadmap database, sorted by due date; walk to a status and change it. A select
    // picks an option only by its whole name: Return on part of one creates a new option.
    "cmd-p",
    "wait:400",
    "type:road",
    "wait:600",
    "enter",
    "wait:1300",
    "down down down down down down down",
    "right",
    "wait:300",
    "enter",
    "wait:600",
    "type:in progress",
    "wait:500",
    "enter",
    "wait:1500",
    // Themes, from the settings page; the table shows again in the last one.
    "cmd-,",
    "wait:1000",
    "theme:catppuccin-mocha",
    "wait:1300",
    "theme:dracula",
    "wait:1300",
    "theme:light",
    "wait:1200",
    "escape",
    "wait:2200",
    "quit",
];

fn main() {
    let Some(data) = std::env::args().nth(1) else {
        eprintln!("usage: demo DATA_DIR");
        return;
    };
    let data = Path::new(&data).to_path_buf();
    if data.exists() {
        eprintln!("{} exists; give a new directory", data.display());
        return;
    }
    if let Err(error) = prepare(&data) {
        eprintln!("could not build the showcase workspace: {error}");
        return;
    }
    Application::new()
        .with_assets(scroll_ui::Assets)
        .run(move |cx: &mut App| {
            scroll_app::logger::install();
            app::start(data.clone(), cx);
            cx.activate(true);
            cx.spawn(async move |cx| {
                let window = wait_for_window(cx).await;
                println!("demo: window open");
                for step in TOUR {
                    run_step(step, window, cx).await;
                }
            })
            .detach();
        });
}

/// The workspace, the list naming it, and the window's frame, theme and starting page.
fn prepare(data: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data)?;
    let showcase = seed::build(&data.join(WORKSPACE_FILE))?;
    let [x, y, width, height] = WINDOW;
    let files = [
        (
            "workspaces.json".to_string(),
            json!({
                "workspaces": [{"name": "Fernway", "file": WORKSPACE_FILE, "server": null}],
                "current": 0,
            }),
        ),
        (
            "settings.json".to_string(),
            json!({
                "window": {"x": x, "y": y, "width": width, "height": height},
                "sidebar_width": SIDEBAR_WIDTH,
            }),
        ),
        (
            "preferences.json".to_string(),
            json!({"appearance.theme": "dark"}),
        ),
        (
            WORKSPACE_FILE.replace(".scroll", ".settings.json"),
            json!({
                "last_page": showcase.home,
                "expanded_pages": showcase.expanded,
                "are_tags_shown": true,
            }),
        ),
    ];
    for (name, contents) in files {
        std::fs::write(data.join(name), serde_json::to_string_pretty(&contents)?)?;
    }
    Ok(())
}

async fn wait_for_window(cx: &mut AsyncApp) -> AnyWindowHandle {
    loop {
        if let Some(window) = cx.update(|cx| app::main_window(cx)).ok().flatten() {
            return window;
        }
        cx.background_executor()
            .timer(Duration::from_millis(10))
            .await;
    }
}

async fn run_step(step: &str, window: AnyWindowHandle, cx: &mut AsyncApp) {
    if let Some(milliseconds) = step.strip_prefix("wait:").and_then(|ms| ms.parse().ok()) {
        return pause(Duration::from_millis(milliseconds), cx).await;
    }
    if let Some(text) = step.strip_prefix("type:") {
        for character in text.chars() {
            dispatch(window, cx, |window, cx| {
                if let Some(keystroke) = typed(character) {
                    window.dispatch_keystroke(keystroke, cx);
                }
            });
            pause(TYPING_PAUSE, cx).await;
        }
        return;
    }
    if step == "quit" {
        println!("demo: tour finished at unix {:.3}", unix_now());
        let _ = cx.update(|cx| cx.quit());
        return;
    }
    if let Some(theme) = step.strip_prefix("theme:") {
        let _ = cx.update(|cx| set_theme(theme, cx));
        return;
    }
    if let Some(id) = step.strip_prefix("command:") {
        dispatch(window, cx, |window, cx| match app::command_action(id) {
            Some(action) => window.dispatch_action(action, cx),
            None => eprintln!("no command {id}"),
        });
        return pause(STEP_PAUSE, cx).await;
    }
    for key in step.split(' ') {
        match Keystroke::parse(key) {
            Ok(keystroke) => dispatch(window, cx, |window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            }),
            Err(error) => eprintln!("skipping unknown keystroke {key:?}: {error}"),
        }
        pause(KEY_PAUSE, cx).await;
    }
    pause(STEP_PAUSE, cx).await;
}

fn dispatch(
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
    send: impl FnOnce(&mut gpui::Window, &mut App),
) {
    if cx
        .update(|cx| window.update(cx, |_, window, cx| send(window, cx)))
        .is_err()
    {
        eprintln!("the window closed");
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

async fn pause(duration: Duration, cx: &mut AsyncApp) {
    cx.background_executor().timer(duration).await;
}

/// The keystroke that types `character`.
fn typed(character: char) -> Option<Keystroke> {
    let key = if character == ' ' {
        "space".to_string()
    } else {
        character.to_string()
    };
    let mut keystroke = Keystroke::parse(&key).ok()?;
    keystroke.key_char = Some(character.to_string());
    Some(keystroke)
}

/// Picks a theme as a click on its row of the settings page does.
fn set_theme(id: &str, cx: &mut App) {
    let theme = Settings::sections(cx)
        .into_iter()
        .flat_map(|(_, settings)| settings)
        .find(|setting| setting.key() == "appearance.theme");
    match theme {
        Some(theme) => theme.set_json(&Value::String(id.to_string()), cx),
        None => eprintln!("no theme setting"),
    }
}
