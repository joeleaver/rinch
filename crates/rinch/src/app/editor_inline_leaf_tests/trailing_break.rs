//! #1172: a paragraph ending in a hard break, on desktop: its empty last
//! line, the view's trailing-break placeholder that makes it, and that line
//! along the real input paths (review of PR #1197), and through a copy and
//! paste (#3096).

use super::*;

/// #1172: a paragraph ending in a hard break shows the empty line after it,
/// and the caret after the break sits on it. rinch-dom now lays out
/// `<p>ab<br></p>` as one line, as Chrome does; the view's trailing-break
/// placeholder — a second, unmodelled `<br>` — is what makes the line.
#[test]
fn a_paragraph_ending_in_a_hard_break_shows_its_empty_line() {
    let mut p = page("<p>ab<br><br></p>");
    assert_eq!(
        p.handle.doc().child(0).child_count(),
        2,
        "positive control: `ab` and the break, no third node in the model"
    );
    assert_eq!(
        p.block_box().3,
        2.0 * LINE,
        "the empty line after the break"
    );
    assert_eq!(p.local(2).1, 0.0, "before the break: line 1");
    assert_eq!(p.local(3), (0.0, LINE), "after the break: line 2's start");
    let (bx, by, _, _) = p.block_box();
    let (cx, cy) = p.caret_drawn(3);
    assert!(
        (cx - bx).abs() < 1.5 && cy >= by + LINE - 1.0,
        "the drawn caret is at line 2's start: ({cx}, {cy}) in a block at ({bx}, {by})"
    );
    // A press on the empty line lands after the break.
    p.handle.set_selection(Selection::cursor(Pos(1)));
    idle(&mut p.app);
    p.click(bx + 100.0, by + 1.5 * LINE);
    assert_eq!(p.head(), 3, "the empty line: the paragraph's end");
    // Control: without the break, one line.
    let q = page("<p>ab</p>");
    assert_eq!(q.block_box().3, LINE);
}

/// Shift+Enter at a paragraph's end makes the line; Backspace takes it away.
#[test]
fn shift_enter_at_the_end_makes_the_line_and_backspace_removes_it() {
    let mut p = page("<p>ab</p>");
    p.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(p.handle.command("insertHardBreak"));
    idle(&mut p.app);
    assert_eq!(p.head(), 3, "after the break");
    assert_eq!(p.block_box().3, 2.0 * LINE);
    assert_eq!(p.local(3), (0.0, LINE));
    assert!(p.handle.command("deleteCharBackward"));
    idle(&mut p.app);
    assert_eq!(p.block_box().3, LINE, "the break and its line are gone");
    let doc = p.app.doc.as_ref().unwrap().borrow();
    assert!(
        doc.query_selector_all("[data-pm-trailing-break]")
            .is_empty(),
        "and so is the placeholder"
    );
}

// ── The trailing-break placeholder along the real input paths (review of PR #1197) ──

fn key(app: &mut RinchApp, key: KeyCode, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    idle(app);
}

fn placeholders(p: &Page) -> usize {
    let doc = p.app.doc.as_ref().unwrap().borrow();
    doc.query_selector_all("[data-pm-trailing-break]").len()
}

fn drawn_caret(p: &Page) -> Option<(f32, f32, f32, f32)> {
    let doc = p.app.doc.as_ref().unwrap().borrow();
    doc.query_selector_all("[data-pm-caret]")
        .into_iter()
        .map(|id| painted_element_box(&doc.tree, id.0))
        .find(|b| b.3 > 0.0)
}

/// Shift+Enter at the end through the real key path, then type a char.
#[test]
fn r_shift_enter_then_type_lands_on_line_two() {
    let mut p = page("<p>ab</p>");
    p.handle.set_selection(Selection::cursor(Pos(3)));
    idle(&mut p.app);
    key(&mut p.app, KeyCode::Enter, true);
    assert_eq!(p.handle.doc().child(0).child_count(), 2, "ab + break");
    assert_eq!(p.head(), 3);
    let (_, by, _, bh) = p.block_box();
    assert_eq!(bh, 2.0 * LINE);
    let c = drawn_caret(&p).expect("caret drawn");
    assert!(c.1 >= by + LINE - 1.0, "caret on line 2: {c:?}");
    assert!(p.handle.insert_text("x"));
    idle(&mut p.app);
    assert_eq!(p.handle.doc().child(0).child_count(), 3);
    assert_eq!(p.local(3), (0.0, LINE), "x starts line 2");
    assert_eq!(p.block_box().3, 2.0 * LINE, "still two lines");
    assert_eq!(placeholders(&p), 0);
}

