//! Drives a real `PageEditor` through GPUI's headless test platform: keystrokes go through
//! the key bindings, actions and the platform input handler, edits through Loro. The test
//! platform does not shape text, so caret geometry is not covered here.

use gpui::{Entity, TestAppContext, VisualTestContext};

use super::rich_text::MarkKind;
use super::{PageEditor, Selection, init};
use crate::editor::document::{BlockId, PageDocument, Placement};

fn open_editor<'a>(
    paragraphs: &[&str],
    cx: &'a mut TestAppContext,
) -> (Entity<PageEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        scroll_ui::init(cx);
        init(cx);
        crate::commands::bind_keys(crate::window::command_list::all(), cx);
    });
    let page = PageDocument::create(1, paragraphs).unwrap();
    let first_block = page.ensure_block().unwrap();
    cx.add_window_view(|window, cx| {
        let editor = PageEditor::new(page, Selection::caret(first_block, 0), cx);
        window.focus(&editor.focus_handle);
        editor
    })
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

fn block(editor: &Entity<PageEditor>, index: usize, cx: &mut VisualTestContext) -> BlockId {
    editor.read_with(cx, |editor, _| editor.page().block_ids()[index])
}

#[gpui::test]
fn typing_is_written_to_the_loro_document(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["hello"], cx);
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" world");
    assert_eq!(texts(&editor, cx), ["hello world"]);
}

#[gpui::test]
fn enter_splits_and_backspace_at_start_merges(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["onetwo"], cx);
    cx.simulate_keystrokes("right right right enter");
    assert_eq!(texts(&editor, cx), ["one", "two"]);
    let second = block(&editor, 1, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection()).block,
        second
    );

    cx.simulate_keystrokes("backspace");
    assert_eq!(texts(&editor, cx), ["onetwo"]);
    let caret = editor.read_with(cx, |editor, _| editor.selection().range.head);
    assert_eq!(caret, 3);
}

#[gpui::test]
fn arrows_cross_block_boundaries(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["ab", "cd"], cx);
    cx.simulate_keystrokes("right right right");
    let second = block(&editor, 1, cx);
    let selection = editor.read_with(cx, |editor, _| editor.selection());
    assert_eq!((selection.block, selection.range.head), (second, 0));
    cx.simulate_keystrokes("left");
    let first = block(&editor, 0, cx);
    let selection = editor.read_with(cx, |editor, _| editor.selection());
    assert_eq!((selection.block, selection.range.head), (first, 2));
}

#[gpui::test]
fn shortcuts_toggle_marks_on_the_selection(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["bold me"], cx);
    let first = block(&editor, 0, cx);
    cx.simulate_keystrokes("cmd-a cmd-b");
    let is_bold = editor.read_with(cx, |editor, _| {
        editor
            .page()
            .block_content(first)
            .unwrap()
            .is_marked_throughout(&(0..7), MarkKind::Bold)
    });
    assert!(is_bold);
    cx.simulate_keystrokes("cmd-b");
    let is_bold = editor.read_with(cx, |editor, _| {
        editor
            .page()
            .block_content(first)
            .unwrap()
            .is_marked_throughout(&(0..7), MarkKind::Bold)
    });
    assert!(!is_bold);
}

#[gpui::test]
fn a_mark_toggled_at_the_caret_applies_to_the_next_typing(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["plain"], cx);
    let first = block(&editor, 0, cx);
    cx.simulate_keystrokes("cmd-down cmd-i");
    cx.simulate_input("!");
    let content = editor.read_with(cx, |editor, _| editor.page().block_content(first).unwrap());
    assert_eq!(content.text, "plain!");
    assert!(content.is_marked_throughout(&(5..6), MarkKind::Italic));
    assert!(!content.is_marked_throughout(&(0..5), MarkKind::Italic));
}

#[gpui::test]
fn undo_and_redo_go_through_loro(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["base"], cx);
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input("!");
    cx.simulate_keystrokes("enter");
    assert_eq!(texts(&editor, cx), ["base!", ""]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(texts(&editor, cx), ["base"]);
    let caret = editor.read_with(cx, |editor, _| editor.selection().range.head);
    assert_eq!(caret, 4, "undo puts the caret back where the edit started");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(texts(&editor, cx), ["base!", ""]);
}

