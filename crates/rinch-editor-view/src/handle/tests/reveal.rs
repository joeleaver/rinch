use super::*;
use std::cell::Cell;

/// Five empty paragraphs — 0[p 1]2[p 3]4[p 5]6[p 7]8[p 9]10 — as 200x20
/// boxes stacked from y = 0. Empty because the mock has no text layout:
/// an empty block's caret is the one caret it can place.
fn five() -> Harness {
    let s = schema();
    let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
    mount(doc_node(&s, (0..5).map(|_| empty()).collect()))
}

fn measured() -> Harness {
    let h = five();
    measure(&h, &[0.0, 20.0, 40.0, 60.0, 80.0]);
    h
}

/// `(top, height)` of a scrolled-to probe, read off its inline style.
fn probe_box(h: &Harness, id: NodeId) -> (String, String) {
    let d = h.mock.borrow();
    assert_eq!(
        d.get_attribute(id, "data-pm-reveal").as_deref(),
        Some("true"),
        "the scroll target is a reveal probe"
    );
    let style = d.get_attribute(id, "style").unwrap_or_default();
    let get = |name: &str| {
        style
            .split(';')
            .map(str::trim)
            .find_map(|decl| decl.strip_prefix(&format!("{name}: ")))
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(get("visibility"), "hidden", "a probe is never painted");
    (get("top"), get("height"))
}

fn px(v: f32) -> String {
    format!("{v}px")
}

#[test]
fn a_range_scrolls_to_its_end_then_its_start_with_a_margin() {
    let h = measured();
    h.handle.scroll_into_view(Pos(3), Pos(7));
    assert!(h.handle.reveal_pass(), "it placed the probes");
    let requests = drain(&h);
    assert_eq!(requests.len(), 2, "two scrolls: {requests:?}");
    // The end (paragraph 3, y 60) first, then the start (paragraph 1,
    // y 20), each 16px above and below its 20px line.
    assert_eq!(probe_box(&h, requests[0]), (px(44.0), px(52.0)));
    assert_eq!(probe_box(&h, requests[1]), (px(4.0), px(52.0)));
    assert_eq!(h.handle.pending_reveal(), None, "fulfilled");
    assert!(!h.handle.reveal_pass(), "and not again");
    assert!(drain(&h).is_empty());
}

#[test]
fn the_ends_may_come_in_either_order_and_a_caret_is_one_probe() {
    let h = measured();
    h.handle.scroll_into_view(Pos(7), Pos(3));
    h.handle.reveal_pass();
    let requests = drain(&h);
    assert_eq!(
        probe_box(&h, requests[1]),
        (px(4.0), px(52.0)),
        "the start last"
    );

    h.handle.scroll_into_view(Pos(5), Pos(5));
    assert!(h.handle.reveal_pass());
    let requests = drain(&h);
    assert_eq!(requests.len(), 1, "a caret is one scroll");
    assert_eq!(probe_box(&h, requests[0]), (px(24.0), px(52.0)));
}

#[test]
fn the_margin_stays_inside_the_document() {
    let h = measured();
    // First line: nothing above y 0. Last line: nothing below y 100.
    h.handle.scroll_into_view(Pos(1), Pos(9));
    h.handle.reveal_pass();
    let requests = drain(&h);
    assert_eq!(probe_box(&h, requests[0]), (px(64.0), px(36.0)), "end");
    assert_eq!(probe_box(&h, requests[1]), (px(0.0), px(36.0)), "start");
}

#[test]
fn a_position_between_blocks_reveals_the_nearest_text() {
    let h = measured();
    // 4 is between paragraphs 1 and 2: the start looks forward, the end back.
    h.handle.scroll_into_view(Pos(4), Pos(4));
    h.handle.reveal_pass();
    let requests = drain(&h);
    assert_eq!(probe_box(&h, requests[0]), (px(24.0), px(52.0)));
}

#[test]
fn a_reveal_waits_for_geometry_and_needs_no_focus() {
    let h = five();
    let sel = h.handle.selection();
    h.handle.scroll_into_view(Pos(7), Pos(7));
    assert!(!h.handle.reveal_pass(), "nothing is laid out");
    assert!(drain(&h).is_empty(), "so nothing scrolls");
    assert_eq!(h.handle.pending_reveal(), Some(Pos(7)), "and it waits");

    measure(&h, &[0.0, 20.0, 40.0, 60.0, 80.0]);
    assert!(h.handle.reveal_pass(), "the first pass with geometry");
    assert_eq!(drain(&h).len(), 1);
    assert_eq!(h.handle.selection(), sel, "the selection is untouched");
}

#[test]
fn a_later_request_replaces_an_unfulfilled_one() {
    let h = five();
    h.handle.scroll_into_view(Pos(9), Pos(9));
    h.handle.scroll_into_view(Pos(3), Pos(3));
    measure(&h, &[0.0, 20.0, 40.0, 60.0, 80.0]);
    h.handle.reveal_pass();
    let requests = drain(&h);
    assert_eq!(requests.len(), 1);
    assert_eq!(probe_box(&h, requests[0]), (px(4.0), px(52.0)));
}

#[test]
fn a_local_edit_carries_a_pending_reveal_and_a_load_drops_it() {
    let h = five();
    h.handle.scroll_into_view(Pos(5), Pos(7));
    // Type into paragraph 0: everything after it moves up by one.
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.insert_text("x"));
    assert_eq!(h.handle.pending_reveal(), Some(Pos(6)));

    h.handle.load_html("<p>new</p>");
    assert_eq!(
        h.handle.pending_reveal(),
        None,
        "a load names a new document"
    );
}

