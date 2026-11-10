//! Two editors of one page, each its own Loro replica, exchanging updates and carets through
//! an in-process relay with network-like latency: what one person types reaches the other,
//! concurrent edits converge, and carets stay on their text.

mod relay;

use std::time::Duration;

use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, TestAppContext, VisualTestContext, Window, div,
};
use scroll_ui::TextSelection;

use crate::editor::document::{BlockId, PageDocument, PageResult};
use crate::editor::rich_text::MarkKind;
use crate::editor::{self, PageEditor, Selection};
use relay::{RelayLink, introduce, relay};

const SIMULATED_LATENCY: Duration = Duration::from_millis(120);

const PAGE_PARAGRAPHS: &[&str] = &[
    "This page is open twice. Each pane is its own Loro replica; edits travel between them as Loro updates through a relay that adds a little latency.",
    "Type in either pane and watch the other. Try typing in the same paragraph from both sides at once: both edits survive and the panes converge.",
    "Formatting: select text and press ⌘B for bold, ⌘I for italic, ⌘E for code, ⇧⌘X for strikethrough and ⌘K for a link. ⌘Z undoes only your own edits.",
    "Enter splits a paragraph, Backspace at the start of one joins it to the previous, and the arrow keys move between them.",
];

struct Replica {
    name: SharedString,
    page: PageDocument,
    selection: Selection,
}

/// Peer that writes the starting content, so neither pane's undo history contains it.
const SEED_PEER: u64 = 100;

/// Writes the demo page once, then opens two replicas of it from the same snapshot.
fn create_replicas() -> PageResult<[Replica; 2]> {
    let mut seed = PageDocument::create(SEED_PEER, PAGE_PARAGRAPHS)?;
    format_shortcut_examples(&mut seed)?;
    let snapshot = seed.export_snapshot()?;
    let mut left = PageDocument::from_snapshot(1, &snapshot)?;
    let mut right = PageDocument::from_snapshot(2, &snapshot)?;
    // Each replica starts somewhere different so both carets are visible: one with a
    // phrase selected, the other at the end of a paragraph.
    Ok([
        Replica {
            name: "Ada".into(),
            selection: phrase_selection(&mut left, 0, "edits travel between them")?,
            page: left,
        },
        Replica {
            name: "Grace".into(),
            selection: paragraph_end(&mut right, 1)?,
            page: right,
        },
    ])
}

fn phrase_selection(
    page: &mut PageDocument,
    block_index: usize,
    phrase: &str,
) -> PageResult<Selection> {
    let block = block_at(page, block_index)?;
    let text = page.block_content(block)?.text;
    let range = text
        .find(phrase)
        .map_or(0..0, |start| start..start + phrase.len());
    Ok(Selection {
        block,
        range: TextSelection::from_range(range, false),
    })
}

fn paragraph_end(page: &mut PageDocument, block_index: usize) -> PageResult<Selection> {
    let block = block_at(page, block_index)?;
    Ok(Selection::caret(
        block,
        page.block_content(block)?.text.len(),
    ))
}

/// The block at `index`, or the first block if the page is shorter than expected.
fn block_at(page: &mut PageDocument, index: usize) -> PageResult<BlockId> {
    match page.block_ids().get(index) {
        Some(block) => Ok(*block),
        None => page.ensure_block(),
    }
}

/// Shows each mark on the paragraph that lists the formatting shortcuts.
fn format_shortcut_examples(page: &mut PageDocument) -> PageResult<()> {
    let Some(&block) = page.block_ids().get(2) else {
        return Ok(());
    };
    let text = page.block_content(block)?.text;
    let range_of = |phrase: &str| text.find(phrase).map(|start| start..start + phrase.len());
    for (phrase, kind) in [
        ("⌘B for bold", MarkKind::Bold),
        ("⌘I for italic", MarkKind::Italic),
        ("⌘E for code", MarkKind::Code),
        ("⇧⌘X for strikethrough", MarkKind::Strikethrough),
    ] {
        if let Some(range) = range_of(phrase) {
            page.add_mark(block, range, kind)?;
        }
    }
    if let Some(range) = range_of("⌘K for a link") {
        page.add_link(block, range, "https://loro.dev")?;
    }
    page.commit();
    Ok(())
}

/// The two editors side by side, each sending its edits to the other.
struct ReplicasWindow {
    editors: [Entity<PageEditor>; 2],
    _subscriptions: Vec<Subscription>,
}