#[gpui::test]
fn ime_composition_replaces_marked_text(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            use gpui::EntityInputHandler;
            editor.replace_and_mark_text_in_range(None, "ni", None, window, cx);
            editor.replace_and_mark_text_in_range(None, "nihao", None, window, cx);
            assert_eq!(editor.marked_text_range(window, cx), Some(0..5));
            editor.replace_text_in_range(None, "你好", window, cx);
            assert_eq!(editor.marked_text_range(window, cx), None);
        });
    });
    assert_eq!(texts(&editor, cx), ["你好"]);
    let caret = editor.read_with(cx, |editor, _| editor.selection().range.head);
    assert_eq!(caret, "你好".len());
}

#[gpui::test]
fn cmd_k_asks_for_a_url_and_links_the_selection(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["see docs"], cx);
    let first = block(&editor, 0, cx);
    cx.simulate_keystrokes("cmd-down alt-shift-left cmd-k");
    assert!(editor.read_with(cx, |editor, _| editor.link_prompt.is_some()));
    cx.simulate_input("https://loro.dev");
    cx.simulate_keystrokes("enter");
    let content = editor.read_with(cx, |editor, _| editor.page().block_content(first).unwrap());
    assert!(editor.read_with(cx, |editor, _| editor.link_prompt.is_none()));
    assert_eq!(content.link_at(5), Some("https://loro.dev"));
    assert_eq!(content.link_at(0), None);
    assert_eq!(
        content.text, "see docs",
        "typing in the prompt must not reach the page"
    );

    cx.simulate_keystrokes("cmd-k");
    let content = editor.read_with(cx, |editor, _| editor.page().block_content(first).unwrap());
    assert_eq!(content.link_at(5), None, "cmd-k on a link removes it");
}

fn kinds(
    editor: &Entity<PageEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(crate::editor::document::BlockKind, usize)> {
    editor.read_with(cx, |editor, _| {
        editor
            .page()
            .blocks()
            .iter()
            .map(|block| (block.kind, block.depth))
            .collect()
    })
}

use crate::editor::document::BlockKind;

#[gpui::test]
fn markdown_prefixes_change_the_block_kind(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("## Title");
    assert_eq!(texts(&editor, cx), ["Title"]);
    assert_eq!(kinds(&editor, cx), [(BlockKind::Heading2, 0)]);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("- item");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("[] task");
    assert_eq!(texts(&editor, cx), ["Title", "item", "task"]);
    assert_eq!(
        kinds(&editor, cx),
        [
            (BlockKind::Heading2, 0),
            (BlockKind::BulletedListItem, 0),
            (BlockKind::ToDo, 0)
        ]
    );
}

#[gpui::test]
fn inline_markdown_becomes_marks(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    let first = block(&editor, 0, cx);
    cx.simulate_input("a **bold** b");
    let content = editor.read_with(cx, |editor, _| editor.page().block_content(first).unwrap());
    assert_eq!(content.text, "a bold b");
    assert!(content.is_marked_throughout(&(2..6), MarkKind::Bold));
    assert!(
        !content.is_marked_throughout(&(6..8), MarkKind::Bold),
        "typing after the closing ** is not bold"
    );
}

#[gpui::test]
fn the_slash_menu_filters_and_converts(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("/quo");
    assert!(editor.read_with(cx, |editor, _| editor.completion.is_some()));
    cx.simulate_keystrokes("enter");
    assert_eq!(texts(&editor, cx), [""]);
    assert_eq!(kinds(&editor, cx), [(BlockKind::Quote, 0)]);

    cx.simulate_input("/zz");
    cx.simulate_keystrokes("escape");
    assert!(editor.read_with(cx, |editor, _| editor.completion.is_none()));
    assert_eq!(
        texts(&editor, cx),
        ["/zz"],
        "escape leaves the typed text alone"
    );
}

#[gpui::test]
fn enter_and_backspace_follow_block_kinds(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("1. one");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("two");
    assert_eq!(
        kinds(&editor, cx),
        [
            (BlockKind::NumberedListItem, 0),
            (BlockKind::NumberedListItem, 0)
        ]
    );
    cx.simulate_keystrokes("enter enter");
    assert_eq!(
        kinds(&editor, cx)[2],
        (BlockKind::Paragraph, 0),
        "enter on an empty item ends the list"
    );

    cx.simulate_keystrokes("up cmd-left backspace");
    assert_eq!(
        kinds(&editor, cx)[1],
        (BlockKind::Paragraph, 0),
        "backspace at the start drops the style first"
    );
    cx.simulate_keystrokes("backspace");
    assert_eq!(texts(&editor, cx), ["onetwo", ""]);
}

#[gpui::test]
fn tab_nests_and_shift_tab_unnests(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["parent", "child"], cx);
    cx.simulate_keystrokes("down tab");
    assert_eq!(
        kinds(&editor, cx),
        [(BlockKind::Paragraph, 0), (BlockKind::Paragraph, 1)]
    );
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(
        kinds(&editor, cx),
        [(BlockKind::Paragraph, 0), (BlockKind::Paragraph, 0)]
    );
}

