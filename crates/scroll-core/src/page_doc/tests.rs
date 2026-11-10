use std::sync::Arc;

use super::history::UNDO_MERGE_INTERVAL_MS;
use super::*;
use loro::cursor::Side;

fn texts(doc: &PageDoc) -> Vec<String> {
    doc.blocks()
        .unwrap()
        .into_iter()
        .map(|block| block.text.clone())
        .collect()
}

fn block_with_text(doc: &PageDoc, placement: BlockPlacement, text: &str) -> BlockId {
    let id = doc.insert_block(placement, BlockKind::Paragraph).unwrap();
    doc.insert_text(id, 0, text).unwrap();
    id
}

#[test]
fn blocks_are_walked_in_document_order_with_depth() {
    let doc = PageDoc::new();
    let first = block_with_text(&doc, BlockPlacement::LastChildOf(None), "first");
    let third = block_with_text(&doc, BlockPlacement::After(first), "third");
    let child = block_with_text(&doc, BlockPlacement::FirstChildOf(Some(first)), "child");
    let second = block_with_text(&doc, BlockPlacement::After(first), "second");
    let top = block_with_text(&doc, BlockPlacement::FirstChildOf(None), "top");

    let blocks = doc.blocks().unwrap();
    let order: Vec<(BlockId, usize, Option<BlockId>)> = blocks
        .iter()
        .map(|block| (block.id, block.depth, block.parent))
        .collect();
    assert_eq!(
        order,
        vec![
            (top, 0, None),
            (first, 0, None),
            (child, 1, Some(first)),
            (second, 0, None),
            (third, 0, None)
        ]
    );
    assert_eq!(doc.block(child).unwrap().depth, 1);
}

#[test]
fn moving_and_deleting_blocks_carries_their_children() {
    let doc = PageDoc::new();
    let a = block_with_text(&doc, BlockPlacement::LastChildOf(None), "a");
    let b = block_with_text(&doc, BlockPlacement::LastChildOf(None), "b");
    let c = block_with_text(&doc, BlockPlacement::LastChildOf(None), "c");
    block_with_text(&doc, BlockPlacement::LastChildOf(Some(a)), "a child");

    doc.move_block(a, BlockPlacement::After(c)).unwrap();
    assert_eq!(texts(&doc), ["b", "c", "a", "a child"]);

    doc.move_block(b, BlockPlacement::LastChildOf(None))
        .unwrap();
    assert_eq!(texts(&doc), ["c", "a", "a child", "b"]);

    doc.move_block(c, BlockPlacement::LastChildOf(Some(a)))
        .unwrap();
    assert_eq!(texts(&doc), ["a", "a child", "c", "b"]);

    assert!(
        doc.move_block(a, BlockPlacement::FirstChildOf(Some(c)))
            .is_err()
    );

    doc.delete_block(a).unwrap();
    assert_eq!(texts(&doc), ["b"]);
    assert!(matches!(
        doc.insert_text(a, 0, "x"),
        Err(PageDocError::BlockNotFound(_))
    ));
}

#[test]
fn block_kind_and_to_do_state_change() {
    let doc = PageDoc::new();
    let id = doc
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    assert!(matches!(
        doc.toggle_to_do(id),
        Err(PageDocError::NotAToDo(_))
    ));

    doc.set_block_kind(id, BlockKind::ToDo).unwrap();
    doc.toggle_to_do(id).unwrap();
    let block = doc.block(id).unwrap();
    assert_eq!((block.kind, block.is_checked), (BlockKind::ToDo, true));

    doc.toggle_to_do(id).unwrap();
    assert!(!doc.block(id).unwrap().is_checked);

    doc.set_block_kind(id, BlockKind::Heading2).unwrap();
    assert_eq!(doc.block(id).unwrap().kind, BlockKind::Heading2);
}