#[test]
fn r_two_trailing_breaks() {
    let p = page("<p>ab<br><br><br></p>");
    assert_eq!(p.block_box().3, 3.0 * LINE);
    assert_eq!(p.local(3), (0.0, LINE));
    assert_eq!(p.local(4), (0.0, 2.0 * LINE));
    assert_eq!(placeholders(&p), 1);
}

#[test]
fn r_arrows_home_end_across_a_trailing_break() {
    let mut p = page("<p>ab<br><br></p><p>cd</p>");
    // Start at end of "ab" (before the break, char 2).
    p.handle.set_selection(Selection::cursor(Pos(3)));
    idle(&mut p.app);
    key(&mut p.app, KeyCode::ArrowDown, false);
    assert_eq!(p.head(), 3, "down: the empty line after the break");
    key(&mut p.app, KeyCode::ArrowDown, false);
    // next paragraph content starts at Pos(1 + 3 + 1 + 1) = 6
    assert!(
        p.handle.selection().head().0 >= 6,
        "into the next paragraph"
    );
    key(&mut p.app, KeyCode::ArrowUp, false);
    assert_eq!(p.head(), 3, "up: back to the empty line");
    key(&mut p.app, KeyCode::ArrowUp, false);
    assert!(p.head() <= 2, "up: line 1");
    // Home / End on the empty line.
    p.handle.set_selection(Selection::cursor(Pos(4)));
    idle(&mut p.app);
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), 3, "home on the empty line stays");
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), 3, "end on the empty line stays");
    // End on line 1: before the break.
    p.handle.set_selection(Selection::cursor(Pos(1)));
    idle(&mut p.app);
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), 2, "end on line 1: before the break");
    let c = drawn_caret(&p).expect("caret");
    let (_, by, _, _) = p.block_box();
    assert!(c.1 < by + LINE - 1.0, "drawn on line 1: {c:?}");
    // Backspace from the empty line removes the break and the line.
    p.handle.set_selection(Selection::cursor(Pos(4)));
    idle(&mut p.app);
    key(&mut p.app, KeyCode::Backspace, false);
    assert_eq!(p.block_box().3, LINE);
    assert_eq!(placeholders(&p), 0);
}

#[test]
fn r_undo_redo_around_the_placeholder() {
    let mut p = page("<p>ab</p>");
    p.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(p.handle.command("insertHardBreak"));
    idle(&mut p.app);
    assert_eq!(placeholders(&p), 1);
    assert!(p.handle.command("undo"));
    idle(&mut p.app);
    assert_eq!(placeholders(&p), 0);
    assert_eq!(p.block_box().3, LINE);
    assert!(p.handle.command("redo"));
    idle(&mut p.app);
    assert_eq!(placeholders(&p), 1);
    assert_eq!(p.block_box().3, 2.0 * LINE);
}

#[test]
fn r_serializers_and_copy_never_see_it() {
    let mut p = page("<p>ab<br><br></p><p>cd</p>");
    let html = rinch_editor_core::serialize::node_to_html(&p.handle.doc());
    assert!(!html.contains("trailing"), "{html}");
    // The break and the placeholder the writer adds for it (#3096), not the
    // view's own placeholder element.
    assert_eq!(html, "<p>ab<br><br></p><p>cd</p>");
    p.handle.set_selection(Selection::text(Pos(1), Pos(9)));
    idle(&mut p.app);
    let (h, t) = p.handle.selection_clipboard().unwrap();
    assert_eq!(h, "<p>ab<br><br></p><p>cd</p>");
    assert_eq!(t, "ab\n\ncd");
}

#[test]
fn r_placeholder_in_list_heading_table() {
    for (html, what) in [
        ("<ul><li><p>ab<br><br></p></li></ul>", "list item"),
        ("<h1>ab<br><br></h1>", "heading"),
        (
            "<table><tr><td><p>ab<br><br></p></td></tr></table>",
            "table cell",
        ),
    ] {
        let p = page(html);
        assert_eq!(placeholders(&p), 1, "{what}");
    }
}

