use super::*;
use std::cell::Cell;

mod convergence;
mod read_only;
mod robustness;
mod sticky;
mod structures;

/// The concatenated text of every block in a handle's document, blocks
/// joined by `\n` — a cheap, layout-free convergence probe.
fn doc_text(h: &EditorHandle) -> String {
    fn collect(n: &Node, out: &mut String) {
        if let Some(t) = n.text() {
            out.push_str(t);
            return;
        }
        for i in 0..n.child_count() {
            collect(n.child(i), out);
        }
    }
    let doc = h.doc();
    let mut s = String::new();
    for i in 0..doc.child_count() {
        if i > 0 {
            s.push('\n');
        }
        collect(doc.child(i), &mut s);
    }
    s
}

/// Wire `host` and `guest` into a synchronous in-process loopback: each
/// side's outbound delta is delivered straight to the other's
/// `collab_receive`. Returns after the guest has adopted the host's
/// document.
fn loopback(host: &EditorHandle, guest: &EditorHandle) {
    let guest_in = guest.clone();
    let snapshot = host
        .start_collaboration_host(move |delta| {
            guest_in.collab_receive(&delta);
        })
        .expect("host projects its document");
    let host_in = host.clone();
    guest
        .start_collaboration_guest(&snapshot, move |delta| {
            host_in.collab_receive(&delta);
        })
        .expect("guest joins from the snapshot");
}

/// A peer's keystroke in the paragraph the caret is in leaves the caret in
/// that paragraph, where it was in the text, and asks the platform to
/// repaint the overlays. Both used to go wrong: the caret was carried to
/// the next paragraph, and on a runtime that refreshes the caret from its
/// input handlers (web) it stayed painted where the line used to end.
#[test]
fn a_remote_edit_in_the_carets_paragraph_keeps_the_caret_and_asks_for_a_repaint() {
    thread_local! {
        static REFRESHES: Cell<u32> = const { Cell::new(0) };
    }
    fn count_refresh() {
        REFRESHES.with(|n| n.set(n.get() + 1));
    }

    let s = schema();
    let host = mount(doc_node(
        &s,
        vec![para(&s, "Once in a while"), para(&s, "next")],
    ))
    .handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    // The guest's caret sits at the end of the first line; the host
    // backspaces that line's last letter.
    guest.set_selection(Selection::cursor(Pos(16)));
    crate::registry::set_overlay_refresher(count_refresh);
    REFRESHES.with(|n| n.set(0));
    host.set_selection(Selection::cursor(Pos(16)));
    host.ime_delete_surrounding(1, 0);
    assert_eq!(doc_text(&guest), "Once in a whil\nnext");

    assert_eq!(
        guest.state().selection.head(),
        Pos(15),
        "the caret stays at the end of its own line"
    );
    assert!(
        REFRESHES.with(|n| n.get()) >= 1,
        "the platform was asked to repaint the overlays"
    );
}

/// A peer typing in an earlier paragraph shifts the caret's position,
/// not its line: the caret-affinity hint (#301) follows it. A peer typing
/// before the caret in its own paragraph re-wraps it and drops the hint.
#[test]
fn a_remote_edit_above_carries_the_caret_affinity_and_one_beside_it_drops_it() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "top"), para(&s, "second line")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);
    // "second line" is 6..17; the guest's caret at "second| line" (12).
    guest.set_selection_with_affinity(Selection::cursor(Pos(12)), CaretAffinity::Upstream);
    host.set_selection(Selection::cursor(Pos(4)));
    assert!(host.insert_text("XY"));
    assert_eq!(doc_text(&guest), "topXY\nsecond line");
    assert_eq!(
        guest.selection(),
        Selection::cursor(Pos(14)),
        "shifted by two"
    );
    assert_eq!(
        guest.caret_affinity(),
        CaretAffinity::Upstream,
        "and still upstream"
    );

    host.set_selection(Selection::cursor(Pos(9)));
    assert!(host.insert_text("Z"));
    assert_eq!(doc_text(&guest), "topXY\nsZecond line");
    assert_eq!(guest.selection(), Selection::cursor(Pos(15)));
    assert_eq!(
        guest.caret_affinity(),
        CaretAffinity::Downstream,
        "its own block re-wrapped"
    );
}

/// Typing right after a link, through the handle, reaches the peer as plain
/// text: `link` is non-inclusive in the model, and the projection writes that
/// into the CRDT rather than leaving the char inside the link's range (where
/// yrs puts an insert at a range's end).
#[test]
fn a_char_typed_after_a_link_reaches_the_peer_outside_it() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "see here now")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    host.set_selection(Selection::text(Pos(5), Pos(9)));
    assert!(host.toggle_link("https://rust-lang.org"));
    loopback(&host, &guest);

    host.set_selection(Selection::cursor(Pos(9)));
    assert!(host.insert_text("X"));
    let expected = [
        ("see ".to_string(), false),
        ("here".to_string(), true),
        ("X now".to_string(), false),
    ];
    assert_eq!(runs_by_mark(&host.doc(), "link"), expected);
    assert_eq!(
        runs_by_mark(&guest.doc(), "link"),
        expected,
        "the guest sees the typed char outside the link too"
    );
    guest.set_selection(Selection::cursor(Pos(10)));
    assert_eq!(guest.active_link_href(), None);
}

// ── Read-only and collaboration ──────────────────────────────────────
//
// The pair of properties a read-only collaborator is for: what peers write
// still arrives, and nothing it does ever leaves.