#[test]
fn text_edits_use_byte_offsets() {
    let doc = PageDoc::new();
    let id = block_with_text(&doc, BlockPlacement::LastChildOf(None), "héllo");
    doc.insert_text(id, "hé".len(), "-").unwrap();
    assert_eq!(doc.block(id).unwrap().text, "hé-llo");

    doc.delete_text(id, 1..4).unwrap();
    assert_eq!(doc.block(id).unwrap().text, "hllo");

    // Byte 2 of "é" would split a character.
    let accented = block_with_text(&doc, BlockPlacement::LastChildOf(None), "é");
    assert!(matches!(
        doc.insert_text(accented, 1, "x"),
        Err(PageDocError::InvalidOffset { .. })
    ));
    assert!(matches!(
        doc.insert_text(accented, 9, "x"),
        Err(PageDocError::InvalidOffset { .. })
    ));
    assert!(matches!(
        doc.apply_mark(accented, 0..1, &Mark::Bold),
        Err(PageDocError::InvalidOffset { .. })
    ));
    assert!(matches!(
        doc.cursor(accented, 1, Side::Left),
        Err(PageDocError::InvalidOffset { .. })
    ));
}

#[test]
fn marks_are_read_back_as_spans() {
    let doc = PageDoc::new();
    let id = block_with_text(&doc, BlockPlacement::LastChildOf(None), "bold and link");
    doc.apply_mark(id, 0..4, &Mark::Bold).unwrap();
    doc.apply_mark(id, 0..4, &Mark::Italic).unwrap();
    doc.apply_mark(id, 9..13, &Mark::Link("https://example.com".into()))
        .unwrap();

    let spans = doc.block(id).unwrap().spans.clone();
    assert_eq!(
        spans,
        vec![
            TextSpan {
                range: 0..4,
                marks: vec![Mark::Bold, Mark::Italic]
            },
            TextSpan {
                range: 4..9,
                marks: vec![]
            },
            TextSpan {
                range: 9..13,
                marks: vec![Mark::Link("https://example.com".into())]
            },
        ]
    );

    doc.remove_mark(id, 0..2, MarkKind::Bold).unwrap();
    let spans = doc.block(id).unwrap().spans.clone();
    assert_eq!(
        spans[0],
        TextSpan {
            range: 0..2,
            marks: vec![Mark::Italic]
        }
    );
    assert_eq!(
        spans[1],
        TextSpan {
            range: 2..4,
            marks: vec![Mark::Bold, Mark::Italic]
        }
    );
}

#[test]
fn bold_extends_when_typing_at_its_end_but_links_do_not() {
    let doc = PageDoc::new();
    let id = block_with_text(&doc, BlockPlacement::LastChildOf(None), "ab");
    doc.apply_mark(id, 0..1, &Mark::Bold).unwrap();
    doc.apply_mark(id, 1..2, &Mark::Link("u".into())).unwrap();
    doc.insert_text(id, 1, "X").unwrap();
    doc.insert_text(id, 3, "Y").unwrap();

    let spans = doc.block(id).unwrap().spans.clone();
    assert_eq!(
        spans,
        vec![
            TextSpan {
                range: 0..2,
                marks: vec![Mark::Bold]
            },
            TextSpan {
                range: 2..3,
                marks: vec![Mark::Link("u".into())]
            },
            TextSpan {
                range: 3..4,
                marks: vec![]
            },
        ]
    );
}

#[test]
fn concurrent_edits_converge_after_exchanging_updates() {
    let mut alice = PageDoc::new();
    let shared = block_with_text(&alice, BlockPlacement::LastChildOf(None), "hello");
    let mut bob = PageDoc::new();
    bob.import(&alice.take_local_update().unwrap().unwrap())
        .unwrap();

    // Both edit the same block, and the block list, without seeing each other's changes.
    alice.insert_text(shared, 5, " world").unwrap();
    alice.apply_mark(shared, 0..5, &Mark::Bold).unwrap();
    block_with_text(&alice, BlockPlacement::After(shared), "from alice");
    bob.insert_text(shared, 0, "oh, ").unwrap();
    bob.set_block_kind(shared, BlockKind::Heading1).unwrap();
    block_with_text(&bob, BlockPlacement::After(shared), "from bob");

    let from_alice = alice.take_local_update().unwrap().unwrap();
    let from_bob = bob.take_local_update().unwrap().unwrap();
    alice.import(&from_bob).unwrap();
    bob.import(&from_alice).unwrap();

    assert_eq!(alice.blocks().unwrap(), bob.blocks().unwrap());
    let merged = alice.block(shared).unwrap();
    assert_eq!(merged.text, "oh, hello world");
    assert_eq!(merged.kind, BlockKind::Heading1);
    // Whether "oh, " joins the bold depends on how the peers' ids order the concurrent
    // insert against the mark's start; either way "hello" stays bold on both sides.
    let hello = "oh, ".len().."oh, hello".len();
    let bold_hello = merged.spans.iter().any(|span| {
        span.range.start <= hello.start
            && hello.end <= span.range.end
            && span.marks.contains(&Mark::Bold)
    });
    assert!(bold_hello, "spans were {:?}", merged.spans);
    assert_eq!(alice.blocks().unwrap().len(), 3);
}

