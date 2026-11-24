//! #2993: a code block grows past one line. Enter and Shift+Enter in a code
//! block type a `"\n"` (ProseMirror's `newlineInCode`), Mod+Enter leaves it
//! for a paragraph after it (`exitCode`); outside a code block, and for a
//! selection that leaves the block, both are inapplicable and Enter splits,
//! Shift+Enter breaks, as before.
//!
//! Positions: `<pre>abcd</pre>` is the code block at 0 with its text at 1..5
//! (caret 3 is `ab|cd`) and its end at 6.

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::*;

fn state(html: &str, selection: Selection) -> EditorState {
    let s = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&s, html).expect("parses");
    let doc = s.branch("doc", slice.content).expect("a document");
    let st = EditorState::create(s, doc, default_plugins());
    let mut tr = st.tr();
    tr.set_selection(selection);
    st.apply(tr)
}

fn caret(html: &str, pos: usize) -> EditorState {
    state(html, Selection::cursor(Pos(pos)))
}

fn html(st: &EditorState) -> String {
    node_to_html(&st.doc)
}

/// The command the default keymap binds to `key`, run.
fn press(st: &EditorState, key: &str) -> Option<EditorState> {
    let name = st
        .keymap()
        .command_for(&KeyBinding::parse(key).unwrap())
        .unwrap_or_else(|| panic!("{key} is bound"))
        .to_string();
    st.run(&name)
}

#[test]
fn the_default_keymap_binds_the_code_block_keys() {
    let st = caret("<p>x</p>", 1);
    let bound = |k: &str| {
        st.keymap()
            .command_for(&KeyBinding::parse(k).unwrap())
            .map(str::to_string)
    };
    assert_eq!(bound("Enter").as_deref(), Some("enter"));
    assert_eq!(bound("Shift-Enter").as_deref(), Some("insertLineBreak"));
    assert_eq!(bound("Mod-Enter").as_deref(), Some("exitCode"));
}

#[test]
fn enter_in_a_code_block_types_a_newline_at_the_caret() {
    let after = press(&caret("<pre>abcd</pre>", 3), "Enter").expect("applies");
    assert_eq!(html(&after), "<pre>ab\ncd</pre>");
    assert_eq!(after.selection, Selection::cursor(Pos(4)));
    // Again, on the new line: a third line, still one block.
    let again = press(&after, "Enter").expect("applies");
    assert_eq!(html(&again), "<pre>ab\n\ncd</pre>");
    assert_eq!(again.selection, Selection::cursor(Pos(5)));
}

#[test]
fn enter_at_the_end_of_a_code_block_stays_in_it() {
    let after = press(&caret("<pre>abcd</pre><p>x</p>", 5), "Enter").expect("applies");
    assert_eq!(html(&after), "<pre>abcd\n</pre><p>x</p>");
    assert_eq!(after.selection, Selection::cursor(Pos(6)));
}

#[test]
fn shift_enter_in_a_code_block_types_a_newline_too() {
    let after = press(&caret("<pre>abcd</pre>", 2), "Shift-Enter").expect("applies");
    assert_eq!(html(&after), "<pre>a\nbcd</pre>");
    assert_eq!(after.selection, Selection::cursor(Pos(3)));
}

#[test]
fn the_newline_replaces_a_selection_in_the_block() {
    for key in ["Enter", "Shift-Enter"] {
        let st = state("<pre>abcd</pre>", Selection::text(Pos(2), Pos(4)));
        let after = press(&st, key).expect("applies");
        assert_eq!(html(&after), "<pre>a\nd</pre>", "{key}");
        assert_eq!(after.selection, Selection::cursor(Pos(3)), "{key}");
    }
}