#[gpui::test]
fn line_breaks_and_code_blocks_keep_text_in_one_block(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("a");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("b");
    assert_eq!(texts(&editor, cx), ["a\nb"]);

    cx.simulate_keystrokes("enter");
    cx.simulate_input("```");
    cx.simulate_input("x");
    cx.simulate_keystrokes("enter tab");
    cx.simulate_input("y");
    assert_eq!(texts(&editor, cx), ["a\nb", "x\n    y"]);
    assert_eq!(kinds(&editor, cx)[1].0, BlockKind::Code);
}

#[gpui::test]
fn cmd_enter_toggles_a_to_do(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("[] buy milk");
    cx.simulate_keystrokes("cmd-enter");
    let is_checked = editor.read_with(cx, |editor, _| editor.page().blocks()[0].is_checked);
    assert!(is_checked);
}

#[gpui::test]
fn escape_selects_blocks_that_can_be_moved_and_deleted(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["one", "two", "three"], cx);
    cx.simulate_keystrokes("escape shift-down");
    cx.simulate_keystrokes("cmd-shift-down");
    assert_eq!(texts(&editor, cx), ["three", "one", "two"]);
    cx.simulate_keystrokes("backspace");
    assert_eq!(texts(&editor, cx), ["three"]);
    assert!(editor.read_with(cx, |editor, _| editor.block_selection.is_none()));
}

#[gpui::test]
fn ime_composition_is_not_written_until_committed(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["x"], cx);
    cx.simulate_keystrokes("cmd-down");
    editor.update(cx, |editor, _| editor.take_outgoing_updates());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            use gpui::EntityInputHandler;
            editor.replace_and_mark_text_in_range(None, "n", None, window, cx);
            editor.replace_and_mark_text_in_range(None, "ni", None, window, cx);
            assert_eq!(
                editor
                    .text_for_range(0..3, &mut None, window, cx)
                    .as_deref(),
                Some("xni")
            );
        });
    });
    assert_eq!(
        texts(&editor, cx),
        ["x"],
        "the document never sees half-composed text"
    );
    assert!(
        editor
            .update(cx, |editor, _| editor.take_outgoing_updates())
            .is_empty()
    );

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            use gpui::EntityInputHandler;
            editor.replace_text_in_range(None, "你", window, cx);
        });
    });
    assert_eq!(texts(&editor, cx), ["x你"]);
    assert!(
        !editor
            .update(cx, |editor, _| editor.take_outgoing_updates())
            .is_empty()
    );
}

#[gpui::test]
fn dragging_moves_blocks_into_nesting(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["a", "b", "c"], cx);
    let [a, c] = [block(&editor, 0, cx), block(&editor, 2, cx)];
    editor.update(cx, |editor, cx| {
        editor.move_dragged(
            &[c],
            crate::editor::document::Placement::FirstChildOf(Some(a)),
            cx,
        );
    });
    assert_eq!(texts(&editor, cx), ["a", "c", "b"]);
    assert_eq!(kinds(&editor, cx)[1], (BlockKind::Paragraph, 1));
}

#[gpui::test]
fn a_five_thousand_block_page_opens_and_edits(cx: &mut TestAppContext) {
    let paragraphs: Vec<String> = (0..5000)
        .map(|index| format!("Paragraph number {index}"))
        .collect();
    let paragraphs: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
    let (editor, cx) = open_editor(&paragraphs, cx);
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input("!");
    let last = editor.read_with(cx, |editor, _| editor.page().block_ids()[4999]);
    let text = editor.read_with(cx, |editor, _| {
        editor.page().block_content(last).unwrap().text
    });
    assert_eq!(text, "Paragraph number 4999!");
}

