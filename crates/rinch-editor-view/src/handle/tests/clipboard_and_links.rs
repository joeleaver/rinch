//! The clipboard, paste and links.

use super::*;

#[test]
fn selection_clipboard_serializes_marked_run() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    // Empty selection → nothing to copy.
    h.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(h.handle.selection_clipboard().is_none());

    // Bold the whole word, select it, copy.
    h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
    assert!(h.handle.command("toggleBold"));
    let (html, plain) = h
        .handle
        .selection_clipboard()
        .expect("non-empty selection copies");
    assert_eq!(html, "<strong>abcd</strong>", "rich HTML carries the mark");
    assert_eq!(plain, "abcd", "plain text drops the mark");
}

#[test]
fn paste_html_inserts_rich_content_at_cursor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab")]));
    h.handle.set_selection(Selection::cursor(Pos(2))); // between a and b
    assert!(h.handle.replace_selection_with_html("<strong>X</strong>"));

    // "a" + bold "X" + "b" inside the one paragraph.
    let p = children(&h, h.container_id)[0];
    assert_eq!(text(&h, p).as_deref(), Some("aXb"));
    let strong = children(&h, p)
        .into_iter()
        .find(|&c| tag(&h, c).as_deref() == Some("strong"));
    assert!(strong.is_some(), "pasted bold run wrapped in <strong>");
    // Cursor lands after the inserted run (model pos 3: 0[p 1 a X b]).
    assert!(
        h.handle.selection().is_empty(),
        "paste collapses the cursor"
    );
    assert_eq!(h.handle.selection().head(), Pos(3));
}

#[test]
fn insert_image_places_an_inline_image_node() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab")]));
    h.handle.set_selection(Selection::cursor(Pos(2))); // between "a" and "b"
    assert!(h.handle.insert_image("data:image/png;base64,AAAA", "shot"));
    // The image lands inline in the paragraph, between the text runs.
    let p = children(&h, h.container_id)[0];
    let img = children(&h, p)
        .into_iter()
        .find(|&c| tag(&h, c).as_deref() == Some("img"));
    assert!(img.is_some(), "image node placed inline in the paragraph");
    let img = img.unwrap();
    assert_eq!(
        h.doc.borrow().get_attribute(img, "src").as_deref(),
        Some("data:image/png;base64,AAAA")
    );
}

#[test]
fn toggle_link_sets_and_removes_a_link_mark() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "click here")]));
    // Select "click", then link it.
    h.handle.set_selection(Selection::text(Pos(1), Pos(6)));
    assert!(!h.handle.is_mark_active("link"));
    assert!(h.handle.toggle_link("https://example.com"), "link applies");
    assert!(h.handle.is_mark_active("link"), "state reports link active");
    assert_eq!(
        h.handle.active_link_href().as_deref(),
        Some("https://example.com")
    );

    // The host re-projected: the run is now wrapped in <a href=...>.
    let p = children(&h, h.container_id)[0];
    let anchor = children(&h, p)
        .into_iter()
        .find(|&c| tag(&h, c).as_deref() == Some("a"));
    assert!(anchor.is_some(), "anchor projected into the host");
    assert_eq!(
        h.doc
            .borrow()
            .get_attribute(anchor.unwrap(), "href")
            .as_deref(),
        Some("https://example.com")
    );

    // Toggling again over the same range removes the link.
    assert!(h.handle.toggle_link("https://ignored.example"));
    assert!(
        !h.handle.is_mark_active("link"),
        "link removed on re-toggle"
    );
    assert_eq!(h.handle.active_link_href(), None);
}

#[test]
fn toggle_link_is_a_noop_on_a_collapsed_cursor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abc")]));
    h.handle.set_selection(Selection::cursor(Pos(2)));
    // A link needs a range — the collapsed toggle changes nothing.
    assert!(!h.handle.toggle_link("https://example.com"));
    assert!(!h.handle.is_mark_active("link"));
    assert_eq!(h.handle.active_link_href(), None);
}

#[test]
fn active_link_href_reads_the_target_from_inside_a_link() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "linked text")]));
    h.handle.set_selection(Selection::text(Pos(1), Pos(12)));
    assert!(h.handle.toggle_link("https://rust-lang.org"));
    // Collapse the cursor inside the linked run — the href is still readable.
    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert_eq!(
        h.handle.active_link_href().as_deref(),
        Some("https://rust-lang.org")
    );
}

