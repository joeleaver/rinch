//! The scroll gate: when a caret change scrolls the caret into view.

use super::*;

/// The handle→request plumbing: `update_caret` must *fulfil* the view's
/// `ScrollSelectionIntoView` by calling `scroll_into_view()` on the caret
/// element, and must do it only when the caret moved. The mock queues those
/// calls the way the desktop backend does, so the queue is what we read.
///
/// Before this, the returned `Vec<ViewRequest>` was dropped on the floor and
/// `ScrollSelectionIntoView` had no consumer anywhere in the tree.
#[test]
fn update_caret_scrolls_the_caret_into_view_only_when_it_moved() {
    let s = schema();
    let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
    let h = mount(doc_node(&s, vec![empty(), empty()]));

    // Measure the two paragraphs so the caret has somewhere to land (the mock
    // lays nothing out; `__set_node_layout` is the injection point).
    let blocks = children(&h, h.container_id);
    {
        let mut m = h.mock.borrow_mut();
        for (i, b) in blocks.iter().enumerate() {
            m.__set_node_layout(*b, 0.0, i as f32 * 20.0, 200.0, 20.0);
        }
    }
    let drained = |h: &Harness| h.doc.borrow_mut().drain_scroll_into_view_requests();

    // doc(p(), p()) → 0[p 1]2[p 3]4. The fresh editor's cursor is at 1, and
    // placing it there scrolls nothing (focus alone is not a move — #846).
    h.handle.update_caret();
    assert!(
        drained(&h).is_empty(),
        "placing an unmoved caret scrolls nothing"
    );
    h.handle.set_selection(Selection::cursor(Pos(3)));
    h.handle.update_caret();
    let first = drained(&h);
    assert_eq!(first.len(), 1, "a caret move scrolls to it");
    assert_eq!(
        h.doc.borrow().get_attribute(first[0], "data-pm-caret"),
        Some("true".to_string()),
        "and it is the caret element that was scrolled to"
    );

    // The runtime's post-layout pass runs on every frame; an unchanged caret
    // must not keep re-scrolling, or a user who scrolled away is dragged back.
    h.handle.update_caret();
    h.handle.update_caret();
    assert!(drained(&h).is_empty(), "a repeat pass scrolls nothing");

    h.handle.set_selection(Selection::cursor(Pos(1)));
    h.handle.update_caret();
    assert_eq!(drained(&h).len(), 1, "a moved caret scrolls again");
}

// ── The scroll gate: what brings the caret into view, and what does not ──

/// `doc(p(), p(), p())` — 0[p 1]2[p 3]4[p 5]6 — with the three paragraphs
/// measured as stacked 200x20 boxes, and the caret pass drained once so the
/// focus-gained arm is spent. Empty paragraphs because the mock has no text
/// layout: an empty block's caret is the one caret path it can place.
fn gate_rig() -> Harness {
    let s = schema();
    let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
    let h = mount(doc_node(&s, vec![empty(), empty(), empty()]));
    measure(&h, &[0.0, 20.0, 40.0]);
    h.handle.set_selection(Selection::cursor(Pos(5)));
    h.handle.update_caret();
    assert_eq!(
        drain(&h).len(),
        1,
        "positive control: first placement scrolls"
    );
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "positive control: the gate is idle");
    h
}

fn caret_top(h: &Harness) -> Option<String> {
    let d = h.mock.borrow();
    (0..d.__node_count())
        .map(NodeId::from_index)
        .find(|id| d.get_attribute(*id, "data-pm-caret").is_some())
        .and_then(|id| d.get_attribute(id, "style"))
}

/// An insertion into paragraph 1 that does **not** set the selection — it is
/// mapped, the shape a collaborator's edit or an app's own edit takes.
fn edit_above_without_moving(h: &Harness) {
    assert!(h.handle.update(|state| {
        let mut tr = state.tr();
        let text = state.schema().text("x").unwrap();
        tr.replace_with(1, 1, Fragment::from_node(text)).unwrap();
        Some(tr)
    }));
}