#[gpui::test]
fn colon_names_complete_to_emoji(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.simulate_input("Nice :red_heart: and :thumbs");
    cx.simulate_keystrokes("enter");
    assert_eq!(texts(&editor, cx), vec!["Nice ❤️ and 👍".to_string()]);
}

#[gpui::test]
fn copied_blocks_paste_with_kinds_and_marks(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&[""], cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        "# Title\n- [x] done **bold**\n  - child".into(),
    ));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(
        texts(&editor, cx),
        vec!["Title".to_string(), "done bold".into(), "child".into()]
    );
    editor.read_with(cx, |editor, _| {
        let infos = editor.page().blocks();
        assert_eq!(infos[0].kind, crate::editor::document::BlockKind::Heading1);
        assert_eq!(infos[1].kind, crate::editor::document::BlockKind::ToDo);
        assert!(infos[1].is_checked);
        assert_eq!(infos[2].depth, 1);
    });
    // Copy all blocks and paste them again: the app's own format comes back unchanged.
    cx.simulate_keystrokes("escape cmd-a cmd-c");
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert_eq!(copied, "# Title\n- [x] done **bold**\n  - child");
}

#[gpui::test]
fn after_enter_the_drag_handle_follows_the_caret_until_the_pointer_moves(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["one"], cx);
    let pointer = gpui::point(gpui::px(100.0), gpui::px(100.0));
    let handle_at_caret = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.block_with_handle_at_caret())
    };
    cx.simulate_mouse_move(pointer, None, gpui::Modifiers::none());
    assert_eq!(handle_at_caret(cx), None, "the pointer decides");

    cx.simulate_keystrokes("cmd-down enter");
    assert_eq!(handle_at_caret(cx), Some(block(&editor, 1, cx)));

    cx.simulate_mouse_move(
        pointer + gpui::point(gpui::px(1.0), gpui::px(0.0)),
        None,
        gpui::Modifiers::none(),
    );
    assert_eq!(handle_at_caret(cx), None);
}

#[gpui::test]
fn dragging_from_one_block_into_another_selects_the_blocks_between(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["one", "two", "three"], cx);
    cx.run_until_parked();
    let centre = |index: usize, cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.blocks[index]
                .layout
                .as_ref()
                .unwrap()
                .bounds()
                .center()
        })
    };
    let selected = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.selected_block_range())
    };
    let (first, third) = (centre(0, cx), centre(2, cx));
    let left = gpui::MouseButton::Left;
    let none = gpui::Modifiers::none();

    cx.simulate_mouse_down(first, left, none);
    cx.simulate_mouse_move(third, Some(left), none);
    assert_eq!(selected(cx), Some(0..3));
    cx.simulate_mouse_move(first, Some(left), none);
    assert_eq!(
        selected(cx),
        None,
        "back in the first block it selects text again"
    );
    cx.simulate_mouse_move(third, Some(left), none);
    cx.simulate_mouse_up(third, left, none);
    assert_eq!(selected(cx), Some(0..3));

    cx.simulate_keystrokes("backspace");
    assert_eq!(texts(&editor, cx), [""]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        texts(&editor, cx),
        ["one", "two", "three"],
        "deleting the selection is one undo step"
    );
}

#[gpui::test]
fn shift_click_selects_the_blocks_up_to_the_one_clicked(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(&["one", "two", "three"], cx);
    cx.run_until_parked();
    let third = editor.read_with(cx, |editor, _| {
        editor.blocks[2].layout.as_ref().unwrap().bounds().center()
    });
    cx.simulate_click(third, gpui::Modifiers::shift());
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selected_block_range()),
        Some(0..3)
    );
}

#[gpui::test]
fn moving_a_selection_of_blocks_keeps_their_order_and_nesting_and_undoes_in_one_step(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_editor(&["one", "two", "three", "four"], cx);
    cx.simulate_keystrokes("right right right right right right right right tab");
    cx.simulate_input("!");
    // Loro folds edits made within its merge interval into one undo step.
    std::thread::sleep(std::time::Duration::from_millis(700));
    cx.simulate_keystrokes("left left escape shift-down");
    let last = block(&editor, 3, cx);

    editor.update(cx, |editor, cx| {
        let selected = editor.target_blocks();
        editor.move_dragged(&selected, Placement::After(last), cx)
    });

    assert_eq!(texts(&editor, cx), ["one", "four", "two", "!three"]);
    let depths: Vec<usize> = editor.read_with(cx, |editor, _| {
        editor.blocks.iter().map(|block| block.info.depth).collect()
    });
    assert_eq!(
        depths,
        [0, 0, 0, 1],
        "the nested block moved with its parent"
    );
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selected_block_range()),
        Some(2..4),
        "the moved blocks stay selected"
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(texts(&editor, cx), ["one", "two", "!three", "four"]);
}

