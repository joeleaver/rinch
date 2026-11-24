//! Selection anchors carried through edits.

use super::*;

/// The property the whole anchor exists for: text typed **before** the
/// anchored position pushes it along, so a late insertion still lands where
/// the user asked rather than where the offset happened to point.
#[test]
fn an_anchor_is_carried_by_an_edit_in_front_of_it() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
    // Anchor at the space, i.e. "insert here" -> "hello| world".
    h.handle.set_selection(Selection::cursor(Pos(6)));
    let anchor = h.handle.anchor_selection();

    // The user keeps typing at the *start* of the line while we wait.
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.insert_text("XY"));

    let carried = anchor.selection().expect("still valid");
    assert_eq!(
        carried.head(),
        Pos(8),
        "two characters inserted ahead of the anchor move it by two"
    );

    // Inserting there splits the original text at the same *content* point.
    h.handle.set_selection(carried);
    assert!(h.handle.replace_selection_with_text("!"));
    let p = children(&h, h.container_id)[0];
    assert_eq!(text(&h, p).as_deref(), Some("XYhello! world"));
}

/// An edit *after* the anchor leaves it alone — the paste does not drift
/// toward text the user typed somewhere else.
#[test]
fn an_anchor_ignores_an_edit_behind_it() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
    h.handle.set_selection(Selection::cursor(Pos(6)));
    let anchor = h.handle.anchor_selection();

    h.handle.set_selection(Selection::cursor(Pos(12))); // end of line
    assert!(h.handle.insert_text("!!!"));

    assert_eq!(anchor.selection().expect("still valid").head(), Pos(6));
}

/// An anchored *range* (paste over a selection) survives as a range, so the
/// late paste still replaces what the user had selected.
#[test]
fn an_anchored_range_stays_a_range() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
    h.handle.set_selection(Selection::text(Pos(7), Pos(12))); // "world"
    let anchor = h.handle.anchor_selection();

    // Plain characters, not "* " — that would fire the bullet-list input
    // rule and restructure the block, which is the `load_doc` case, not this
    // one.
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.insert_text("AB"));

    let carried = anchor.selection().expect("still valid");
    assert_eq!((carried.from(), carried.to()), (Pos(9), Pos(14)));
    h.handle.set_selection(carried);
    assert!(h.handle.replace_selection_with_text("there"));
    let p = children(&h, h.container_id)[0];
    assert_eq!(text(&h, p).as_deref(), Some("ABhello there"));
}

/// A command's edits carry anchors too — `command` goes through
/// `run_mapped`, not just `update`, so a keymap-driven edit while a paste is
/// in flight is mapped like any other.
#[test]
fn a_command_carries_an_anchor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "one"), para(&s, "two")]));
    // Anchor in the *second* paragraph.
    h.handle.set_selection(Selection::cursor(Pos(8)));
    let anchor = h.handle.anchor_selection();
    let before = anchor.selection().unwrap().head();

    // Wrapping the first paragraph in a blockquote adds an opening token in
    // front of the anchor, so the anchor must shift.
    h.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(h.handle.command("wrapInBlockquote"));
    h.handle.set_selection(Selection::cursor(Pos(2)));

    let after = anchor.selection().expect("still valid").head();
    assert!(
        after.0 > before.0,
        "the anchor moved with the content the command pushed along \
         (before {before:?}, after {after:?})"
    );
}

/// Loading a new document invalidates outstanding anchors: the content the
/// user aimed at no longer exists, and quietly reusing the offset would drop
/// the paste into unrelated text.
#[test]
fn a_document_load_invalidates_an_anchor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
    h.handle.set_selection(Selection::cursor(Pos(6)));
    let anchor = h.handle.anchor_selection();
    assert!(anchor.selection().is_some());

    h.handle
        .load_doc(doc_node(&s, vec![para(&s, "something else")]));
    assert!(
        anchor.selection().is_none(),
        "an anchor into a replaced document must not resolve"
    );
}

/// A selection-only change is not a document change, so it neither moves nor
/// invalidates an anchor: the caret wandering is exactly the case the anchor
/// is meant to be immune to.
#[test]
fn moving_the_caret_does_not_disturb_an_anchor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello world")]));
    h.handle.set_selection(Selection::cursor(Pos(6)));
    let anchor = h.handle.anchor_selection();

    h.handle.set_selection(Selection::cursor(Pos(1)));
    h.handle.set_selection(Selection::text(Pos(2), Pos(4)));

    assert_eq!(anchor.selection().expect("still valid").head(), Pos(6));
}

/// Dropping an anchor releases it — an abandoned asynchronous operation must
/// not leave the editor mapping a position for the rest of its life.
#[test]
fn dropping_an_anchor_releases_it() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")]));
    let live = || h.handle.inner.borrow().anchors.borrow().live.len();

    let a = h.handle.anchor_selection();
    let b = h.handle.anchor_selection();
    assert_eq!(live(), 2);
    drop(a);
    assert_eq!(live(), 1);
    drop(b);
    assert_eq!(live(), 0, "no anchor outlives the value that owns it");
}