#[test]
fn local_updates_exclude_imported_changes_and_are_taken_once() {
    let mut alice = PageDoc::new();
    let mut bob = PageDoc::new();
    let block = block_with_text(&alice, BlockPlacement::LastChildOf(None), "a");
    let first = alice.take_local_update().unwrap().unwrap();
    assert_eq!(alice.take_local_update().unwrap(), None);

    bob.import(&first).unwrap();
    bob.insert_text(block, 1, "b").unwrap();
    let from_bob = bob.take_local_update().unwrap().unwrap();
    // Bob's update holds only Bob's edit, so it is useless without Alice's first update.
    let stranger = PageDoc::new();
    stranger.import(&from_bob).unwrap();
    assert!(stranger.blocks().unwrap().is_empty());
    stranger.import(&first).unwrap();
    assert_eq!(texts(&stranger), ["ab"]);
}

#[test]
fn snapshots_round_trip() {
    let doc = PageDoc::new();
    let id = block_with_text(&doc, BlockPlacement::LastChildOf(None), "kept");
    doc.apply_mark(id, 0..4, &Mark::Strikethrough).unwrap();

    let restored = PageDoc::from_snapshot(&doc.export_snapshot().unwrap()).unwrap();
    assert_eq!(restored.blocks().unwrap(), doc.blocks().unwrap());
    // Editing after a restore still works and still uses the mark configuration.
    restored.insert_text(id, 4, "!").unwrap();
    assert_eq!(
        restored.block(id).unwrap().spans[0].marks,
        vec![Mark::Strikethrough]
    );
}

#[test]
fn cursors_follow_their_characters_and_resolve_without_knowing_the_block() {
    let alice = PageDoc::new();
    block_with_text(&alice, BlockPlacement::LastChildOf(None), "first");
    let id = block_with_text(&alice, BlockPlacement::LastChildOf(None), "world");
    let cursor = alice.cursor(id, 0, Side::Middle).unwrap();

    let bob = PageDoc::from_snapshot(&alice.export_snapshot().unwrap()).unwrap();
    bob.insert_text(id, 0, "héllo ").unwrap();
    alice.import(&bob.export_snapshot().unwrap()).unwrap();

    assert_eq!(
        alice.cursor_position(&cursor).unwrap(),
        (id, "héllo ".len())
    );
    // A cursor made on one replica resolves on another.
    assert_eq!(bob.cursor_position(&cursor).unwrap(), (id, "héllo ".len()));

    let stranger = PageDoc::new();
    assert!(matches!(
        stranger.cursor_position(&cursor),
        Err(PageDocError::CursorNotInPage)
    ));
    alice.delete_block(id).unwrap();
    // A deleted block's text is no longer part of the page.
    assert!(matches!(
        alice.cursor_position(&cursor),
        Err(PageDocError::CursorNotInPage)
    ));
}

/// A replica whose undo history is empty, holding one block with `text`.
fn fresh_replica(text: &str) -> (PageDoc, BlockId) {
    let source = PageDoc::new();
    let id = block_with_text(&source, BlockPlacement::LastChildOf(None), text);
    (
        PageDoc::from_snapshot(&source.export_snapshot().unwrap()).unwrap(),
        id,
    )
}