/// The placed requests the reveal queued, `(probe, fraction, margin)`.
fn drain_placed(h: &Harness) -> Vec<(NodeId, f32, f32)> {
    h.mock.borrow_mut().drain_scroll_to_fraction_requests()
}

#[test]
fn a_fraction_places_one_unmargined_probe_over_the_start() {
    let h = measured();
    h.handle
        .scroll_into_view_aligned(Pos(3), Pos(7), ScrollAlign::Fraction(1.0 / 3.0));
    assert!(h.handle.reveal_pass());
    assert!(drain(&h).is_empty(), "no nearest scroll");
    let placed = drain_placed(&h);
    assert_eq!(placed.len(), 1, "one placement: {placed:?}");
    let (probe, fraction, margin) = placed[0];
    // Paragraph 1's line (y 20, 20 tall), not grown: the placement
    // keeps the margin itself.
    assert_eq!(probe_box(&h, probe), (px(20.0), px(20.0)));
    assert!((fraction - 1.0 / 3.0).abs() < 1e-6);
    assert_eq!(margin, crate::view::REVEAL_MARGIN);
    assert_eq!(h.handle.pending_reveal(), None, "fulfilled");
}

#[test]
fn a_fraction_is_clamped_and_nearest_is_what_scroll_into_view_does() {
    let h = measured();
    h.handle
        .scroll_into_view_aligned(Pos(5), Pos(5), ScrollAlign::Fraction(4.0));
    h.handle.reveal_pass();
    assert_eq!(drain_placed(&h)[0].1, 1.0);
    h.handle
        .scroll_into_view_aligned(Pos(5), Pos(5), ScrollAlign::Fraction(f32::NAN));
    h.handle.reveal_pass();
    assert_eq!(drain_placed(&h)[0].1, 0.0);

    h.handle
        .scroll_into_view_aligned(Pos(3), Pos(7), ScrollAlign::Nearest);
    h.handle.reveal_pass();
    let requests = drain(&h);
    assert!(drain_placed(&h).is_empty());
    assert_eq!(probe_box(&h, requests[0]), (px(44.0), px(52.0)));
    assert_eq!(probe_box(&h, requests[1]), (px(4.0), px(52.0)));
}

#[test]
fn a_placed_reveal_waits_for_geometry_and_is_carried_by_edits() {
    let h = five();
    h.handle
        .scroll_into_view_aligned(Pos(5), Pos(7), ScrollAlign::Fraction(0.5));
    assert!(!h.handle.reveal_pass(), "nothing is laid out");
    assert!(drain_placed(&h).is_empty(), "so nothing scrolls");
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.insert_text("x"));
    assert_eq!(h.handle.pending_reveal(), Some(Pos(6)), "carried");
    measure(&h, &[0.0, 20.0, 40.0, 60.0, 80.0]);
    assert!(h.handle.reveal_pass(), "the first pass with geometry");
    let placed = drain_placed(&h);
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].1, 0.5, "still placed, not nearest");
}

