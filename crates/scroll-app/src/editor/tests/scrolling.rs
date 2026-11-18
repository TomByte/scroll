//! Where the page is scrolled to after edits, caret moves, and the wheel. The test platform does
//! not shape text, so every line of a block is one line high, which is all these need.

use std::time::Duration;

use gpui::{
    Entity, ListOffset, Modifiers, MouseButton, MouseDownEvent, Pixels, ScrollDelta,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
};
use scroll_ui::ScrollSource;

use super::open_editor;
use crate::editor::PageEditor;
use crate::editor::outline::SHOW_OUTLINE;
use crate::editor::scrolling::SMOOTH_SCROLLING;

/// Longer than a glide takes to settle.
const GLIDE_DONE: Duration = Duration::from_millis(400);

/// A page of `count` one-line paragraphs, drawn once.
fn open_long_page(
    count: usize,
    cx: &mut TestAppContext,
) -> (Entity<PageEditor>, &mut VisualTestContext) {
    let paragraphs: Vec<String> = (0..count).map(|index| format!("Block {index}")).collect();
    let paragraphs: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
    let (editor, cx) = open_editor(&paragraphs, cx);
    cx.run_until_parked();
    (editor, cx)
}

/// Draws the frames a caret reveal may take.
fn settle(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) {
    for _ in 0..4 {
        editor.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
}

fn top(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> (usize, Pixels) {
    let ListOffset {
        item_ix,
        offset_in_item,
    } = editor.read_with(cx, |editor, _| editor.scroll.top());
    (item_ix, offset_in_item)
}

fn caret_index(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |editor, _| {
        editor.block_index(editor.selection.block).unwrap()
    })
}

fn is_caret_on_screen(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |editor, _| {
        let caret_block = &editor.blocks[editor.block_index(editor.selection.block).unwrap()];
        let (Some(viewport), Some(layout)) = (editor.viewport, caret_block.layout.as_ref()) else {
            return false;
        };
        caret_block.layout_frame == editor.frame
            && layout.bounds().top() >= viewport.top()
            && layout.bounds().bottom() <= viewport.bottom()
    })
}

/// Whether the caret's line, in a block that may be taller than the screen, is on it.
fn is_caret_line_on_screen(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |editor, _| {
        let caret_block = &editor.blocks[editor.block_index(editor.selection.block).unwrap()];
        let (Some(viewport), Some(layout)) = (editor.viewport, caret_block.layout.as_ref()) else {
            return false;
        };
        let caret = layout.caret_origin(editor.selection.range.head).y;
        viewport.top() <= caret && caret + layout.line_height() <= viewport.bottom()
    })
}

/// How many rows fit on the screen.
fn rows_per_screen(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |editor, _| editor.scroll.visible_rows().len())
}

#[gpui::test]
fn a_new_block_in_the_middle_of_a_long_page_leaves_the_page_where_it_is(cx: &mut TestAppContext) {
    let (editor, cx) = open_long_page(400, cx);
    // Scroll down, then put the caret in a block in the middle of the screen.
    editor.update(cx, |editor, cx| editor.scroll_by(px(900.0), cx));
    cx.run_until_parked();
    let middle = editor.read_with(cx, |editor, _| {
        let visible = editor.scroll.visible_rows();
        (visible.start + visible.end) / 2
    });
    editor.update(cx, |editor, cx| {
        let block = editor.blocks[middle].info.id;
        editor.move_caret(block, 0, cx);
    });
    settle(&editor, cx);
    let before = top(&editor, cx);
    assert!(before.0 > 0, "the page is scrolled");

    cx.simulate_keystrokes("cmd-right enter");
    settle(&editor, cx);

    assert_eq!(
        caret_index(&editor, cx),
        middle + 1,
        "the caret is in the new block"
    );
    assert_eq!(top(&editor, cx), before, "the page did not move");
    assert!(is_caret_on_screen(&editor, cx));
}

#[gpui::test]
fn a_new_block_at_the_bottom_of_the_screen_scrolls_only_as_far_as_it_needs(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_long_page(400, cx);
    let screen = rows_per_screen(&editor, cx);
    let last_on_screen = editor.read_with(cx, |editor, _| editor.scroll.visible_rows().end - 1);
    editor.update(cx, |editor, cx| {
        let block = editor.blocks[last_on_screen].info.id;
        editor.move_caret(block, 0, cx);
    });
    settle(&editor, cx);
    let before = top(&editor, cx).0;

    cx.simulate_keystrokes("cmd-right enter");
    settle(&editor, cx);

    assert!(is_caret_on_screen(&editor, cx));
    let moved = top(&editor, cx).0 - before;
    assert!(
        moved < screen / 2,
        "scrolled {moved} rows of a {screen}-row screen"
    );
}