#[gpui::test]
fn the_page_width_setting_reaches_an_open_editor(cx: &mut TestAppContext) {
    use crate::page_layout::{PAGE_WIDTH, PageWidth};

    let (editor, cx) = open_editor(&["wide"], cx);
    cx.update(|_, cx| PAGE_WIDTH.set(PageWidth::Full, cx));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_width),
        PageWidth::Full
    );
}

mod diagrams {
    use gpui::{Modifiers, TestAppContext, VisualTestContext, px, size};

    use super::{block, open_editor, texts};
    use crate::editor::document::{BlockKind, PageDocument};
    use crate::editor::mermaid::EXAMPLE_DIAGRAM;
    use crate::editor::{PageAccess, PageEditor, Selection};

    fn insert_diagram(
        cx: &mut TestAppContext,
    ) -> (gpui::Entity<PageEditor>, &mut VisualTestContext) {
        let (editor, cx) = open_editor(&[""], cx);
        cx.simulate_input("/mermaid");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        (editor, cx)
    }

    /// The id of the image the diagram shows, and its error, once drawing has finished.
    fn shown(
        editor: &gpui::Entity<PageEditor>,
        cx: &mut VisualTestContext,
    ) -> (Option<gpui::ImageId>, Option<String>) {
        cx.run_until_parked();
        let first = block(editor, 0, cx);
        editor.read_with(cx, |editor, _| {
            let view = editor.diagrams.view(first);
            assert!(!view.is_drawing, "drawing finished");
            (
                view.drawn.map(|drawn| drawn.image.id),
                view.error.map(|error| error.to_string()),
            )
        })
    }

    fn is_editing(editor: &gpui::Entity<PageEditor>, cx: &mut VisualTestContext) -> bool {
        let first = block(editor, 0, cx);
        editor.update_in(cx, |editor, window, _| {
            editor.is_editing_diagram(first, window)
        })
    }

