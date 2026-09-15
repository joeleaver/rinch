//! #763 — `animation-play-state: paused` stops the animation's clock, and it
//! must stop the app's too.
//!
//! A paused `@keyframes` animation keeps its `ActiveAnimation` entry, and has
//! to: its frozen sample is written into `computed_style` on every cascade, and
//! that is what makes it *look* paused rather than un-animated
//! (`display_none_animation_tests::a_paused_animation_on_a_rendered_box_is_untouched`
//! pins that the entry survives a re-cascade). But `tick_animations` used to
//! count that entry as running — `is_complete` answers `false` for anything
//! paused — and re-apply its frozen sample every tick, marking the node dirty
//! as though it had moved. The desktop frame clock schedules another frame on
//! either, so a paused spinner rendered at full rate, forever, with nothing on
//! screen moving. The same symptom #747 fixed for a hidden element, by a
//! different route.
//!
//! This file pins the rinch-dom half: what `tick_animations` answers and what
//! it leaves dirty. `crates/rinch/src/app/paused_animation_frames_tests.rs`
//! pins the frame clock itself, both of its readers (the redraw request and
//! K23's "was there anything to tick").
//!
//! Every animation here runs for 1000s or longer, so nothing below can stop
//! asking for frames by expiring — every `false` is the pause, never a timeout.
//! Every resolve runs at one viewport size: a change of more than half a pixel
//! re-cascades the whole document, which is a second route to every answer
//! here (see `display_none_animation_tests::VP`).
//!
//! # Mutants, and what kills each
//!
//! Measured: each mutant applied to the committed source, `cargo test -p
//! rinch-dom -p rinch --no-fail-fast` run against it (85 result lines, the
//! unmutated control green), the source restored from the commit. "shell" names
//! fixtures in `crates/rinch/src/app/paused_animation_frames_tests.rs`.
//!
//! | mutant | killed by |
//! |---|---|
//! | `main`: a paused entry counts as running | 9: `a_paused_animation_asks_for_no_frames`, `a_paused_font_size_animation_does_not_invalidate_layout`, `resuming_a_paused_animation_asks_for_frames_again`, `a_resumed_animation_continues_from_where_it_was_paused`, `a_finished_transition_does_not_displace_a_paused_sample`, and 4 in the shell |
//! | a paused entry is **dropped** by the tick instead of kept quiet | 4: `a_paused_animation_asks_for_no_frames`, `two_animations_on_one_node_one_paused_still_ask_for_frames`, `a_resumed_animation_continues_from_where_it_was_paused`, shell `a_paused_animation_lets_the_app_go_idle` |
//! | the tick still re-applies a paused sample **and marks the node dirty** | 4: `a_paused_animation_leaves_the_tree_clean_after_a_tick` and 3 in the shell |
//! | the text-measure pre-pass is not narrowed to running animations | 2: `a_paused_font_size_animation_does_not_invalidate_layout`, shell `a_paused_animation_owes_the_android_loop_no_frame` |
//! | the tick answers `false` whenever anything is paused | 2: `a_running_animation_beside_a_paused_one_still_asks_for_frames`, `two_animations_on_one_node_one_paused_still_ask_for_frames` |
//! | a node's entries are judged by its **first** animation only | 2: `two_animations_on_one_node_one_paused_still_ask_for_frames`, `a_finished_transition_does_not_displace_a_paused_sample` |
//! | the tick **skips** a paused animation instead of re-applying its sample | `a_finished_transition_does_not_displace_a_paused_sample`, **alone** |
//! | resuming does not move the start time (the clock jumps the pause) | `a_resumed_animation_continues_from_where_it_was_paused`, **alone** |
//! | resuming restarts the animation from t=0 | `a_resumed_animation_continues_from_where_it_was_paused`, **alone** |

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `width` and `height` are the animated properties because they affect layout,
/// which is the dirtying a re-applied sample does that is easiest to see.
const CSS: &str = "
    @keyframes k763-grow { from { width: 40px; } to { width: 100px; } }
    @keyframes k763-tall { from { height: 10px; } to { height: 100px; } }
    @keyframes k763-type { from { font-size: 10px; } to { font-size: 30px; } }
    @keyframes k763-slide { from { width: 0px; } to { width: 10000px; } }

    .box  { width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
    .spin { animation: k763-grow 1000s linear infinite; }
    .held { animation: k763-grow 1000s linear infinite paused; }
    .both { animation: k763-grow 1000s linear infinite, k763-tall 1000s linear infinite;
            animation-play-state: paused, running; }
    .type-held { animation: k763-type 1000s linear infinite paused; }
    .slide { animation: k763-slide 10000ms linear infinite; }
    .slide--held { animation-play-state: paused; }
    .eased { transition: width 60ms linear; }
    .narrow { width: 20px; }
";

const VP: (f32, f32) = (800.0, 600.0);

fn animations(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|v| v.len())
        .unwrap_or(0)
}

/// The node's computed `width` in px, or `None` when it is not a length.
fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