#[gpui::test]
fn arrowing_down_past_the_bottom_scrolls_a_line_not_a_screen(cx: &mut TestAppContext) {
    let (editor, cx) = open_long_page(400, cx);
    let screen = rows_per_screen(&editor, cx);
    for _ in 0..screen + 2 {
        cx.simulate_keystrokes("down");
        settle(&editor, cx);
    }
    assert_eq!(caret_index(&editor, cx), screen + 2);
    assert!(is_caret_on_screen(&editor, cx));
    let scrolled = top(&editor, cx).0;
    assert!(
        (1..screen / 2).contains(&scrolled),
        "scrolled {scrolled} rows of a {screen}-row screen"
    );
}

#[gpui::test]
fn the_caret_is_found_far_from_the_screen_in_both_directions(cx: &mut TestAppContext) {
    let (editor, cx) = open_long_page(400, cx);
    cx.simulate_keystrokes("cmd-down");
    settle(&editor, cx);
    assert_eq!(caret_index(&editor, cx), 399);
    assert!(is_caret_on_screen(&editor, cx), "at the end");

    cx.simulate_keystrokes("cmd-up");
    settle(&editor, cx);
    assert_eq!(caret_index(&editor, cx), 0);
    assert!(is_caret_on_screen(&editor, cx), "back at the start");
    assert_eq!(top(&editor, cx), (0, px(0.0)));
}

#[gpui::test]
fn typing_in_the_block_at_the_top_of_the_screen_does_not_jump_to_its_start(
    cx: &mut TestAppContext,
) {
    // One block far taller than the screen, then more page below it.
    let tall: String = (0..200).map(|line| format!("line {line}\n")).collect();
    let (editor, cx) = open_editor(&[&tall, "after"], cx);
    cx.run_until_parked();
    let line_120 = tall.match_indices('\n').nth(119).unwrap().0;
    editor.update(cx, |editor, cx| {
        let block = editor.blocks[0].info.id;
        editor.move_caret(block, line_120, cx);
    });
    settle(&editor, cx);
    let before = top(&editor, cx);
    assert!(
        before.0 == 0 && before.1 > px(500.0),
        "scrolled well into the tall block: {before:?}"
    );

    cx.simulate_input("x");
    settle(&editor, cx);
    assert_eq!(top(&editor, cx), before, "typing on the caret's line");

    // A new line at the bottom edge scrolls by that line.
    cx.simulate_keystrokes("shift-enter");
    settle(&editor, cx);
    let after = top(&editor, cx);
    assert_eq!(after.0, 0);
    let moved = after.1 - before.1;
    assert!(px(0.0) < moved && moved < px(60.0), "moved {moved:?}");
    assert!(is_caret_line_on_screen(&editor, cx));
}

#[gpui::test]
fn the_scrollbar_counts_the_rows_not_drawn_yet(cx: &mut TestAppContext) {
    let (editor, cx) = open_long_page(1000, cx);
    settle(&editor, cx);
    let (position, screen) = editor.read_with(cx, |editor, cx| {
        (
            editor.scroll.position(cx),
            editor.scroll.visible_rows().len(),
        )
    });
    let row_height = position.viewport / screen as f32;
    let whole_page = row_height * 1000.0;
    assert_eq!(position.offset, px(0.0));
    assert!(
        position.max_offset > whole_page * 0.8 && position.max_offset < whole_page * 1.2,
        "{position:?} for a page about {whole_page:?} high"
    );

    // Dragging the thumb to the end goes to the end of the page, not of what was drawn.
    editor.update(cx, |editor, cx| {
        editor.scroll.scroll_to(position.max_offset, cx);
        cx.notify();
    });
    settle(&editor, cx);
    let last_row_on_screen = editor.read_with(cx, |editor, _| editor.scroll.visible_rows().end);
    assert_eq!(last_row_on_screen, 1000);
    // The last row turned out taller than the average (it carries the space at the page's
    // end), so the thumb is within that space of the end.
    let at_end = editor.read_with(cx, |editor, cx| editor.scroll.position(cx));
    assert!(at_end.max_offset - at_end.offset < px(200.0), "{at_end:?}");
}

fn turn_wheel(lines: f32, cx: &mut VisualTestContext) {
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(300.0), px(300.0)),
        delta: ScrollDelta::Lines(point(0.0, lines)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
}

fn scrolled(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |editor, cx| editor.scroll.position(cx).offset)
}