#[test]
fn undo_reverts_only_local_edits_and_restores_the_selection() {
    let (mut alice, id) = fresh_replica("base");
    let bob = PageDoc::from_snapshot(&alice.export_snapshot().unwrap()).unwrap();
    alice.remember_selection(&[alice.cursor(id, 4, Side::Middle).unwrap()]);
    alice.insert_text(id, 4, " alice").unwrap();
    bob.insert_text(id, 0, "bob ").unwrap();
    alice.import(&bob.export_snapshot().unwrap()).unwrap();

    let selection = alice.undo().unwrap();
    assert_eq!(alice.block(id).unwrap().text, "bob base");
    // The caret goes back to where it was, shifted by Bob's insert.
    let restored: Vec<_> = selection
        .iter()
        .map(|cursor| alice.cursor_position(cursor).unwrap())
        .collect();
    assert_eq!(restored, vec![(id, "bob base".len())]);

    alice.redo().unwrap();
    assert_eq!(alice.block(id).unwrap().text, "bob base alice");
    assert!(alice.undo().is_ok());
}

#[test]
fn several_edits_undo_as_one_step_when_grouped() {
    let (mut doc, id) = fresh_replica("a");
    doc.edit_as_one_step(|doc| {
        doc.insert_text(id, 1, "b")?;
        // Far enough apart that, ungrouped, these would be two undo steps.
        wait_for_a_new_undo_step();
        let next = doc.insert_block(BlockPlacement::After(id), BlockKind::Quote)?;
        doc.insert_text(next, 0, "c")
    })
    .unwrap();
    assert_eq!(texts(&doc), ["ab", "c"]);

    doc.undo().unwrap();
    assert_eq!(texts(&doc), ["a"]);
}

#[test]
fn subscribers_hear_which_blocks_changed() {
    let alice = PageDoc::new();
    let changes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = changes.clone();
    let _subscription =
        alice.subscribe(move |change| recorded.lock().unwrap().push(change.clone()));

    let id = alice
        .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
        .unwrap();
    assert!(
        changes
            .lock()
            .unwrap()
            .iter()
            .any(|change| change.is_structure_changed)
    );

    changes.lock().unwrap().clear();
    alice.insert_text(id, 0, "a").unwrap();
    alice.set_block_kind(id, BlockKind::Quote).unwrap();
    let local = changes.lock().unwrap().clone();
    assert!(
        local
            .iter()
            .all(|change| !change.is_structure_changed && change.blocks == vec![id])
    );
    assert_eq!(local.len(), 2);

    changes.lock().unwrap().clear();
    let bob = PageDoc::from_snapshot(&alice.export_snapshot().unwrap()).unwrap();
    bob.insert_text(id, 1, "b").unwrap();
    alice.import(&bob.export_snapshot().unwrap()).unwrap();
    assert!(
        changes
            .lock()
            .unwrap()
            .iter()
            .any(|change| change.blocks == vec![id])
    );
}

#[test]
fn splitting_moves_the_tail_with_its_marks_in_one_undo_step() {
    let (mut doc, first) = fresh_replica("hello world");
    doc.apply_mark(first, 6..11, &Mark::Italic).unwrap();
    wait_for_a_new_undo_step();

    let second = doc.split_block(first, 5, BlockKind::ToDo).unwrap();
    assert_eq!(texts(&doc), ["hello", " world"]);
    let second_block = doc.block(second).unwrap();
    assert_eq!(second_block.kind, BlockKind::ToDo);
    assert_eq!(
        second_block.spans,
        vec![
            TextSpan {
                range: 0..1,
                marks: vec![]
            },
            TextSpan {
                range: 1..6,
                marks: vec![Mark::Italic]
            },
        ]
    );

    doc.undo().unwrap();
    assert_eq!(texts(&doc), ["hello world"]);
    // Only the split was undone: the italic applied before it stays.
    assert_eq!(doc.block(first).unwrap().spans[1].marks, vec![Mark::Italic]);
}

/// Loro merges local edits closer together than its merge interval into one undo step.
fn wait_for_a_new_undo_step() {
    std::thread::sleep(std::time::Duration::from_millis(
        UNDO_MERGE_INTERVAL_MS as u64 + 50,
    ));
}