/// An insertion exactly at a pending range's edges stays outside it:
/// the start maps forward, the end back. Pins the direction of each
/// bias, which an insertion strictly before the range cannot (#922's
/// review, M4).
#[test]
fn an_insertion_at_either_edge_of_a_pending_reveal_stays_outside_it() {
    let h = five();
    h.handle.scroll_into_view(Pos(5), Pos(7));
    // At the start (paragraph 2 is empty, 5 is its content start).
    h.handle.set_selection(Selection::cursor(Pos(5)));
    assert!(h.handle.insert_text("x"));
    assert_eq!(
        h.handle.pending_reveal_range(),
        Some((Pos(6), Pos(8))),
        "the start moves past text inserted at it"
    );
    // At the end: paragraph 3's content now starts at 8.
    h.handle.set_selection(Selection::cursor(Pos(8)));
    assert!(h.handle.insert_text("y"));
    assert_eq!(
        h.handle.pending_reveal_range(),
        Some((Pos(6), Pos(8))),
        "the end stays before text inserted at it"
    );
}

/// A range whose end has no geometry (a virtualized editor's end
/// block, still collapsed) reveals its start alone rather than waiting
/// for good (#922's review, D3).
#[test]
fn a_range_whose_end_has_no_geometry_reveals_its_start() {
    let h = five();
    // Paragraphs 0..=2 measured; paragraph 4 (the end) is not.
    measure(&h, &[0.0, 20.0, 40.0]);
    h.handle.scroll_into_view(Pos(3), Pos(9));
    assert!(h.handle.reveal_pass(), "fulfilled on the start alone");
    let requests = drain(&h);
    assert_eq!(requests.len(), 1, "one scroll: {requests:?}");
    assert_eq!(probe_box(&h, requests[0]), (px(4.0), px(52.0)));
    assert_eq!(h.handle.pending_reveal(), None);
}

/// The start stays required: with no geometry for it the reveal waits.
#[test]
fn a_range_whose_start_has_no_geometry_waits() {
    let h = five();
    measure(&h, &[0.0]);
    h.handle.scroll_into_view(Pos(3), Pos(9));
    assert!(!h.handle.reveal_pass());
    assert!(drain(&h).is_empty());
    assert_eq!(h.handle.pending_reveal(), Some(Pos(3)));
}

/// A reveal that no pass can fulfil (no geometry, ever) is dropped after
/// `REVEAL_PATIENCE` passes, not kept to scroll much later; one short
/// of that it still waits and is fulfilled. A new request starts the
/// count again.
#[test]
fn an_unfulfillable_reveal_expires_after_its_patience() {
    let h = five();
    h.handle.scroll_into_view(Pos(7), Pos(7));
    for _ in 1..REVEAL_PATIENCE {
        assert!(!h.handle.reveal_pass());
    }
    assert_eq!(h.handle.pending_reveal(), Some(Pos(7)), "one short: waits");
    measure(&h, &[0.0, 20.0, 40.0, 60.0, 80.0]);
    assert!(h.handle.reveal_pass(), "and is fulfilled");
    drain(&h);

    let h = five();
    h.handle.scroll_into_view(Pos(7), Pos(7));
    for _ in 1..REVEAL_PATIENCE {
        h.handle.reveal_pass();
    }
    h.handle.scroll_into_view(Pos(5), Pos(5));
    h.handle.reveal_pass();
    assert_eq!(
        h.handle.pending_reveal(),
        Some(Pos(5)),
        "a new request, a new count"
    );
    for _ in 1..REVEAL_PATIENCE {
        h.handle.reveal_pass();
    }
    assert_eq!(h.handle.pending_reveal(), None, "expired");
    measure(&h, &[0.0, 20.0, 40.0, 60.0, 80.0]);
    assert!(!h.handle.reveal_pass(), "shown later, it scrolls nothing");
    assert!(drain(&h).is_empty());
}

#[test]
fn before_mount_both_are_no_ops() {
    let h = crate::create_editor();
    h.scroll_into_view(Pos(1), Pos(1));
    assert_eq!(
        h.pending_reveal(),
        None,
        "nothing is remembered for the mount"
    );
    FOCUSED.with(|f| f.set(None));
    crate::registry::set_focus_handler(record_focus);
    h.focus();
    assert_eq!(FOCUSED.with(Cell::get), None, "nothing to focus");
}

thread_local! {
    static FOCUSED: Cell<Option<u64>> = const { Cell::new(None) };
}

fn record_focus(container: u64) {
    FOCUSED.with(|f| f.set(Some(container)));
}