#[test]
fn a_selection_leaving_the_code_block_is_not_a_newline() {
    // Anchor in the paragraph, head in the code block: not the same parent,
    // so `newlineInCode` and `exitCode` are inapplicable (ProseMirror's
    // `sameParent`), and Enter's chain splits as it does for any range.
    let st = state("<p>xy</p><pre>abcd</pre>", Selection::text(Pos(2), Pos(7)));
    assert!(st.run("newlineInCode").is_none());
    assert!(st.run("exitCode").is_none());
    let after = st.run("enter").expect("splits");
    assert!(!html(&after).contains('\n'), "{}", html(&after));
}

#[test]
fn enter_in_a_code_block_inside_a_list_item_is_a_newline_not_an_item() {
    // `<ul><li><pre>ab</pre></li></ul>`: list 0, item 1, code 2, text 3..5.
    let after = press(&caret("<ul><li><pre>ab</pre></li></ul>", 4), "Enter").expect("applies");
    assert_eq!(html(&after), "<ul><li><pre>a\nb</pre></li></ul>");
}

#[test]
fn mod_enter_leaves_the_code_block_for_a_paragraph_after_it() {
    // From the end and from the middle alike: the block is left as it is.
    for pos in [5, 3, 1] {
        let after = press(&caret("<pre>abcd</pre>", pos), "Mod-Enter").expect("applies");
        assert_eq!(html(&after), "<pre>abcd</pre><p></p>", "from {pos}");
        assert_eq!(after.selection, Selection::cursor(Pos(7)), "from {pos}");
    }
    // Between the block and the next one.
    let after = press(&caret("<pre>ab</pre><p>x</p>", 2), "Mod-Enter").expect("applies");
    assert_eq!(html(&after), "<pre>ab</pre><p></p><p>x</p>");
    assert_eq!(after.selection, Selection::cursor(Pos(5)));
    // Over a range in the block: the range is not deleted.
    let st = state("<pre>abcd</pre>", Selection::text(Pos(2), Pos(4)));
    let after = press(&st, "Mod-Enter").expect("applies");
    assert_eq!(html(&after), "<pre>abcd</pre><p></p>");
    assert_eq!(after.selection, Selection::cursor(Pos(7)));
}

#[test]
fn outside_a_code_block_the_keys_do_what_they_did() {
    let st = caret("<p>abcd</p>", 3);
    assert!(st.run("newlineInCode").is_none());
    assert!(st.run("exitCode").is_none());
    assert!(press(&st, "Mod-Enter").is_none());
    assert_eq!(
        html(&press(&st, "Enter").expect("splits")),
        "<p>ab</p><p>cd</p>"
    );
    assert_eq!(
        html(&press(&st, "Shift-Enter").expect("breaks")),
        "<p>ab<br>cd</p>"
    );
    // A heading is a textblock, not code.
    let h = caret("<h1>abcd</h1>", 3);
    assert!(h.run("newlineInCode").is_none());
    assert!(h.run("exitCode").is_none());
}

#[test]
fn a_multi_line_code_block_round_trips_through_html() {
    let typed = press(
        &press(&caret("<pre>abcd</pre>", 3), "Enter").unwrap(),
        "Enter",
    )
    .unwrap();
    let out = html(&typed);
    assert_eq!(out, "<pre>ab\n\ncd</pre>", "one block, three lines");
    // Read back with the same schema (a node's type compares by its schema).
    let s = typed.schema();
    let back = s
        .branch("doc", slice_from_html(s, &out).expect("parses").content)
        .expect("a document");
    assert_eq!(back, typed.doc, "{out}");
}

#[cfg(feature = "markdown")]
#[test]
fn a_multi_line_code_block_round_trips_through_markdown() {
    use rinch_editor_core::serialize::{doc_from_markdown, doc_to_markdown};
    let typed = press(&caret("<pre>abcd</pre>", 3), "Enter").unwrap();
    let md = doc_to_markdown(&typed.doc);
    assert_eq!(md.trim_end(), "```\nab\ncd\n```");
    let back = doc_from_markdown(typed.schema(), &md).expect("parses");
    assert_eq!(back, typed.doc, "{md}");
}