#[gpui::test]
fn a_mouse_wheel_jumps_by_default_and_glides_with_smooth_scrolling(cx: &mut TestAppContext) {
    let (editor, cx) = open_long_page(400, cx);
    settle(&editor, cx);
    turn_wheel(-3.0, cx);
    let step = scrolled(&editor, cx);
    assert_eq!(step, px(60.0), "three lines at once");

    cx.update(|_, cx| SMOOTH_SCROLLING.set(true, cx));
    turn_wheel(-3.0, cx);
    let just_after = scrolled(&editor, cx);
    assert!(
        step < just_after && just_after < step * 2.0,
        "under way but not there: {just_after:?}"
    );
    for _ in 0..3 {
        std::thread::sleep(GLIDE_DONE);
        settle(&editor, cx);
    }
    assert_eq!(scrolled(&editor, cx), step * 2.0, "it arrives exactly");
    assert!(!editor.read_with(cx, |editor, _| editor.glide.is_moving()));
}

/// A page with three headed sections of `section` paragraphs each.
fn open_headed_page(
    section: usize,
    cx: &mut TestAppContext,
) -> (Entity<PageEditor>, &mut VisualTestContext) {
    open_sections(
        &[("Alpha", section), ("Beta", section), ("Gamma", section)],
        cx,
    )
}

/// A page of headed sections, each a heading and that many paragraphs, with the outline shown.
fn open_sections<'a>(
    sections: &[(&str, usize)],
    cx: &'a mut TestAppContext,
) -> (Entity<PageEditor>, &'a mut VisualTestContext) {
    let mut lines = Vec::new();
    for (heading, paragraphs) in sections {
        lines.push(format!("# {heading}"));
        lines.extend((0..*paragraphs).map(|index| format!("{heading} text {index}")));
    }
    let (editor, cx) = open_editor(&[""], cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(lines.join("\n")));
    cx.simulate_keystrokes("cmd-v cmd-up");
    cx.update(|_, cx| SHOW_OUTLINE.set(true, cx));
    settle(&editor, cx);
    (editor, cx)
}

/// The outline's headings, with whether each is marked as the current one.
fn outline(editor: &Entity<PageEditor>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    editor.read_with(cx, |editor, _| {
        editor
            .outline_headings()
            .into_iter()
            .map(|(title, is_current)| (title.to_string(), is_current))
            .collect()
    })
}

#[gpui::test]
fn the_outline_lists_headings_and_marks_the_one_the_reader_is_on(cx: &mut TestAppContext) {
    let (editor, cx) = open_headed_page(100, cx);
    let page_width = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.viewport.unwrap().size.width)
    };
    assert!(cx.debug_bounds("page-outline").is_some());
    assert_eq!(
        outline(&editor, cx),
        [
            ("Alpha".to_string(), true),
            ("Beta".to_string(), false),
            ("Gamma".to_string(), false)
        ]
    );
    let beside_outline = page_width(cx);

    cx.update(|_, cx| SHOW_OUTLINE.set(false, cx));
    settle(&editor, cx);
    assert!(
        page_width(cx) > beside_outline,
        "the page takes the outline's room back"
    );
}

#[gpui::test]
fn clicking_a_heading_in_the_outline_goes_to_it(cx: &mut TestAppContext) {
    let (editor, cx) = open_headed_page(100, cx);
    let gamma = cx.debug_bounds("outline-entry-2").expect("a third entry");
    cx.simulate_click(gamma.center(), Modifiers::none());
    settle(&editor, cx);

    let gamma_index = 2 * 101;
    assert_eq!(caret_index(&editor, cx), gamma_index);
    let heading_top = editor.read_with(cx, |editor, _| {
        let layout = editor.blocks[gamma_index].layout.as_ref().unwrap();
        layout.bounds().top() - editor.viewport.unwrap().top()
    });
    assert!(
        px(0.0) < heading_top && heading_top < px(120.0),
        "near the top of the page area: {heading_top:?}"
    );
    assert_eq!(
        outline(&editor, cx),
        [
            ("Alpha".to_string(), false),
            ("Beta".to_string(), false),
            ("Gamma".to_string(), true)
        ],
        "only Gamma is marked, though the end of Beta still shows above it"
    );
    let arrived_at = top(&editor, cx);

    // Typing carries on in the heading, and the page stays put.
    cx.simulate_input("The ");
    settle(&editor, cx);
    assert_eq!(outline(&editor, cx)[2].0, "The Gamma");
    assert_eq!(top(&editor, cx), arrived_at);
}

#[gpui::test]
fn a_heading_already_on_screen_is_brought_to_the_top_too(cx: &mut TestAppContext) {
    // Two sections that fill the first screen, so the third heading starts at its foot (where
    // its old position, were it still trusted after the jump, would scroll the page on).
    let (editor, cx) = open_sections(&[("Alpha", 17), ("Beta", 17), ("Gamma", 100)], cx);
    let heading_top = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            let layout = editor.blocks[2 * 18].layout.as_ref().unwrap();
            layout.bounds().top() - editor.viewport.unwrap().top()
        })
    };
    let screen = editor.read_with(cx, |editor, _| editor.viewport.unwrap().size.height);
    let before = heading_top(cx);
    assert!(
        before > screen - px(100.0) && before < screen - px(40.0),
        "Gamma starts at the foot of the screen: {before:?} of {screen:?}"
    );

    let gamma = cx.debug_bounds("outline-entry-2").expect("a third entry");
    cx.simulate_click(gamma.center(), Modifiers::none());
    settle(&editor, cx);

    assert_eq!(caret_index(&editor, cx), 2 * 18);
    let after = heading_top(cx);
    assert!(
        px(0.0) < after && after < px(120.0),
        "near the top of the page area, and all of it showing: {after:?}"
    );
}

