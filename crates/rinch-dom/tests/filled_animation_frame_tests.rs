//! #782 — a finished `forwards`/`both` animation stops asking for frames.
//!
//! An animation that has run all its iterations and fills keeps its
//! `ActiveAnimation` entry, and has to: the fill *is* its effect, and the
//! cascade re-applies it on every restyle. But `tick_animations` kept counting
//! such an entry as active and re-applying its sample with the node marked
//! dirty, every tick, forever — the frame-clock spin #763 fixed for a paused
//! animation, reached through a finished one. A filling animation's sample is
//! as constant as a paused one's.
//!
//! So the tick that *finishes* it writes the fill and marks the node dirty —
//! that frame changes what the box looks like — and records the entry as
//! settled. Later ticks re-apply the sample quietly and do not count it, and
//! `NodeTree::has_running_animations` does not count it either.
//!
//! `crates/rinch/src/app/paused_animation_frames_tests.rs` pins the frame clock
//! itself for this shape too.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    @keyframes k782-grow { from { width: 40px; } to { width: 100px; } }
    .box { width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
    .once-fwd { animation: k782-grow 50ms linear 1 forwards; }
    .once-both { animation: k782-grow 50ms linear 1 both; }
    @keyframes k782-type { from { font-size: 20px; } to { font-size: 30px; } }
    .once-type { animation: k782-type 50ms linear 1 forwards; }
    .again { animation-iteration-count: 1000000; }
    .other { color: red; }
";

const VP: (f32, f32) = (800.0, 600.0);

fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

fn animations(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|v| v.len())
        .unwrap_or(0)
}

fn mount(class: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "class", class);
    doc.append_child(body, node);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    (doc, node)
}

/// What the shell does between frames: resolve what the last one left, take the
/// mutated-node set, drain the repaint queue, and (a paint does not, but this
/// makes a flag visible) clear the node's own flags.
fn hand_off(doc: &mut RinchDocument, node: NodeId) {
    doc.resolve_layout(VP.0, VP.1);
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    doc.tree.get_mut(node.0).unwrap().dirty = rinch_dom::DirtyFlags::empty();
}

/// Mount the animation, let it finish, and run the tick that finishes it.
fn finished(class: &str) -> (RinchDocument, NodeId) {
    let (mut doc, node) = mount(class);
    assert!(
        doc.tree.has_running_animations(),
        "precondition: it is running before it finishes"
    );
    std::thread::sleep(std::time::Duration::from_millis(150));
    hand_off(&mut doc, node);

    doc.tick_animations();
    assert_eq!(
        width_px(&doc, node),
        Some(100.0),
        "precondition: the finishing tick wrote the `forwards` value"
    );
    assert!(
        doc.tree.dirty_nodes.contains(&node.0),
        "the finishing tick changed the box, so it has to say so — this is the \
         frame that shows the animation's end"
    );
    (doc, node)
}

#[test]
fn a_finished_forwards_animation_asks_for_no_frames() {
    let (mut doc, node) = finished("box once-fwd");

    for round in 0..3 {
        hand_off(&mut doc, node);
        assert!(
            !doc.tree.has_running_animations(),
            "round {round}: a filling animation has nothing left to advance"
        );
        assert!(
            !doc.tick_animations(),
            "round {round}: so it must not ask the shell for another frame"
        );
        assert!(
            doc.tree.dirty_nodes.is_empty() && doc.tree.paint_dirty_nodes.is_empty(),
            "round {round}: nor dirty the box it has already painted"
        );
        assert!(!doc.tree.layout_dirty, "round {round}: nor owe a layout");
        assert_eq!(
            doc.tree.get(node.0).unwrap().dirty,
            rinch_dom::DirtyFlags::empty(),
            "round {round}: nor flag the node"
        );
    }

    assert_eq!(
        animations(&doc, node),
        1,
        "the entry stays — it is the fill"
    );
    assert_eq!(width_px(&doc, node), Some(100.0), "and the box shows it");
}

/// `both` fills forwards too, and is the same shape.
#[test]
fn a_finished_both_animation_asks_for_no_frames() {
    let (mut doc, node) = finished("box once-both");
    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "no frame");
    assert!(doc.tree.dirty_nodes.is_empty(), "and nothing dirtied");
    assert_eq!(width_px(&doc, node), Some(100.0));
}

/// A restyle of the node still shows the filled value, and does not start the
/// clock again.
#[test]
fn a_restyle_keeps_showing_the_fill() {
    let (mut doc, node) = finished("box once-fwd");
    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "precondition: settled");

    doc.set_attribute(node, "class", "box once-fwd other");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        width_px(&doc, node),
        Some(100.0),
        "the cascade re-applied the fill over the box's own 10px"
    );
    assert!(
        !doc.tree.has_running_animations(),
        "an unrelated restyle does not un-finish it"
    );
    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "no frame after the restyle either");
    assert!(doc.tree.dirty_nodes.is_empty(), "and nothing dirtied");
    assert_eq!(width_px(&doc, node), Some(100.0), "still filled");
}

/// A restyle that gives a finished animation more iterations makes it run
/// again — so being settled is a fact about the animation's timing now, not a
/// latch set once.
#[test]
fn a_restyle_that_extends_a_finished_animation_runs_it_again() {
    let (mut doc, node) = finished("box once-fwd");
    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "precondition: settled");

    doc.set_attribute(node, "class", "box once-fwd again");
    doc.resolve_layout(VP.0, VP.1);
    assert!(
        doc.tree.has_running_animations(),
        "a million iterations of 50ms is running, not filling"
    );
    assert!(doc.tick_animations(), "so the clock runs again");
}

/// A filled **typography** animation. The tick's text-measure pre-pass
/// re-measures every animation it can still move; a settled fill is not one of
/// them, or it would set `layout_dirty` on every tick — a frame owed, forever,
/// by a route the redraw request never sees.
#[test]
fn a_finished_font_size_fill_owes_no_layout() {
    let (mut doc, node) = mount("box once-type");
    std::thread::sleep(std::time::Duration::from_millis(150));
    hand_off(&mut doc, node);
    doc.tick_animations();
    assert_eq!(
        doc.tree.get(node.0).unwrap().computed_style.font_size,
        30.0,
        "precondition: the finishing tick wrote the 30px fill"
    );

    for round in 0..3 {
        hand_off(&mut doc, node);
        assert!(!doc.tick_animations(), "round {round}: no frame");
        assert!(
            !doc.tree.layout_dirty,
            "round {round}: and no re-measure of a font size that cannot change"
        );
    }
}