    #[gpui::test]
    fn the_slash_menu_inserts_an_example_diagram_to_edit(cx: &mut TestAppContext) {
        let (editor, cx) = insert_diagram(cx);
        let first = block(&editor, 0, cx);
        assert_eq!(texts(&editor, cx), [EXAMPLE_DIAGRAM]);
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.kind_of(first), BlockKind::Code);
            assert!(editor.is_diagram(first));
            assert_eq!(
                editor.selection(),
                Selection::caret(first, EXAMPLE_DIAGRAM.len())
            );
        });
        assert!(
            is_editing(&editor, cx),
            "the source shows with the caret in it"
        );
        let (image, error) = shown(&editor, cx);
        assert!(image.is_some() && error.is_none(), "{error:?}");
        assert!(
            cx.debug_bounds("diagram-image").is_some(),
            "the preview is drawn"
        );
    }

    #[gpui::test]
    fn a_diagram_is_stored_as_a_mermaid_code_block(cx: &mut TestAppContext) {
        let (editor, cx) = insert_diagram(cx);
        let snapshot = editor.read_with(cx, |editor, _| editor.page().export_snapshot().unwrap());
        let reloaded = PageDocument::from_snapshot(2, &snapshot).unwrap();
        let first = reloaded.block_ids()[0];
        assert_eq!(reloaded.blocks()[0].kind, BlockKind::Code);
        assert_eq!(reloaded.block_language(first).as_deref(), Some("mermaid"));
        assert_eq!(reloaded.block_content(first).unwrap().text, EXAMPLE_DIAGRAM);

        // What builds from before diagrams wrote: a code block with no language stays code.
        let older = PageDocument::create(3, &["graph TD"]).unwrap();
        let code = older.ensure_block().unwrap();
        older.set_kind(code, BlockKind::Code).unwrap();
        let older = PageDocument::from_snapshot(4, &older.export_snapshot().unwrap()).unwrap();
        let (editor, cx) =
            cx.add_window_view(|_, cx| PageEditor::new(older, Selection::caret(code, 0), cx));
        assert!(!editor.read_with(cx, |editor, _| editor.is_diagram(code)));
    }

    #[gpui::test]
    fn an_edit_draws_the_diagram_again_and_an_error_keeps_the_last_drawing(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = insert_diagram(cx);
        let (first_image, _) = shown(&editor, cx);

        cx.simulate_keystrokes("enter");
        cx.simulate_input("    C --> D[Archive]");
        let (edited_image, error) = shown(&editor, cx);
        assert!(error.is_none(), "{error:?}");
        assert_ne!(edited_image, first_image, "the edit was drawn");

        let (again, _) = shown(&editor, cx);
        assert_eq!(
            again, edited_image,
            "an unchanged diagram is not drawn again"
        );

        // In one edit, as typing it a key at a time draws the states in between.
        editor.update(cx, |editor, cx| {
            let caret = editor.selection().range.head;
            editor.type_text(caret..caret, " -->", cx)
        });
        let (kept_image, error) = shown(&editor, cx);
        assert!(error.is_some(), "the broken source is reported");
        assert_eq!(kept_image, edited_image, "the last good drawing stays up");

        cx.simulate_keystrokes("backspace backspace backspace backspace");
        let (_, error) = shown(&editor, cx);
        assert!(error.is_none(), "fixing the source clears the error");
    }

    #[gpui::test]
    fn the_source_shows_only_while_editing(cx: &mut TestAppContext) {
        let (editor, cx) = insert_diagram(cx);
        assert!(is_editing(&editor, cx));
        let done = cx.debug_bounds("diagram-done").expect("a Done button");
        cx.simulate_click(done.center(), Modifiers::none());
        assert!(!is_editing(&editor, cx), "Done shows the diagram alone");

        let edit = cx.debug_bounds("diagram-edit").expect("an Edit button");
        cx.simulate_click(edit.center(), Modifiers::none());
        assert!(is_editing(&editor, cx), "Edit shows the source again");
    }

    #[gpui::test]
    fn arrowing_into_a_diagram_shows_its_source_from_the_nearest_end(cx: &mut TestAppContext) {
        let (editor, cx) = open_editor(&["above", ""], cx);
        let diagram = block(&editor, 1, cx);
        editor.update(cx, |editor, cx| {
            editor.convert_to_diagram(diagram, 0..0, cx)
        });
        let above = block(&editor, 0, cx);
        editor.update(cx, |editor, cx| editor.move_caret(above, 5, cx));
        cx.run_until_parked();
        let is_editing = |cx: &mut VisualTestContext| {
            editor.update_in(cx, |editor, window, _| {
                editor.is_editing_diagram(diagram, window)
            })
        };
        assert!(!is_editing(cx), "the caret left");

        cx.simulate_keystrokes("down");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection()),
            Selection::caret(diagram, 0)
        );
        assert!(is_editing(cx), "the source shows again");
    }

    #[gpui::test]
    fn a_viewer_sees_the_diagram_but_not_its_source(cx: &mut TestAppContext) {
        let (editor, cx) = insert_diagram(cx);
        editor.update(cx, |editor, cx| editor.set_access(PageAccess::ReadOnly, cx));
        cx.run_until_parked();
        assert!(!is_editing(&editor, cx));
        assert!(cx.debug_bounds("diagram-image").is_some());
        assert!(cx.debug_bounds("diagram-edit").is_none());
    }

    #[gpui::test]
    fn a_wide_diagram_is_scaled_down_to_the_column(cx: &mut TestAppContext) {
        let (editor, cx) = insert_diagram(cx);
        let natural = editor.read_with(cx, |editor, _| {
            let first = editor.page().block_ids()[0];
            editor.diagrams.view(first).drawn.unwrap().size
        });
        cx.simulate_resize(size(px(260.0), px(800.0)));
        cx.run_until_parked();
        let image = cx.debug_bounds("diagram-image").unwrap();
        assert!(image.size.width < natural.width, "{image:?} {natural:?}");
        let proportions = |size: gpui::Size<gpui::Pixels>| size.width / size.height;
        assert!((proportions(image.size) - proportions(natural)).abs() < 0.01);
    }
}