/// Geometry moving under a caret the user did not move must not scroll —
/// the four ways #837's review measured it (a resize reflow, a remote or app
/// edit above the caret, a virtualized block being measured) all reduce to
/// "the caret overlay moved, and nothing local changed the selection".
#[test]
fn a_caret_that_moves_without_a_local_selection_change_does_not_scroll() {
    let h = gate_rig();
    let before = caret_top(&h);

    // Layout moves (a reflow above the caret).
    measure(&h, &[0.0, 20.0, 90.0]);
    h.handle.update_caret();
    assert_ne!(
        caret_top(&h),
        before,
        "positive control: the caret overlay moved"
    );
    assert!(
        drain(&h).is_empty(),
        "a reflow under the caret does not scroll"
    );

    // An edit above it that only maps the selection.
    let sel = h.handle.selection();
    edit_above_without_moving(&h);
    assert_ne!(
        h.handle.selection(),
        sel,
        "positive control: the caret was mapped"
    );
    measure(&h, &[0.0, 20.0, 130.0]);
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "an edit elsewhere does not scroll");

    // A load replaces the document and resets the selection.
    let s = schema();
    let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
    h.handle
        .load_doc(doc_node(&s, vec![empty(), empty(), empty()]));
    measure(&h, &[0.0, 20.0, 40.0]);
    h.handle.update_caret();
    assert!(
        caret_top(&h).is_some(),
        "positive control: the loaded doc has a caret"
    );
    assert!(drain(&h).is_empty(), "a load does not scroll");
}

/// The other side: a local change scrolls even when the caret's geometry does
/// **not** change (a Delete at the caret; here an explicit selection set to
/// where the caret already is, over an edit elsewhere) — the view reports
/// nothing moved, and it is the handle's gate that asks.
#[test]
fn a_local_selection_change_scrolls_even_when_the_caret_does_not_move() {
    let h = gate_rig();
    let before = caret_top(&h);
    assert!(h.handle.update(|state| {
        let mut tr = state.tr();
        let text = state.schema().text("x").unwrap();
        tr.replace_with(1, 1, Fragment::from_node(text)).unwrap();
        let mapped = tr.selection();
        tr.set_selection(mapped);
        Some(tr)
    }));
    h.handle.update_caret();
    assert_eq!(
        caret_top(&h),
        before,
        "positive control: the caret did not move"
    );
    assert_eq!(drain(&h).len(), 1, "an explicit selection set scrolls");
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "once");
}

/// A scroll asked for while the caret cannot be placed (its block is not
/// measured yet — a fresh paragraph from Enter, a virtualized block) waits for
/// the pass that can place it, rather than being spent on a pass with nothing
/// to scroll to.
#[test]
fn a_scroll_waits_for_a_caret_that_can_be_placed() {
    let s = schema();
    let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
    let h = mount(doc_node(&s, vec![empty(), empty()]));
    // Only the first paragraph is measured.
    measure(&h, &[0.0]);
    h.handle.set_selection(Selection::cursor(Pos(3)));
    h.handle.update_caret();
    assert!(caret_top(&h).is_none(), "positive control: no caret yet");
    assert!(drain(&h).is_empty());
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "still nothing to scroll to");
    measure(&h, &[0.0, 20.0]);
    h.handle.update_caret();
    assert_eq!(
        drain(&h).len(),
        1,
        "the owed scroll lands once the caret exists"
    );
    h.handle.update_caret();
    assert!(drain(&h).is_empty());
}

/// Focus alone does not scroll. The runtime's sweep hides an unfocused
/// editor's overlays and places the focused one's, and the only way to focus
/// an editor is a press: one that places the caret arms through
/// `set_selection`, and one that does not (a task checkbox) must not pull
/// the user to the old caret. A scroll owed from while the editor was
/// unfocused is still paid on the first focused pass.
#[test]
fn regaining_focus_alone_does_not_scroll_but_an_owed_scroll_is_paid() {
    let h = gate_rig();
    h.handle.hide_overlays();
    h.handle.update_caret();
    assert!(
        caret_top(&h).is_some(),
        "positive control: the caret is back"
    );
    assert!(drain(&h).is_empty(), "focus gained scrolls nothing");

    h.handle.hide_overlays();
    h.handle.set_selection(Selection::cursor(Pos(3)));
    h.handle.hide_overlays();
    assert!(drain(&h).is_empty(), "an unfocused editor scrolls nothing");
    h.handle.update_caret();
    assert_eq!(drain(&h).len(), 1, "the owed scroll lands on focus");
}

