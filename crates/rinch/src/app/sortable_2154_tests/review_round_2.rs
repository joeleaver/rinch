//! The second review round: what a drag leaves behind, and co-edits mid-drag.

use super::*;

// ── Second review round.

/// Every node in the document's slab, connected or not.
fn slab_len(f: &Fixture) -> usize {
    f.app.doc.as_ref().unwrap().borrow().tree.nodes.len()
}

/// A preview's nodes go when its drag ends: after the first drag (which may
/// keep a wrapper for the next), cancelled drags leave the slab as it was.
#[test]
fn cancelled_drags_with_a_copy_leave_no_nodes_behind() {
    let mut f = fixture(Options::default());
    grab(&mut f, "a", 0.5, 0.5);
    key(&mut f, KeyCode::Escape, None);
    let base = slab_len(&f);
    for k in ["a", "b", "c", "d", "e", "f", "a", "b", "c", "d"] {
        grab(&mut f, k, 0.5, 0.5);
        move_to(&mut f, (500.0, 400.0));
        key(&mut f, KeyCode::Escape, None);
    }
    assert_eq!(
        slab_len(&f),
        base,
        "nodes left behind by 10 cancelled drags"
    );
}

/// Dropping costs what the app's re-render costs, the same with a copy as
/// with no preview.
#[test]
fn drops_with_a_copy_cost_no_more_nodes_than_drops_without() {
    let growth = |preview: SortPreview| {
        let mut f = fixture(Options {
            preview,
            ..Options::default()
        });
        // One drop first: whatever is made once is made.
        let drop_a_after_b = |f: &mut Fixture| {
            grab(f, "a", 0.5, 0.5);
            let over = point(f, item(f, "b"), 0.5, 0.8);
            move_to(f, over);
            release_at(f, over);
        };
        drop_a_after_b(&mut f);
        let base = slab_len(&f);
        for _ in 0..6 {
            drop_a_after_b(&mut f);
        }
        slab_len(&f) - base
    };
    assert_eq!(growth(SortPreview::Clone), growth(SortPreview::None));
}

/// A co-edit mid-drag (an item inserted into the source's list) keeps the
/// gap: the list is laid out again in the drop's order with the new item in it.
#[test]
fn the_gap_in_the_sources_list_survives_an_item_added_mid_drag() {
    let mut f = fixture(Options {
        indicator: SortIndicator::Gap,
        source: SortSource::Unchanged,
        preview: SortPreview::None,
    });
    grab(&mut f, "d", 0.5, 0.5);
    let over = point(&f, item(&f, "b"), 0.5, 0.2);
    move_to(&mut f, over);
    assert_eq!(
        (dy(&f, "a"), dy(&f, "b"), dy(&f, "c"), dy(&f, "d")),
        (0.0, STEP, STEP, -2.0 * STEP)
    );
    // A collaborator puts z first in c1: every card is laid out a step lower.
    f.board.update(|b| b[0].1.insert(0, "z".into()));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let over = point(&f, item(&f, "b"), 0.5, 0.2);
    move_to(&mut f, (over.0, over.1 + 0.5));
    move_to(&mut f, over);
    let target = sortable_drag().unwrap().target;
    assert_eq!(
        target,
        Some(SortTarget {
            list: "c1".into(),
            index: 2
        })
    );
    assert_eq!(
        (
            dy(&f, "z"),
            dy(&f, "a"),
            dy(&f, "b"),
            dy(&f, "c"),
            dy(&f, "d")
        ),
        (0.0, 0.0, STEP, STEP, -2.0 * STEP)
    );
    key(&mut f, KeyCode::Escape, None);
}

/// A place withdrawn by a leave is taken back only from the source drawn at
/// that place. Here the source was re-rendered at home (removed and added
/// again by a co-edit): the pointer on it there is over its own place, not
/// the withdrawn one, and whatever is on offer is what is drawn.
#[test]
fn a_rerendered_source_at_home_does_not_take_back_a_withdrawn_place() {
    let mut f = fixture(Options {
        indicator: SortIndicator::Gap,
        source: SortSource::Unchanged,
        preview: SortPreview::None,
    });
    grab(&mut f, "d", 0.5, 0.5);
    let over = point(&f, item(&f, "a"), 0.5, 0.2);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "d"), -3.0 * STEP, "d drawn first");
    // A co-edit takes d out and puts it back: a new node, at home.
    f.board.update(|b| b[0].1.retain(|k| k != "d"));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    f.board.update(|b| b[0].1.push("d".into()));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let home = point(&f, item(&f, "d"), 0.5, 0.5);
    for _ in 0..4 {
        move_to(&mut f, home);
        let state = sortable_drag().unwrap();
        let shifts: Vec<f32> = ["a", "b", "c", "d"].iter().map(|k| dy(&f, k)).collect();
        // What is on offer is what is drawn: no place and no shift, or the
        // place and its gap.
        match &state.target {
            None => assert!(shifts.iter().all(|s| *s == 0.0), "{state:?} {shifts:?}"),
            Some(t) => {
                assert_eq!(t.index, 0, "{state:?}");
                assert_eq!(shifts, [STEP, STEP, STEP, -3.0 * STEP], "{state:?}");
            }
        }
    }
    key(&mut f, KeyCode::Escape, None);
}

/// Document `doc`'s entries in the registry named `name` (#2797).
fn entries_in(doc: u64, name: &str) -> usize {
    rinch_core::doc_entries(doc)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map_or(0, |(_, count)| count)
}

/// A document's preview wrapper, kept between drags, and the focus request a
/// keyboard drop parks go with the document (#2797): an app dropped with
/// both leaves neither on the thread.
#[test]
fn a_dropped_apps_preview_wrapper_and_focus_request_are_forgotten() {
    const WRAPPERS: &str = "rinch_components::sortable::GHOST_WRAPPERS";
    const FOCUS: &str = "rinch_components::sortable::PENDING_FOCUS";
    let _guard = crate::app::timer_backend_lock();
    // No timer runs, so the focus request stays parked.
    rinch_core::set_timer_backend(|_, _| {});
    let mut f = fixture(Options {
        preview: SortPreview::Clone,
        ..Options::default()
    });
    let doc = f.app.doc_key();
    // A pointer drag with a copy keeps the document's wrapper.
    let at = grab(&mut f, "b", 0.5, 0.5);
    move_to(&mut f, (at.0, at.1 + 60.0));
    release_at(&mut f, (at.0, at.1 + 60.0));
    // A keyboard drop parks the focus of the item it moved.
    let at = point(&f, item(&f, "a"), 0.5, 0.5);
    press_at(&mut f, at);
    release_at(&mut f, at);
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowDown, None);
    key(&mut f, KeyCode::Space, Some(" "));
    assert_eq!(
        (entries_in(doc, WRAPPERS), entries_in(doc, FOCUS)),
        (1, 1),
        "control: the wrapper kept and the focus parked"
    );
    drop(f);
    assert_eq!(
        (entries_in(doc, WRAPPERS), entries_in(doc, FOCUS)),
        (0, 0),
        "the wrapper and the focus request outlived their document"
    );
}
