use super::*;

fn hinted() -> EditorHandle {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle;
    h.set_selection_with_affinity(Selection::cursor(Pos(4)), CaretAffinity::Upstream);
    assert_eq!(
        h.caret_affinity(),
        CaretAffinity::Upstream,
        "control: the hint holds"
    );
    assert_eq!(h.caret_affinity_at(Pos(4)), CaretAffinity::Upstream);
    assert_eq!(
        h.caret_affinity_at(Pos(3)),
        CaretAffinity::Downstream,
        "only at the head"
    );
    h
}

#[test]
fn a_plain_set_selection_clears_it_even_to_the_same_caret() {
    let h = hinted();
    h.set_selection(Selection::cursor(Pos(4)));
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}

#[test]
fn a_typed_character_clears_it() {
    let h = hinted();
    assert!(h.insert_text("x"));
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}

#[test]
fn an_edit_that_leaves_the_selection_keeps_it() {
    let h = hinted();
    // An edit in the second paragraph maps the caret onto itself.
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.delete(9, 10).ok()?;
        Some(tr)
    }));
    assert_eq!(h.selection(), Selection::cursor(Pos(4)));
    assert_eq!(h.caret_affinity(), CaretAffinity::Upstream);
}

/// An edit in an earlier paragraph shifts the caret's position but not
/// its place in its own line: the hint follows it (mapped, as
/// CodeMirror maps `assoc`).
#[test]
fn an_edit_in_an_earlier_block_carries_it_to_the_shifted_caret() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle;
    h.set_selection_with_affinity(Selection::cursor(Pos(10)), CaretAffinity::Upstream);
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.delete(2, 3).ok()?;
        Some(tr)
    }));
    assert_eq!(h.selection(), Selection::cursor(Pos(9)), "control: shifted");
    assert_eq!(h.caret_affinity(), CaretAffinity::Upstream);
}

/// An edit in the caret's own block before it re-wraps that block: the
/// hint is dropped, even though the caret only shifted.
#[test]
fn an_edit_before_the_caret_in_its_own_block_drops_it() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle;
    h.set_selection_with_affinity(Selection::cursor(Pos(11)), CaretAffinity::Upstream);
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.delete(8, 9).ok()?;
        Some(tr)
    }));
    assert_eq!(
        h.selection(),
        Selection::cursor(Pos(10)),
        "control: shifted"
    );
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}

/// A same-length replacement before the caret in its own block leaves the
/// caret's position and offset alone but can re-wrap its line: the hint
/// is dropped.
#[test]
fn a_same_length_replacement_before_the_caret_drops_it() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle;
    h.set_selection_with_affinity(Selection::cursor(Pos(11)), CaretAffinity::Upstream);
    assert!(h.update(|state| {
        let mut tr = state.tr();
        let text = state.schema().text("Q").ok()?;
        tr.replace_with(8, 9, Fragment::from_node(text)).ok()?;
        Some(tr)
    }));
    assert_eq!(
        h.selection(),
        Selection::cursor(Pos(11)),
        "control: unmoved"
    );
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}

/// The hint is already stored when the selection-change callback runs,
/// so a popup positioned from it sees the side the caret is drawn on.
#[test]
fn the_selection_callback_sees_the_new_hint() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle;
    let seen = Rc::new(RefCell::new(Vec::new()));
    h.on_selection_change({
        let (h, seen) = (h.clone(), seen.clone());
        move |_| seen.borrow_mut().push(h.caret_affinity())
    });
    h.set_selection_with_affinity(Selection::cursor(Pos(4)), CaretAffinity::Upstream);
    assert_eq!(*seen.borrow(), vec![CaretAffinity::Upstream]);
}

/// A selection that leaves the hinted one and comes back by another
/// route (select all, then an `update` back to the caret) does not
/// revive the old hint (PR #1019, round 3).
#[test]
fn a_selection_that_leaves_and_returns_does_not_revive_it() {
    let h = hinted();
    assert!(h.command("selectAll"));
    assert_ne!(h.selection(), Selection::cursor(Pos(4)), "control: left");
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(4)));
        Some(tr)
    }));
    assert_eq!(h.selection(), Selection::cursor(Pos(4)));
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}

/// An edit strictly after the caret in its own block — a peer typing at
/// the paragraph's end — leaves the caret's line alone: the hint stays.
#[test]
fn an_edit_after_the_caret_in_its_own_block_keeps_it() {
    let h = hinted();
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.delete(5, 6).ok()?;
        Some(tr)
    }));
    assert_eq!(h.selection(), Selection::cursor(Pos(4)), "control: unmoved");
    assert_eq!(h.caret_affinity(), CaretAffinity::Upstream);
}

/// A hint left inert by a selection that moved on is not carried onto
/// the new caret by a later edit.
#[test]
fn an_inert_hint_is_not_carried_onto_another_caret() {
    let h = hinted();
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(9)));
        Some(tr)
    }));
    assert_eq!(
        h.caret_affinity(),
        CaretAffinity::Downstream,
        "control: inert"
    );
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.delete(2, 3).ok()?;
        Some(tr)
    }));
    assert_eq!(h.selection(), Selection::cursor(Pos(8)), "control: shifted");
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}

/// Undoing an earlier-block edit shifts the caret back, and the hint
/// follows it both ways.
#[test]
fn undo_of_an_earlier_block_edit_carries_it_back() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle;
    h.set_selection_with_affinity(Selection::cursor(Pos(10)), CaretAffinity::Upstream);
    assert!(h.update(|state| {
        let mut tr = state.tr();
        tr.delete(2, 3).ok()?;
        Some(tr)
    }));
    assert_eq!(
        h.caret_affinity(),
        CaretAffinity::Upstream,
        "control: carried"
    );
    assert!(h.command("undo"));
    assert_eq!(h.selection(), Selection::cursor(Pos(10)));
    assert_eq!(h.caret_affinity(), CaretAffinity::Upstream);
}

#[test]
fn a_selection_moved_by_undo_clears_it() {
    let h = hinted();
    assert!(h.insert_text("x"));
    h.set_selection_with_affinity(Selection::cursor(Pos(5)), CaretAffinity::Upstream);
    assert!(h.command("undo"));
    assert_ne!(h.selection(), Selection::cursor(Pos(5)));
    assert_eq!(h.caret_affinity(), CaretAffinity::Downstream);
}