impl ReplicasWindow {
    fn new(replicas: [Replica; 2], window: &mut Window, cx: &mut Context<Self>) -> Self {
        let [left, right] = replicas;
        let left_editor = cx.new(|cx| PageEditor::new(left.page, left.selection, cx));
        let right_editor = cx.new(|cx| PageEditor::new(right.page, right.selection, cx));
        let links = [
            (
                left_editor.clone(),
                RelayLink {
                    to: right_editor.clone(),
                    sender_name: left.name.clone(),
                    latency: SIMULATED_LATENCY,
                },
            ),
            (
                right_editor.clone(),
                RelayLink {
                    to: left_editor.clone(),
                    sender_name: right.name.clone(),
                    latency: SIMULATED_LATENCY,
                },
            ),
        ];
        let mut subscriptions = Vec::new();
        for (from, link) in links {
            introduce(&from, &link, cx);
            subscriptions.push(relay(&from, link, cx));
        }
        window.focus(left_editor.read(cx).focus_handle());
        ReplicasWindow {
            editors: [left_editor, right_editor],
            _subscriptions: subscriptions,
        }
    }
}

impl Render for ReplicasWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().size_full().children(
            self.editors
                .iter()
                .map(|editor| div().flex_1().child(editor.clone())),
        )
    }
}

fn open_replicas(cx: &mut TestAppContext) -> (Entity<ReplicasWindow>, &mut VisualTestContext) {
    cx.update(|cx| {
        scroll_ui::init(cx);
        editor::init(cx);
    });
    let replicas = create_replicas().unwrap();
    cx.add_window_view(|window, cx| ReplicasWindow::new(replicas, window, cx))
}

fn editors(
    replicas: &Entity<ReplicasWindow>,
    cx: &mut VisualTestContext,
) -> [Entity<PageEditor>; 2] {
    replicas.read_with(cx, |replicas, _| replicas.editors.clone())
}

fn texts(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    editor.read_with(cx, |editor, _| {
        let page = editor.page();
        page.block_ids()
            .into_iter()
            .map(|block| page.block_content(block).unwrap().text)
            .collect()
    })
}

fn let_relay_deliver(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(SIMULATED_LATENCY * 2);
    cx.run_until_parked();
}

#[gpui::test]
fn typing_in_one_pane_appears_in_the_other(cx: &mut TestAppContext) {
    let (replicas, cx) = open_replicas(cx);
    let [left, right] = editors(&replicas, cx);
    cx.simulate_keystrokes("cmd-up");
    cx.simulate_input("Hi ");
    assert!(texts(&left, cx)[0].starts_with("Hi This page"));
    assert!(
        texts(&right, cx)[0].starts_with("This page"),
        "not delivered before the latency"
    );
    let_relay_deliver(cx);
    assert_eq!(texts(&left, cx), texts(&right, cx));
}

#[gpui::test]
fn concurrent_edits_converge_and_remote_carets_resolve(cx: &mut TestAppContext) {
    let (replicas, cx) = open_replicas(cx);
    let [left, right] = editors(&replicas, cx);
    cx.simulate_keystrokes("cmd-up");
    cx.simulate_input("A");
    cx.update(|window, cx| window.focus(right.read(cx).focus_handle()));
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input("Z");
    let_relay_deliver(cx);

    let left_texts = texts(&left, cx);
    assert_eq!(left_texts, texts(&right, cx));
    assert!(left_texts[0].starts_with('A'));
    assert!(left_texts.last().unwrap().ends_with('Z'));

    let right_caret = right.read_with(cx, |editor, _| editor.selection());
    let seen_by_left = left.read_with(cx, |editor, _| editor.collaborator_position());
    assert_eq!(
        seen_by_left,
        Some((right_caret.block, right_caret.range.head))
    );
}

#[gpui::test]
fn a_remote_insert_before_the_caret_does_not_move_it_off_its_text(cx: &mut TestAppContext) {
    let (replicas, cx) = open_replicas(cx);
    let [left, right] = editors(&replicas, cx);
    cx.update(|window, cx| window.focus(right.read(cx).focus_handle()));
    cx.simulate_keystrokes("cmd-up alt-right alt-right");
    let before = right.read_with(cx, |editor, _| editor.selection());
    assert_eq!(before.range.head, "This page".len());

    cx.update(|window, cx| window.focus(left.read(cx).focus_handle()));
    cx.simulate_keystrokes("cmd-up");
    cx.simulate_input(">> ");
    let_relay_deliver(cx);

    let after = right.read_with(cx, |editor, _| editor.selection());
    assert_eq!(after.block, before.block);
    assert_eq!(after.range.head, before.range.head + ">> ".len());
}

#[gpui::test]
fn each_pane_shows_the_other_caret_from_the_start(cx: &mut TestAppContext) {
    let (replicas, cx) = open_replicas(cx);
    let [left, right] = editors(&replicas, cx);
    for (viewer, other) in [(&left, &right), (&right, &left)] {
        let other_selection = other.read_with(cx, |editor, _| editor.selection());
        let seen = viewer.read_with(cx, |editor, _| editor.collaborator_position());
        assert_eq!(
            seen,
            Some((other_selection.block, other_selection.range.head))
        );
    }
}