#[gpui::test]
fn the_section_the_screen_is_in_stays_marked_once_its_heading_scrolls_off(cx: &mut TestAppContext) {
    let (editor, cx) = open_headed_page(100, cx);
    // Well into Beta's text, with neither Beta's heading nor Gamma's on screen.
    editor.update(cx, |editor, cx| {
        editor.scroll.scroll_row_to_top(101 + 30);
        cx.notify();
    });
    settle(&editor, cx);
    assert_eq!(
        outline(&editor, cx),
        [
            ("Alpha".to_string(), false),
            ("Beta".to_string(), true),
            ("Gamma".to_string(), false)
        ]
    );
}

#[gpui::test]
fn the_caret_s_section_is_the_current_one_while_the_caret_is_on_screen(cx: &mut TestAppContext) {
    // Three headings on the first screen, with the caret in the first.
    let (editor, cx) = open_sections(&[("Alpha", 3), ("Beta", 3), ("Gamma", 100)], cx);
    let current = |cx: &mut VisualTestContext| {
        let headings = outline(&editor, cx).into_iter();
        let current: Vec<String> = headings
            .filter(|(_, is_current)| *is_current)
            .map(|(title, _)| title)
            .collect();
        current
    };
    assert_eq!(current(cx), ["Alpha"]);

    // Into Beta's text: the page has not scrolled, and Beta is where the reader is.
    cx.simulate_keystrokes("down down down down down");
    settle(&editor, cx);
    assert_eq!(caret_index(&editor, cx), 5);
    assert_eq!(top(&editor, cx), (0, px(0.0)));
    assert_eq!(current(cx), ["Beta"]);

    // Selecting whole blocks down into Gamma's section moves the reader there too.
    cx.simulate_keystrokes("escape down down down down");
    settle(&editor, cx);
    assert_eq!(current(cx), ["Gamma"]);
}

#[gpui::test]
fn dragging_the_outline_s_edge_resizes_it_between_its_limits(cx: &mut TestAppContext) {
    let (editor, cx) = open_headed_page(100, cx);
    let outline_width = |cx: &mut VisualTestContext| {
        settle(&editor, cx);
        cx.debug_bounds("page-outline").unwrap().size.width
    };
    let page_width = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.viewport.unwrap().size.width)
    };
    let (left, none) = (MouseButton::Left, Modifiers::none());
    // Presses the outline's left edge, drags it `by` to the right and lets go.
    let drag_edge = |by: Pixels, cx: &mut VisualTestContext| {
        let outline = cx.debug_bounds("page-outline").unwrap();
        let edge = point(outline.left() + px(1.0), outline.center().y);
        cx.simulate_mouse_down(edge, left, none);
        cx.simulate_mouse_move(point(edge.x + by, edge.y), Some(left), none);
        cx.simulate_mouse_up(point(edge.x + by, edge.y), left, none);
    };
    let (default_width, page_beside_it) = (outline_width(cx), page_width(cx));

    drag_edge(px(-60.0), cx);
    assert_eq!(outline_width(cx), default_width + px(60.0));
    assert_eq!(
        page_width(cx),
        page_beside_it - px(60.0),
        "the page gives up the room"
    );
    assert_eq!(
        cx.update(|_, cx| crate::editor::outline::dragged_width(cx)),
        Some(default_width + px(60.0)),
        "for the window to keep"
    );

    // The pointer moving with no button held, after the drag has ended, changes nothing.
    cx.simulate_mouse_move(point(px(300.0), px(300.0)), None, none);
    assert_eq!(outline_width(cx), default_width + px(60.0));

    drag_edge(px(-2000.0), cx);
    let widest = outline_width(cx);
    assert!(
        widest > default_width + px(60.0) && widest < px(500.0),
        "{widest:?}"
    );
    drag_edge(px(2000.0), cx);
    let narrowest = outline_width(cx);
    assert!(
        narrowest > px(100.0) && narrowest < default_width,
        "{narrowest:?}"
    );

    // A double click on the edge puts it back.
    let outline = cx.debug_bounds("page-outline").unwrap();
    cx.simulate_event(MouseDownEvent {
        position: point(outline.left() + px(1.0), outline.center().y),
        button: left,
        modifiers: none,
        click_count: 2,
        first_mouse: false,
    });
    assert_eq!(outline_width(cx), default_width);
}