/// A document with one `div.box` per class list, laid out once with the clock
/// armed. Returns the boxes in the order given.
fn mount(classes: &[&str]) -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let boxes = classes
        .iter()
        .map(|class| {
            let node = doc.create_element("div");
            doc.set_attribute(node, "class", class);
            let text = doc.create_text("text");
            doc.append_child(node, text);
            doc.append_child(body, node);
            node
        })
        .collect();
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    (doc, boxes)
}

/// The control every `false` and every "clean" below rests on: a running
/// animation asks for frames, and its tick dirties the node it moved. Kills
/// nothing on its own and passes on `main`.
#[test]
fn a_running_animation_asks_for_frames() {
    let (mut doc, boxes) = mount(&["box spin"]);
    assert_eq!(animations(&doc, boxes[0]), 1, "precondition: registered");
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    doc.tree.get_mut(boxes[0].0).unwrap().dirty = rinch_dom::DirtyFlags::empty();

    assert!(doc.tick_animations(), "a moving clock needs another frame");
    assert!(
        doc.tree.dirty_nodes.contains(&boxes[0].0) && !doc.tree.paint_dirty_nodes.is_empty(),
        "and a running `width` animation dirties its node on a tick — without \
         this, `a_paused_animation_leaves_the_tree_clean_after_a_tick` would \
         also pass against a tick that never dirties anything"
    );
    assert!(
        doc.tree
            .get(boxes[0].0)
            .unwrap()
            .dirty
            .contains(rinch_dom::DirtyFlags::LAYOUT),
        "and flags the node itself"
    );
}