#[test]
fn a_range_over_two_different_links_reports_the_first() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "one two three")]));
    // "two" (5..8) and "three" (9..14) link to different targets.
    h.handle.set_selection(Selection::text(Pos(5), Pos(8)));
    assert!(h.handle.toggle_link("https://first.example"));
    h.handle.set_selection(Selection::text(Pos(9), Pos(14)));
    assert!(h.handle.toggle_link("https://second.example"));
    // The range starts in plain text before both and ends at the second's end.
    h.handle.set_selection(Selection::text(Pos(2), Pos(14)));
    assert_eq!(
        h.handle.active_link_href().as_deref(),
        Some("https://first.example")
    );
    // Selected the other way round, it is still the first in document order.
    h.handle.set_selection(Selection::text(Pos(14), Pos(2)));
    assert_eq!(
        h.handle.active_link_href().as_deref(),
        Some("https://first.example")
    );
}

#[test]
fn a_caret_right_after_a_link_is_outside_it_and_typing_there_is_plain() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "see here now")]));
    // Link "here" (5..9).
    h.handle.set_selection(Selection::text(Pos(5), Pos(9)));
    assert!(h.handle.toggle_link("https://rust-lang.org"));
    assert_eq!(
        h.handle.active_link_href().as_deref(),
        Some("https://rust-lang.org"),
        "a range over the link reports it, though its end is the link's end"
    );

    h.handle.set_selection(Selection::cursor(Pos(7)));
    assert_eq!(
        h.handle.active_link_href().as_deref(),
        Some("https://rust-lang.org"),
        "inside"
    );
    assert!(h.handle.is_mark_active("link"));
    h.handle.set_selection(Selection::cursor(Pos(5)));
    assert_eq!(h.handle.active_link_href(), None, "at its start");
    h.handle.set_selection(Selection::cursor(Pos(9)));
    assert_eq!(h.handle.active_link_href(), None, "right after it");
    assert!(!h.handle.is_mark_active("link"));

    // Typing there, and the IME's commit, stay outside the link.
    assert!(h.handle.insert_text("X"));
    h.handle.ime_commit("ね");
    assert_eq!(
        runs_by_mark(&h.handle.doc(), "link"),
        [
            ("see ".to_string(), false),
            ("here".to_string(), true),
            ("Xね now".to_string(), false)
        ]
    );
    // The host shows the same: the anchor holds only the link's own text.
    let host_p = children(&h, h.container_id)[0];
    let anchor = children(&h, host_p)
        .into_iter()
        .find(|&c| tag(&h, c).as_deref() == Some("a"))
        .expect("the link is projected");
    assert_eq!(text(&h, anchor).as_deref(), Some("here"));
}

#[test]
fn a_caret_right_after_bold_still_types_bold() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "see here now")]));
    h.handle.set_selection(Selection::text(Pos(5), Pos(9)));
    assert!(h.handle.command("toggleBold"));
    h.handle.set_selection(Selection::cursor(Pos(9)));
    assert!(h.handle.is_mark_active("bold"));
    assert!(h.handle.insert_text("X"));
    assert_eq!(
        runs_by_mark(&h.handle.doc(), "bold"),
        [
            ("see ".to_string(), false),
            ("hereX".to_string(), true),
            (" now".to_string(), false)
        ]
    );
}

#[test]
fn paste_text_over_selection_replaces_it() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")]));
    h.handle.set_selection(Selection::text(Pos(1), Pos(6))); // whole word
    assert!(h.handle.replace_selection_with_text("bye"));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("bye")
    );
    assert!(h.handle.selection().is_empty());
}

#[test]
fn paste_multiline_text_splits_blocks() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "")]));
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.replace_selection_with_text("one\ntwo"));
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 2, "two lines → two paragraphs");
    assert_eq!(text(&h, blocks[0]).as_deref(), Some("one"));
    assert_eq!(text(&h, blocks[1]).as_deref(), Some("two"));
}

#[test]
fn cut_then_paste_round_trips() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    // "Cut" cd: copy then delete the selection.
    h.handle.set_selection(Selection::text(Pos(3), Pos(5)));
    let (html, _plain) = h.handle.selection_clipboard().expect("copies");
    assert!(h.handle.command("deleteSelection"));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("ab")
    );

    // Paste it back at the end.
    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(h.handle.replace_selection_with_html(&html));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("abcd")
    );
}

// ── Node-views (NodeSelection of an image / horizontal rule) ─────────────