/// A code block whose text ends in a newline — from a load, or typed —
/// keeps its empty last line, and the caret after the final newline is on
/// it. #1172 drops parley's line after a text-final newline, so the
/// placeholder has to cover a text child ending in `\n` as well as a hard
/// break (ProseMirror's rule); covering only the break left the caret at
/// the block's origin.
#[test]
fn a_code_block_ending_in_a_newline_keeps_its_last_line() {
    let mut p = page("<pre><code>ab</code></pre>");
    let one = p.block_box().3;
    p.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(p.handle.insert_text("\n"));
    idle(&mut p.app);
    assert_eq!(
        p.handle.doc().child(0).child(0).text(),
        Some("ab\n"),
        "positive control: the text ends in a newline"
    );
    assert_eq!(placeholders(&p), 1);
    let (lx, ly) = p.content_local(3);
    assert!(
        lx.abs() < 0.5 && ly >= LINE - 4.0,
        "after the final newline: line 2's start, got ({lx}, {ly})"
    );
    assert!(
        p.block_box().3 > one + 10.0,
        "a line: {one} -> {}",
        p.block_box().3
    );
    let (_, by, _, _) = p.block_box();
    let c = drawn_caret(&p).expect("caret");
    assert!(
        c.1 >= by + LINE - 4.0,
        "drawn on line 2: {c:?} (block top {by})"
    );
    // The load route: the parse keeps the newline.
    let q = page("<pre><code>ab\n</code></pre>");
    assert_eq!(q.handle.doc().child(0).child(0).text(), Some("ab\n"));
    assert_eq!(placeholders(&q), 1);
    assert!(
        q.content_local(3).1 >= LINE - 4.0,
        "{:?}",
        q.content_local(3)
    );
}

/// An empty paragraph draws one line, its `<p><br></p>` from another editor
/// too: a block's last `<br>` is the browser's placeholder (#3096).
#[test]
fn an_empty_paragraph_draws_one_line() {
    for html in ["<p></p>", "<p><br></p>"] {
        let p = page(html);
        assert_eq!(p.handle.doc().child(0).child_count(), 0, "{html}");
        assert_eq!(p.block_box().3, LINE, "{html}");
    }
}

/// A paragraph ending in a hard break, copied out of rinch and pasted back
/// over everything, is the same document with the same lines (#3096). Its
/// content alone copies as inline markup, `ab<br>`, which keeps its break
/// where nothing follows the caret; the paragraph itself is written
/// `<p>ab<br><br></p>`, which reads back as `ab` and one break.
#[test]
fn a_trailing_break_survives_copy_and_paste() {
    for whole in [false, true] {
        let mut p = page("<p>ab</p>");
        p.handle.set_selection(Selection::cursor(Pos(3)));
        idle(&mut p.app);
        key(&mut p.app, KeyCode::Enter, true);
        let before = p.handle.doc();
        let end = before.content_size();
        let (from, to, copied) = if whole {
            (0, end, "<p>ab<br><br></p>")
        } else {
            (1, end - 1, "ab<br>")
        };
        p.handle.set_selection(Selection::text(Pos(from), Pos(to)));
        idle(&mut p.app);
        let (html, text) = p.handle.selection_clipboard().expect("a copy");
        assert_eq!(html, copied);
        p.handle
            .set_selection(Selection::text(Pos(1), Pos(end - 1)));
        assert!(p.handle.paste(&rinch_editor_core::PasteContent {
            text: Some(text),
            html: Some(html),
        }));
        idle(&mut p.app);
        assert_eq!(p.handle.doc(), before, "the same document ({copied})");
        assert_eq!(p.block_box().3, 2.0 * LINE, "the same two lines");
        assert_eq!(placeholders(&p), 1);
        // The caret after the paste is on the empty line, and typing goes there.
        assert_eq!(p.head(), 3, "{copied}");
        assert!(p.handle.insert_text("x"));
        idle(&mut p.app);
        assert_eq!(p.local(3), (0.0, LINE), "x starts line 2 ({copied})");
    }
}