#[test]
fn focus_hands_the_container_to_the_platform_and_moves_nothing() {
    let h = measured();
    h.handle.set_selection(Selection::text(Pos(3), Pos(7)));
    drain(&h);
    FOCUSED.with(|f| f.set(None));
    crate::registry::set_focus_handler(record_focus);
    h.handle.focus();
    assert_eq!(FOCUSED.with(Cell::get), Some(h.container_id.0));
    assert_eq!(h.handle.selection(), Selection::text(Pos(3), Pos(7)));
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "focus scrolls nothing");
}

thread_local! {
    static BLURRED: Cell<Option<u64>> = const { Cell::new(None) };
}

fn record_blur(container: u64) {
    BLURRED.with(|f| f.set(Some(container)));
}

#[test]
fn blur_before_mount_is_a_no_op() {
    let h = crate::create_editor();
    BLURRED.with(|f| f.set(None));
    crate::registry::set_blur_handler(record_blur);
    h.blur();
    assert_eq!(BLURRED.with(Cell::get), None, "nothing to blur");
}

#[test]
fn blur_hands_the_container_to_the_platform_and_keeps_the_selection() {
    let h = measured();
    h.handle.set_selection(Selection::text(Pos(3), Pos(7)));
    drain(&h);
    BLURRED.with(|f| f.set(None));
    crate::registry::set_blur_handler(record_blur);
    h.handle.blur();
    assert_eq!(BLURRED.with(Cell::get), Some(h.container_id.0));
    assert_eq!(h.handle.selection(), Selection::text(Pos(3), Pos(7)));
    h.handle.update_caret();
    assert!(drain(&h).is_empty(), "blur scrolls nothing");
}

/// Desktop registers no blur handler: the handle parks a release for
/// its container, in posting order with a focus request.
#[test]
fn without_a_handler_blur_parks_a_release_after_a_focus() {
    let h = measured();
    let key = h.doc.borrow().doc_key();
    let _ = rinch_core::take_pending_focus_steps(key, true);
    // The mock document posts no focus request of its own, so park the
    // one desktop's `focus()` would.
    rinch_core::request_focus(key, h.container_id.0);
    h.handle.blur();
    assert_eq!(
        rinch_core::take_pending_focus_steps(key, true),
        vec![
            rinch_core::FocusStep::Request(rinch_core::FocusRequest::Node(h.container_id.0)),
            rinch_core::FocusStep::Release(h.container_id.0),
        ]
    );
}

#[test]
fn a_request_wakes_its_document_until_the_next_overlay_pass() {
    let h = five();
    let key = h.doc.borrow().doc_key();
    assert!(!crate::registry::reveal_owed(key));
    h.handle.scroll_into_view(Pos(1), Pos(1));
    assert!(crate::registry::reveal_owed(key), "owed after the request");
    crate::registry::update_all_carets(Some(key.wrapping_add(1)), None);
    assert!(
        crate::registry::reveal_owed(key),
        "another document's pass leaves it"
    );
    crate::registry::update_all_carets(Some(key), None);
    assert!(
        !crate::registry::reveal_owed(key),
        "the document's own pass clears it, fulfilled or not"
    );
    assert_eq!(
        h.handle.pending_reveal(),
        Some(Pos(1)),
        "not fulfilled: no layout"
    );
}

/// A selection change owes its document an overlay pass, as a reveal
/// does: it changes no DOM, so a runtime that skips a clean frame would
/// never draw the new caret or highlight (#1001). A commit that leaves
/// the selection where it was owes nothing.
#[test]
fn a_selection_change_wakes_its_document_until_the_next_overlay_pass() {
    let h = five();
    let key = h.doc.borrow().doc_key();
    crate::registry::update_all_carets(Some(key), None);
    let here = h.handle.selection();
    h.handle.set_selection(here.clone());
    assert!(
        !crate::registry::overlay_pass_owed(key),
        "an unchanged selection owes nothing"
    );
    h.handle.set_selection(Selection::text(Pos(3), Pos(7)));
    assert_ne!(here, h.handle.selection(), "positive control");
    assert!(
        crate::registry::overlay_pass_owed(key),
        "owed after the selection moved"
    );
    crate::registry::update_all_carets(Some(key.wrapping_add(1)), None);
    assert!(
        crate::registry::overlay_pass_owed(key),
        "another document's pass leaves it"
    );
    crate::registry::update_all_carets(Some(key), None);
    assert!(
        !crate::registry::overlay_pass_owed(key),
        "the document's own pass clears it"
    );
}