/// The issue's probe, verbatim: `.held { animation: … infinite paused }` —
/// after resolve there is one entry, and the tick has nothing to advance.
///
/// The entry and its frozen sample must both survive the tick: the fix is to
/// stop *counting* a paused animation, not to stop *having* one.
#[test]
fn a_paused_animation_asks_for_no_frames() {
    let (mut doc, boxes) = mount(&["box held"]);
    let held = boxes[0];
    assert_eq!(animations(&doc, held), 1, "precondition: registered");
    assert_eq!(
        width_px(&doc, held),
        Some(40.0),
        "precondition: the cascade wrote the frozen sample (t=0 of 40px→100px), \
         which is off the box's own 10px"
    );

    for round in 0..3 {
        assert!(
            !doc.tick_animations(),
            "round {round}: a paused animation has nothing to advance, so it \
             must not ask the shell for another frame"
        );
    }

    assert_eq!(
        animations(&doc, held),
        1,
        "the entry stays — it is what keeps the box looking paused"
    );
    assert_eq!(
        doc.tree.active_animations[&held.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Paused,
        "and it is still the paused one"
    );
    assert_eq!(width_px(&doc, held), Some(40.0), "still showing its sample");
}

/// A paused sample is already in `computed_style` — the cascade put it there —
/// so a tick that re-applies it has changed nothing and must not say it did.
///
/// `width` affects layout, so a tick that marked the node dirty would put it in
/// `dirty_nodes`, which the desktop frame clock resolves and repaints: a
/// redraw per frame by the second route, even with the tick answering `false`.
#[test]
fn a_paused_animation_leaves_the_tree_clean_after_a_tick() {
    let (mut doc, boxes) = mount(&["box held"]);
    let held = boxes[0];
    // What the shell does after a resolve: it takes the mutated-node set, and a
    // paint drains the repaint queue.
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    assert!(
        !doc.tree.layout_dirty,
        "precondition: the resolve left nothing to do"
    );
    // A resolve does not clear a node's own flags; clear them here so that
    // anything the tick sets is visible.
    doc.tree.get_mut(held.0).unwrap().dirty = rinch_dom::DirtyFlags::empty();

    doc.tick_animations();

    assert!(
        doc.tree.dirty_nodes.is_empty(),
        "a paused `width` animation dirtied layout on a tick: {:?}",
        doc.tree.dirty_nodes
    );
    assert!(!doc.tree.layout_dirty, "nor may it set `layout_dirty`");
    assert!(
        doc.tree.paint_dirty_nodes.is_empty(),
        "nor queue a repaint of a box that did not change: {:?}",
        doc.tree.paint_dirty_nodes
    );
    assert_eq!(
        doc.tree.get(held.0).unwrap().dirty,
        rinch_dom::DirtyFlags::empty(),
        "nor flag the node itself"
    );
}

/// `font-size` is a text-measure property, and `RinchDocument::tick_animations`
/// invalidates the text measure of every node animating one *before* the tick
/// (#678) — setting `layout_dirty`, which the desktop wake and the Android loop
/// both read as "a frame is owed". A frozen font size re-wraps nothing.
#[test]
fn a_paused_font_size_animation_does_not_invalidate_layout() {
    let (mut doc, boxes) = mount(&["box type-held"]);
    assert_eq!(animations(&doc, boxes[0]), 1, "precondition: registered");
    assert!(!doc.tree.layout_dirty, "precondition: clean");

    assert!(!doc.tick_animations(), "nothing to advance");
    assert!(
        !doc.tree.layout_dirty,
        "a paused `font-size` animation re-measured text on a tick"
    );
}

/// Pausing one animation must not silence another.
#[test]
fn a_running_animation_beside_a_paused_one_still_asks_for_frames() {
    let (mut doc, boxes) = mount(&["box held", "box spin"]);
    assert_eq!(
        animations(&doc, boxes[0]),
        1,
        "precondition: held registered"
    );
    assert_eq!(
        animations(&doc, boxes[1]),
        1,
        "precondition: spin registered"
    );
    assert!(
        doc.tick_animations(),
        "one paused box does not make the whole document still"
    );
    assert!(doc.tick_animations(), "on every tick, not just the first");
}

/// Two animations on **one** node, the paused one listed first — so a tick that
/// judged a node by its first entry, or by "any paused", goes quiet here.
#[test]
fn two_animations_on_one_node_one_paused_still_ask_for_frames() {
    let (mut doc, boxes) = mount(&["box both"]);
    let both = boxes[0];
    assert_eq!(animations(&doc, both), 2, "precondition: both registered");
    assert_eq!(
        doc.tree.active_animations[&both.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Paused,
        "precondition: the paused one is first"
    );
    assert!(
        doc.tick_animations(),
        "the running `height` animation on the same node still needs frames"
    );
    assert_eq!(animations(&doc, both), 2, "and neither entry was dropped");
}

/// Resuming schedules frames again. The half of the fix that is easy to lose:
/// a tick that went quiet for a paused entry must come back once it runs.
#[test]
fn resuming_a_paused_animation_asks_for_frames_again() {
    let (mut doc, boxes) = mount(&["box held"]);
    let node = boxes[0];
    assert!(!doc.tick_animations(), "precondition: paused and quiet");

    doc.set_attribute(node, "class", "box spin");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, node), 1, "the same entry, resumed");
    assert_eq!(
        doc.tree.active_animations[&node.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Running,
        "and it is running now"
    );
    assert!(doc.tick_animations(), "so the frames come back");
}

/// A paused animation resumed later continues from its frozen time — it
/// neither jumps the pause nor starts over.
///
/// `k763-slide` is 0px→10000px over 10000ms, linear, so its `width` in px *is*
/// its elapsed time in ms. The animation runs for at least 80ms before it is
/// paused, which keeps the frozen sample off zero — at zero, "resume from where
/// it was" and "restart from t=0" are the same answer.
#[test]
fn a_resumed_animation_continues_from_where_it_was_paused() {
    let (mut doc, boxes) = mount(&["box slide"]);
    let node = boxes[0];
    std::thread::sleep(std::time::Duration::from_millis(80));

    doc.set_attribute(node, "class", "box slide slide--held");
    doc.resolve_layout(VP.0, VP.1);
    let frozen = width_px(&doc, node).expect("a length");
    assert!(
        frozen >= 80.0,
        "precondition: paused after at least 80ms of running, read {frozen}"
    );

    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(!doc.tick_animations(), "paused, so nothing to advance");
    assert_eq!(
        width_px(&doc, node),
        Some(frozen),
        "and 150ms later it has not moved"
    );

    doc.set_attribute(node, "class", "box slide");
    doc.resolve_layout(VP.0, VP.1);
    assert!(doc.tick_animations(), "running again");
    let resumed = width_px(&doc, node).expect("a length");

    assert!(
        resumed >= frozen,
        "resumed at {resumed}px, before the {frozen}px it was paused at — \
         that is a restart, not a resume"
    );
    assert!(
        resumed < frozen + 100.0,
        "resumed at {resumed}px against a pause at {frozen}px: the 150ms spent \
         paused was counted as running time"
    );
}

/// The tick still re-applies a paused sample — it only stops *counting* it and
/// *dirtying* for it.
///
/// Skipping a paused entry outright would look equivalent, since the cascade
/// already wrote the sample, and it is not: a transition on the same property
/// writes `computed_style` from its own tick, and the one that **finishes**
/// writes its end value there. The animation tick runs after it, and a paused
/// animation re-applying its sample is what puts the value back to what the
/// cascade says it is. Skipped, the box would sit at the transition's end value
/// until something unrelated re-cascaded it.
#[test]
fn a_finished_transition_does_not_displace_a_paused_sample() {
    let (mut doc, boxes) = mount(&["box eased held"]);
    let node = boxes[0];
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "precondition: frozen at 40px"
    );

    // The base width changes under the paused animation, so a `width`
    // transition starts; the cascade keeps the frozen sample on top.
    doc.set_attribute(node, "class", "box eased held narrow");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        doc.tree.active_transitions.get(&node.0).map(|t| t.len()),
        Some(1),
        "precondition: a `width` transition is running"
    );
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "precondition: and the cascade still shows the paused sample"
    );

    std::thread::sleep(std::time::Duration::from_millis(120));
    doc.tick_transitions();
    assert_eq!(
        width_px(&doc, node),
        Some(20.0),
        "precondition: the finishing transition tick wrote its end value — \
         so what follows is the animation tick's doing, not a fixed point"
    );

    assert!(!doc.tick_animations(), "paused, so no frame");
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "a paused animation's tick put its sample back over the transition's \
         end value, as the cascade has it"
    );
}