#[test]
fn splitting_a_block_with_children_puts_the_tail_first_among_them() {
    let doc = PageDoc::new();
    let parent = block_with_text(&doc, BlockPlacement::LastChildOf(None), "parent");
    block_with_text(&doc, BlockPlacement::LastChildOf(Some(parent)), "child");
    let tail = doc.split_block(parent, 3, BlockKind::Paragraph).unwrap();
    assert_eq!(texts(&doc), ["par", "ent", "child"]);
    assert_eq!(doc.block(tail).unwrap().parent, Some(parent));

    let end = doc.split_block(tail, 3, BlockKind::Paragraph).unwrap();
    assert_eq!(doc.block(end).unwrap().text, "");
}

#[test]
fn merging_appends_text_with_marks_and_lifts_children() {
    let doc = PageDoc::new();
    let top = block_with_text(&doc, BlockPlacement::LastChildOf(None), "bold");
    doc.apply_mark(top, 0..4, &Mark::Bold).unwrap();
    let nested = block_with_text(&doc, BlockPlacement::LastChildOf(Some(top)), "nested");
    let merged = block_with_text(&doc, BlockPlacement::After(top), " plain");
    let grandchild = block_with_text(&doc, BlockPlacement::LastChildOf(Some(merged)), "kid");

    // The block above `merged` in document order is `nested`, the last child of `top`.
    assert_eq!(doc.merge_into_previous(merged).unwrap(), Some((nested, 6)));
    assert_eq!(texts(&doc), ["bold", "nested plain", "kid"]);
    assert_eq!(doc.block(grandchild).unwrap().parent, None);

    assert_eq!(doc.merge_into_previous(nested).unwrap(), Some((top, 4)));
    let merged_block = doc.block(top).unwrap();
    assert_eq!(merged_block.text, "boldnested plain");
    // The appended text keeps its own (absent) marks rather than picking up the bold before it.
    assert_eq!(
        merged_block.spans,
        vec![
            TextSpan {
                range: 0..4,
                marks: vec![Mark::Bold]
            },
            TextSpan {
                range: 4..16,
                marks: vec![]
            },
        ]
    );
    assert_eq!(doc.merge_into_previous(top).unwrap(), None);
}

#[test]
fn merging_is_one_undo_step() {
    let (mut doc, first) = fresh_replica("one");
    let second = doc
        .insert_block(BlockPlacement::After(first), BlockKind::Paragraph)
        .unwrap();
    doc.insert_text(second, 0, "two").unwrap();
    wait_for_a_new_undo_step();

    doc.merge_into_previous(second).unwrap();
    assert_eq!(texts(&doc), ["onetwo"]);
    doc.undo().unwrap();
    assert_eq!(texts(&doc), ["one", "two"]);
}

#[test]
fn the_peer_id_can_be_set_only_with_no_unsent_edits() {
    let mut doc = PageDoc::new();
    doc.set_peer_id(7).unwrap();
    let id = block_with_text(&doc, BlockPlacement::LastChildOf(None), "a");
    assert!(matches!(
        doc.set_peer_id(8),
        Err(PageDocError::UnsentLocalEdits)
    ));

    let update = doc.take_local_update().unwrap().unwrap();
    let receiver = PageDoc::new();
    receiver.import(&update).unwrap();
    assert_eq!(receiver.blocks().unwrap()[0].id.peer, 7);

    doc.set_peer_id(8).unwrap();
    doc.insert_text(id, 1, "b").unwrap();
    receiver
        .import(&doc.take_local_update().unwrap().unwrap())
        .unwrap();
    assert_eq!(texts(&receiver), ["ab"]);
}

#[test]
fn blocks_can_be_placed_and_moved_before_a_sibling() {
    let doc = PageDoc::new();
    let last = block_with_text(&doc, BlockPlacement::LastChildOf(None), "last");
    let first = block_with_text(&doc, BlockPlacement::Before(last), "first");
    let child = block_with_text(&doc, BlockPlacement::LastChildOf(Some(first)), "child");
    let before_child = block_with_text(&doc, BlockPlacement::Before(child), "before child");
    assert_eq!(texts(&doc), ["first", "before child", "child", "last"]);
    assert_eq!(doc.block(before_child).unwrap().parent, Some(first));

    doc.move_block(last, BlockPlacement::Before(first)).unwrap();
    assert_eq!(texts(&doc), ["last", "first", "before child", "child"]);
}

