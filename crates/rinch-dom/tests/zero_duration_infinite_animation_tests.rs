//! #786 — `animation: … 0s infinite` is finished, not running forever.
//!
//! The active duration of an animation is its duration times its iteration
//! count, and `0 × ∞` is `0` (Web Animations §4.6). So a zero-duration infinite
//! animation finishes the moment its delay ends, and fills — or not — like any
//! finished animation. `ActiveAnimation::is_complete` answered `false` for every
//! `IterationCount::Infinite`, so such an entry with `forwards`/`both` was never
//! complete, never filling, and `tick_animations` took its running branch on
//! every tick: re-applied the same sample, dirtied the node (`LAYOUT` for
//! `width`) and counted it, so the frame clock never idled.
//!
//! Every sampled value here is Chrome 155's, measured on the same `@keyframes`
//! (`from 40px to 100px`, the box itself 10px): `infinite forwards` and `both`
//! are `finished@0` at 100px; `reverse` and `alternate-reverse` 40px; `none` and
//! `backwards` have no effect (10px); `200ms both` is `running` at 40px in its
//! delay and `finished` at 100px after it; `-200ms forwards` is finished at
//! 100px from the start.
//!
//! # Mutants, and what kills each
//!
//! Measured over this file, `filled_animation_frame_tests` and
//! `paused_animation_frame_tests` (`--no-fail-fast`).
//!
//! | mutant | killed by |
//! |---|---|
//! | `main`: `is_complete` is `false` for every infinite animation | 5: every fixture here that settles |
//! | a zero-duration infinite animation is complete at once, delay ignored | `a_positive_delay_runs_until_it_ends_then_settles`, **alone** |
//! | every infinite animation has an active duration of 0 | `a_non_zero_infinite_animation_still_runs`, and 5 in `paused_animation_frame_tests` |

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    @keyframes k786 { from { width: 40px; } to { width: 100px; } }
    .box { width: 10px; height: 10px; }
    .fwd { animation: k786 0s linear infinite forwards; }
    .both { animation: k786 0s linear infinite both; }
    .none { animation: k786 0s linear infinite none; }
    .rev { animation: k786 0s linear infinite reverse forwards; }
    .altrev { animation: k786 0s linear infinite alternate-reverse forwards; }
    .delayed { animation: k786 0s linear infinite 200ms both; }
    .negdelay { animation: k786 0s linear infinite -200ms forwards; }
    .slow { animation: k786 50ms linear infinite forwards; }
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

/// What the shell does between frames (see `filled_animation_frame_tests.rs`).
fn hand_off(doc: &mut RinchDocument, node: NodeId) {
    doc.resolve_layout(VP.0, VP.1);
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    doc.tree.get_mut(node.0).unwrap().dirty = rinch_dom::DirtyFlags::empty();
}

/// After at most one tick (the one that writes the fill), the animation asks
/// for no frames and dirties nothing, and the box shows `expected`.
fn assert_settles(class: &str, expected: f32) {
    let (mut doc, node) = mount(class);
    hand_off(&mut doc, node);
    // The tick that settles it: it may dirty the node (it writes the fill), but
    // it does not ask for another frame.
    assert!(
        !doc.tick_animations(),
        "{class}: `0s infinite` has an active duration of 0, so it is finished, \
         not running"
    );
    assert_eq!(width_px(&doc, node), Some(expected), "{class}: the fill");

    for round in 0..3 {
        hand_off(&mut doc, node);
        assert!(
            !doc.tree.has_running_animations(),
            "{class} round {round}: nothing left to advance"
        );
        assert!(
            !doc.tick_animations(),
            "{class} round {round}: so no frame is asked for"
        );
        assert!(
            doc.tree.dirty_nodes.is_empty() && doc.tree.paint_dirty_nodes.is_empty(),
            "{class} round {round}: and the box is not dirtied again"
        );
        assert!(
            !doc.tree.layout_dirty,
            "{class} round {round}: no layout owed"
        );
        assert_eq!(
            width_px(&doc, node),
            Some(expected),
            "{class} round {round}: still filled"
        );
    }
    assert_eq!(animations(&doc, node), 1, "{class}: the entry is the fill");
}

#[test]
fn a_zero_duration_infinite_forwards_animation_settles() {
    assert_settles("box fwd", 100.0);
}

#[test]
fn a_zero_duration_infinite_both_animation_settles() {
    assert_settles("box both", 100.0);
}

/// The direction picks the end sample: Chrome 155 fills `reverse` and
/// `alternate-reverse` at the `from` value.
#[test]
fn reverse_directions_fill_at_the_from_value() {
    assert_settles("box rev", 40.0);
    assert_settles("box altrev", 40.0);
}

/// A negative delay changes nothing: it is finished from the start.
#[test]
fn a_negative_delay_settles_at_once() {
    assert_settles("box negdelay", 100.0);
}

/// `none` has no effect once finished: the entry stays as a marker (#783), the box keeps its own
/// width, and nothing is asked for.
#[test]
fn a_zero_duration_infinite_animation_without_fill_has_no_effect() {
    let (mut doc, node) = mount("box none");
    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "no frame");
    assert!(!doc.tree.has_running_animations());
    assert_eq!(width_px(&doc, node), Some(10.0), "the box's own width");
    // Kept as a settled marker, not dropped (#783): a later restyle of the
    // box must find it by name rather than start it again.
    assert_eq!(
        doc.tree.finished_animations[&node.0].len(),
        1,
        "the entry is kept, finished"
    );
    assert!(
        doc.tree.finished_animations[&node.0][0].settled,
        "and settled, so nothing counts it"
    );
    assert_eq!(animations(&doc, node), 0, "and no tick walks it");
}

/// A positive delay keeps it running — it has a delay to wait out, with the
/// `backwards` half of `both` showing the `from` value — and it settles once
/// the delay ends. Off the fixed point: a mutant that calls every zero-duration
/// animation complete at once fails the first half.
#[test]
fn a_positive_delay_runs_until_it_ends_then_settles() {
    let (mut doc, node) = mount("box delayed");
    hand_off(&mut doc, node);
    assert!(
        doc.tree.has_running_animations(),
        "inside the 200ms delay it is still waiting"
    );
    assert!(doc.tick_animations(), "so it asks for frames");
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "backwards fill in the delay"
    );

    std::thread::sleep(std::time::Duration::from_millis(300));
    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "after the delay it is finished");
    assert!(
        doc.tree.dirty_nodes.contains(&node.0),
        "and the tick that finishes it shows the fill, so it dirties the box"
    );
    assert_eq!(width_px(&doc, node), Some(100.0), "the forwards fill");

    hand_off(&mut doc, node);
    assert!(!doc.tick_animations(), "then quiet");
    assert!(doc.tree.dirty_nodes.is_empty(), "and nothing dirtied");
    assert!(!doc.tree.has_running_animations());
}

/// A positive-duration infinite animation is still running forever.
#[test]
fn a_non_zero_infinite_animation_still_runs() {
    let (mut doc, node) = mount("box slow");
    std::thread::sleep(std::time::Duration::from_millis(120));
    hand_off(&mut doc, node);
    assert!(doc.tree.has_running_animations());
    assert!(
        doc.tick_animations(),
        "an infinite 50ms animation never ends"
    );
    assert!(
        doc.tree.dirty_nodes.contains(&node.0),
        "and each tick moves it"
    );
}