/// Read-only (#832): a refused edit arms nothing, and a selection move — which
/// read-only allows — still scrolls.
#[test]
fn read_only_refused_edits_do_not_scroll_and_selection_moves_do() {
    let h = gate_rig();
    h.handle.set_read_only(true);
    assert!(!h.handle.update(|state| {
        let mut tr = state.tr();
        let text = state.schema().text("x").unwrap();
        tr.replace_with(1, 1, Fragment::from_node(text)).unwrap();
        tr.set_selection(Selection::cursor(Pos(1)));
        Some(tr)
    }));
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "a refused edit scrolls nothing");
    h.handle.set_selection(Selection::cursor(Pos(1)));
    h.handle.update_caret();
    assert_eq!(drain(&h).len(), 1, "a read-only caret move scrolls");
}

/// A paste is the user's input, so a plugin's claim scrolls the caret into
/// view even when its transaction only maps the selection — the very
/// transaction that, dispatched through `update`, is an edit elsewhere and
/// does not scroll (above).
#[test]
fn a_claimed_paste_scrolls_even_when_it_does_not_set_the_selection() {
    struct InsertsAbove;
    impl Plugin for InsertsAbove {
        fn key(&self) -> rinch_editor_core::PluginKey {
            rinch_editor_core::PluginKey("test.inserts-above")
        }
        fn handle_paste(&self, state: &EditorState, _paste: &PasteContent) -> Option<Transaction> {
            let mut tr = state.tr();
            let text = state.schema().text("x").unwrap();
            tr.replace_with(1, 1, Fragment::from_node(text)).unwrap();
            Some(tr)
        }
    }
    let h = gate_rig();
    assert!(h.handle.add_plugin(Rc::new(InsertsAbove)));
    h.handle.update_caret();
    assert!(
        drain(&h).is_empty(),
        "positive control: adding it scrolls nothing"
    );
    let sel = h.handle.selection();
    assert!(h.handle.paste(&PasteContent::text("y")));
    assert_ne!(
        h.handle.selection(),
        sel,
        "positive control: the caret was mapped, not set"
    );
    h.handle.update_caret();
    assert_eq!(drain(&h).len(), 1, "the paste brings the caret into view");
}

/// A picture that lands late, away from the person's caret
/// ([`EditorHandle::insert_image_at`]), owes no scroll: their selection did
/// not change, and the view must not jump to where the picture went. When
/// the selection is still the anchor's, it is the ordinary insert and does.
#[test]
fn a_late_picture_away_from_the_caret_owes_no_scroll() {
    let handle = crate::create_editor();
    assert!(handle.load_html("<p>hello world</p><p>second</p>"));
    handle.set_selection(Selection::cursor(Pos(6)));
    let anchor = handle.anchor_selection();
    handle.set_selection(Selection::cursor(Pos(20)));
    handle.core_mut().scroll.pending = false;
    assert!(handle.insert_image_at(&anchor, "app-blob:1", ""));
    assert!(!handle.core().scroll.pending, "the person did not move");

    let here = handle.anchor_selection();
    assert!(handle.insert_image_at(&here, "app-blob:2", ""));
    assert!(
        handle.core().scroll.pending,
        "control: an insert at the caret"
    );
}

/// The gate's state machine directly: movement alone never scrolls, an owed
/// scroll waits for an anchor and is spent by it, and a text range drops it.
#[test]
fn the_scroll_gate_waits_for_an_anchor_and_a_range_drops_it() {
    let mut g = ScrollGate::default();
    assert!(!g.on_pass(true, false), "idle: never scrolls");
    g.arm();
    assert!(!g.on_pass(false, false), "no anchor: stay owed");
    assert!(g.on_pass(true, false), "anchor: scroll");
    assert!(!g.on_pass(true, false), "spent");

    g.arm();
    assert!(!g.on_pass(false, true), "a range drops the owed scroll");
    assert!(
        !g.on_pass(true, false),
        "so a later collapse does not fire it"
    );
}