#[test]
fn cached_blocks_follow_local_edits_imports_undo_and_structure_changes() {
    let (mut doc, id) = fresh_replica("one");
    let bob = PageDoc::from_snapshot(&doc.export_snapshot().unwrap()).unwrap();
    // Warm the cache before each change, so a stale cache would show.
    assert_eq!(texts(&doc), ["one"]);
    doc.insert_text(id, 3, "!").unwrap();
    assert_eq!(doc.block(id).unwrap().text, "one!");

    assert_eq!(texts(&doc), ["one!"]);
    bob.set_block_kind(id, BlockKind::Quote).unwrap();
    doc.import(&bob.export_snapshot().unwrap()).unwrap();
    assert_eq!(doc.block(id).unwrap().kind, BlockKind::Quote);

    assert_eq!(texts(&doc), ["one!"]);
    doc.undo().unwrap();
    assert_eq!(texts(&doc), ["one"]);

    let second = doc.split_block(id, 1, BlockKind::Paragraph).unwrap();
    assert_eq!(texts(&doc), ["o", "ne"]);
    doc.delete_block(second).unwrap();
    assert_eq!(texts(&doc), ["o"]);
    assert!(matches!(
        doc.block(second),
        Err(PageDocError::BlockNotFound(_))
    ));
}

#[test]
fn a_page_doc_can_move_between_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<PageDoc>();
}

#[test]
#[ignore = "timing; run with --release --ignored to measure"]
fn reading_blocks_after_a_keystroke_in_a_large_page_is_quick() {
    let doc = PageDoc::new();
    let mut previous = None;
    for index in 0..5_000 {
        let placement = previous.map_or(BlockPlacement::LastChildOf(None), BlockPlacement::After);
        let id = doc.insert_block(placement, BlockKind::Paragraph).unwrap();
        doc.insert_text(id, 0, &format!("Block number {index} with some text in it"))
            .unwrap();
        previous = Some(id);
    }
    let target = previous.unwrap();

    let reopened = PageDoc::from_snapshot(&doc.export_snapshot().unwrap()).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(reopened.blocks().unwrap().len(), 5_000);
    println!("reading 5,000 blocks from scratch: {:?}", started.elapsed());

    let mut timings = Vec::new();
    for _ in 0..50 {
        let started = std::time::Instant::now();
        reopened.insert_text(target, 0, "x").unwrap();
        assert_eq!(reopened.blocks().unwrap().len(), 5_000);
        timings.push(started.elapsed());
    }
    timings.sort();
    println!(
        "a keystroke then blocks(): median {:?}, max {:?}",
        timings[timings.len() / 2],
        timings.last().unwrap()
    );
}

#[test]
fn a_block_icon_is_set_kept_across_kinds_synced_and_removed() {
    let doc = PageDoc::new();
    let id = block_with_text(&doc, BlockPlacement::LastChildOf(None), "Note");
    // Documents written before icons existed read as having none.
    assert_eq!(doc.block(id).unwrap().icon, None);
    let cached = doc.blocks().unwrap();
    assert_eq!(cached[0].icon, None);

    // A family emoji is several code points joined; it is stored as one string.
    doc.set_block_icon(id, Some("👨‍👩‍👧")).unwrap();
    assert_eq!(doc.blocks().unwrap()[0].icon.as_deref(), Some("👨‍👩‍👧"));
    doc.set_block_kind(id, BlockKind::Heading2).unwrap();
    assert_eq!(doc.block(id).unwrap().icon.as_deref(), Some("👨‍👩‍👧"));

    let replica = PageDoc::from_snapshot(&doc.export_snapshot().unwrap()).unwrap();
    assert_eq!(replica.block(id).unwrap().icon.as_deref(), Some("👨‍👩‍👧"));

    doc.set_block_icon(id, None).unwrap();
    assert_eq!(doc.blocks().unwrap()[0].icon, None);
}
