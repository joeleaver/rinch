//! #783 — a finished `@keyframes` animation without a forwards fill.
//!
//! css-animations-1: once the active duration ends, an animation whose
//! `animation-fill-mode` is `none` or `backwards` has no effect, so the animated
//! properties go back to their cascaded values. And the animation stays
//! finished: an unrelated restyle — a class, a pause, a theme change — does not
//! play it again. Only a change of `animation-name` (or the element being
//! rendered again after `display: none`, §3) starts a new one.
//!
//! `tick_animations` used to drop such an entry at completion and do nothing
//! else. Two things followed: the last interpolated sample stayed in
//! `computed_style` (and in the layout) until something re-cascaded the node,
//! and the next cascade, finding no entry of that name, minted the still-declared
//! animation again from t=0.
//!
//! Chrome 153 on the issue's markup (`.box { width: 10px }`,
//! `animation: grow 60ms linear 1`, `grow` = 40px → 100px): after the end the
//! box is **10px** and `getAnimations()` is `[]`; an unrelated class or
//! `animation-play-state: paused` added afterwards leaves it at 10px with
//! `getAnimations()` still `[]`.
//!
//! The fixtures sit off the fixed points: a 1600ms delay, two 4000ms iterations, a
//! sample a quarter of the way into the *second* iteration, and an end value
//! (100px) that differs from both the base (10px) and the start (40px). The
//! clock is moved by backdating each entry's `start_time_ms`, never by sleeping;
//! the durations are long so that a loaded host's wall-clock drift between
//! mount and sample (measured: up to ~45ms) moves a sample by under 1px.
//!
//! # Mutants, and what kills each
//!
//! | mutant | killed by |
//! |---|---|
//! | `main`: the completing tick drops the entry and restyles nothing | every `finished_*` fixture below except the two controls |
//! | the entry is kept as finished, but the completing tick does not restyle the node | `a_finished_animation_without_fill_returns_to_its_base_value`, `…_backwards_…`, `…_transform_…` |
//! | the completing tick restyles, but drops the entry (no finished marker) | `an_unrelated_restyle_does_not_replay_…`, `pausing_…`, `a_theme_change_…`, `…does_not_start_a_transition…` |
//! | a finished entry is latched (a restyle never runs it again) | `a_restyle_that_extends_a_finished_animation_runs_it_again`, alone |
//! | a finished entry is counted by `has_running_animations` | `a_finished_animation_without_fill_returns_to_its_base_value`, `an_unrelated_restyle_…` |
//! | the cascade's transition diff counts the sample it is restoring | `a_finished_animation_does_not_start_a_transition_on_its_way_back`, alone |

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    @keyframes k783-grow { from { width: 40px; } to { width: 100px; } }
    @keyframes k783-grow-b { from { width: 40px; } to { width: 100px; } }
    @keyframes k783-slide { from { transform: translateX(30px); } to { transform: translateX(10px); } }
    .box { width: 10px; height: 10px; }
    .none { animation: k783-grow 4000ms linear 1600ms 2 none; }
    .backwards { animation: k783-grow 4000ms linear 1600ms 2 backwards; }
    .forwards { animation: k783-grow 4000ms linear 1600ms 2 forwards; }
    .both { animation: k783-grow 4000ms linear 1600ms 2 both; }
    .slide { animation: k783-slide 4000ms linear 1600ms 2 none; }
    .renamed { animation-name: k783-grow-b; }
    .paused { animation-play-state: paused; }
    .more { animation-iteration-count: 1000000; }
    .tr { transition: width 4000ms linear; }
    .other { color: red; }
    .hidden { display: none; }
    .gen::before { content: \"x\"; display: inline-block; width: 10px; height: 10px;
                   animation: k783-grow 4000ms linear 1600ms 2 none; }
";

const VP: (f32, f32) = (800.0, 600.0);

/// The delay, one iteration, and the iteration count of every animation above.
const DELAY: f64 = 1600.0;
const DURATION: f64 = 4000.0;
/// A quarter of the way into the second iteration: 40 + 60 × 0.25 = 55px.
const DURING: f64 = DELAY + DURATION + 1000.0;
/// Past the end of both iterations.
const AFTER: f64 = DELAY + 2.0 * DURATION + 400.0;

fn width_px(doc: &RinchDocument, node: NodeId) -> f32 {
    match doc.tree.get(node.0).unwrap().computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => px,
        ref other => panic!("width is not a px length: {other:?}"),
    }
}

fn layout_width(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.width
}

fn names(doc: &RinchDocument, node: NodeId) -> Vec<String> {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|anims| anims.iter().map(|a| a.name.clone()).collect())
        .unwrap_or_default()
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

/// Move every animation on `node` back by `ms`: the clock reads `ms` later.
fn age(doc: &mut RinchDocument, node: NodeId, ms: f64) {
    for a in doc
        .tree
        .active_animations
        .get_mut(&node.0)
        .expect("an animation should be on the node")
    {
        a.start_time_ms -= ms;
    }
}

/// One frame as the shell runs it: tick, then resolve what the tick left.
fn frame(doc: &mut RinchDocument) -> bool {
    let running = doc.tick_animations();
    doc.resolve_layout(VP.0, VP.1);
    running
}

fn set_class(doc: &mut RinchDocument, node: NodeId, class: &str) {
    doc.set_attribute(node, "class", class);
    doc.resolve_layout(VP.0, VP.1);
}

/// Mount `class`, sample it a quarter into its second iteration, then run the
/// frame in which it finishes.
fn finished(class: &str) -> (RinchDocument, NodeId) {
    let (mut doc, node) = mount(class);
    age(&mut doc, node, DURING);
    assert!(frame(&mut doc), "precondition: it is running at {DURING}ms");
    assert!(
        (width_px(&doc, node) - 55.0).abs() < 3.0,
        "precondition: a quarter into the second iteration is 55px, got {}",
        width_px(&doc, node)
    );
    age(&mut doc, node, AFTER - DURING);
    frame(&mut doc);
    (doc, node)
}

/// The finished state every no-fill fixture expects: the base width, in style
/// and in layout, with nothing asking for frames.
fn assert_at_rest(doc: &mut RinchDocument, node: NodeId, what: &str) {
    assert_eq!(
        width_px(doc, node),
        10.0,
        "{what}: the box is back at its cascaded 10px, as in Chrome"
    );
    assert_eq!(layout_width(doc, node), 10.0, "{what}: and so is its layout");
    assert!(
        !doc.tree.has_running_animations(),
        "{what}: a finished animation has nothing to advance"
    );
    assert!(!frame(doc), "{what}: and asks the shell for no frame");
    assert_eq!(width_px(doc, node), 10.0, "{what}: a later tick moves nothing");
}

// ── half 1: the last sample is removed ──────────────────────────────────────

#[test]
fn a_finished_animation_without_fill_returns_to_its_base_value() {
    let (mut doc, node) = mount("box none");
    // In the delay, `none` has no effect: the base.
    age(&mut doc, node, DELAY / 2.0);
    assert!(frame(&mut doc));
    assert_eq!(width_px(&doc, node), 10.0, "in the delay, no fill: the base");

    age(&mut doc, node, DURING - DELAY / 2.0);
    assert!(frame(&mut doc));
    let during = width_px(&doc, node);
    assert!((during - 55.0).abs() < 3.0, "during: 55px, got {during}");
    assert!((layout_width(&doc, node) - during).abs() < 1.0);

    age(&mut doc, node, AFTER - DURING);
    frame(&mut doc);
    assert_at_rest(&mut doc, node, "fill none, after the end");
}

#[test]
fn a_finished_backwards_animation_returns_to_its_base_value() {
    let (mut doc, node) = mount("box backwards");
    age(&mut doc, node, DELAY / 2.0);
    assert!(frame(&mut doc));
    assert_eq!(
        width_px(&doc, node),
        40.0,
        "in the delay, `backwards` holds the first keyframe"
    );
    age(&mut doc, node, AFTER - DELAY / 2.0);
    frame(&mut doc);
    assert_at_rest(&mut doc, node, "fill backwards, after the end");
}

/// Controls: a forwards fill keeps the last keyframe, before and after the fix.
#[test]
fn a_finished_forwards_or_both_animation_keeps_its_end_value() {
    for class in ["box forwards", "box both"] {
        let (mut doc, node) = finished(class);
        assert_eq!(width_px(&doc, node), 100.0, "{class}: the fill holds 100px");
        set_class(&mut doc, node, &format!("{class} other"));
        assert_eq!(
            width_px(&doc, node),
            100.0,
            "{class}: and still does after an unrelated restyle"
        );
        assert!(!doc.tree.has_running_animations());
    }
}

/// The comment on the issue from PR #1409's review: a `transform` sample left
/// in place keeps the node a stacking context and an absolute containing block.
#[test]
fn a_finished_transform_animation_without_fill_stops_being_a_containing_block() {
    let (mut doc, node) = mount("box slide");
    age(&mut doc, node, DURING);
    assert!(frame(&mut doc));
    assert!(
        doc.tree.nodes[node.0].establishes_abs_containing_block(),
        "precondition: a running transform makes a containing block"
    );
    age(&mut doc, node, AFTER - DURING);
    frame(&mut doc);
    assert!(
        doc.tree.nodes[node.0].computed_style.transform.is_none,
        "the finished animation leaves the cascaded `transform: none`"
    );
    assert!(!doc.tree.nodes[node.0].establishes_abs_containing_block());
}

// ── half 2: it stays finished ───────────────────────────────────────────────

#[test]
fn an_unrelated_restyle_does_not_replay_a_finished_animation() {
    for class in ["box none", "box backwards"] {
        let (mut doc, node) = finished(class);
        set_class(&mut doc, node, &format!("{class} other"));
        assert_at_rest(&mut doc, node, &format!("{class}, then `.other`"));
    }
}

#[test]
fn pausing_a_finished_animation_does_not_replay_it() {
    let (mut doc, node) = finished("box none");
    set_class(&mut doc, node, "box none paused");
    // `main` re-minted it paused at t=0 — frozen at the delay's base, or at
    // 40px with a fill that covers the delay.
    assert_at_rest(&mut doc, node, "paused after the end");
    let (mut doc, node) = finished("box backwards");
    set_class(&mut doc, node, "box backwards paused");
    assert_at_rest(&mut doc, node, "backwards, paused after the end");
}

#[test]
fn a_theme_change_does_not_replay_a_finished_animation() {
    let (mut doc, node) = finished("box backwards");
    doc.update_theme_variables(":root { --unrelated: 1px; }");
    doc.recompute_all_styles_full();
    doc.resolve_layout(VP.0, VP.1);
    assert_at_rest(&mut doc, node, "after a full restyle");
}

/// A finished animation going back to its base value is not a style change a
/// transition starts from: css-transitions-1's before-change style is updated
/// to the current time, when the animation has no effect any more.
#[test]
fn a_finished_animation_does_not_start_a_transition_on_its_way_back() {
    let (mut doc, node) = finished("box tr none");
    assert!(
        !doc.tree.active_transitions.contains_key(&node.0),
        "no width transition from the last sample back to the base"
    );
    assert_at_rest(&mut doc, node, "with `transition: width` declared");
}

// ── what does start it again ────────────────────────────────────────────────

#[test]
fn changing_the_animation_name_starts_a_new_animation() {
    let (mut doc, node) = finished("box backwards");
    set_class(&mut doc, node, "box backwards renamed");
    assert_eq!(names(&doc, node), ["k783-grow-b"]);
    assert!(doc.tree.has_running_animations(), "the new name runs");
    assert_eq!(
        width_px(&doc, node),
        40.0,
        "from t=0: in its delay, `backwards` holds the first keyframe"
    );
}

/// css-animations-1 §3: an element rendered again after `display: none` starts
/// its animations afresh — the finished marker goes with the hide (#747).
#[test]
fn re_displaying_a_finished_animation_starts_it_again() {
    let (mut doc, node) = finished("box backwards");
    set_class(&mut doc, node, "box backwards hidden");
    assert!(names(&doc, node).is_empty(), "hidden: nothing is kept");
    set_class(&mut doc, node, "box backwards");
    assert_eq!(names(&doc, node), ["k783-grow"]);
    assert!(doc.tree.has_running_animations(), "shown again: it runs");
    assert_eq!(width_px(&doc, node), 40.0, "from t=0, in its delay");
}

/// The #782 rule, for the no-fill case: a restyle that gives a finished
/// animation more iterations leaves it running again, since its clock is now
/// inside the new active duration.
#[test]
fn a_restyle_that_extends_a_finished_animation_runs_it_again() {
    let (mut doc, node) = finished("box none");
    set_class(&mut doc, node, "box none more");
    assert!(doc.tree.has_running_animations(), "more iterations: running");
    assert!(frame(&mut doc), "and ticking");
    let w = width_px(&doc, node);
    assert!(
        (40.0..100.0).contains(&w),
        "the third iteration's sample, not the base: {w}"
    );
}

// ── a generated box ─────────────────────────────────────────────────────────

/// A `::before` box is styled by its originator's cascade (#1004, #1023), so the
/// restyle the finishing tick asks for has to be the originator's.
#[test]
fn a_finished_animation_on_a_before_box_returns_to_its_base_value() {
    let (mut doc, owner) = mount("gen");
    let pseudo = doc.tree.get(owner.0).unwrap().children[0];
    assert!(doc.tree.get(pseudo).unwrap().is_pseudo_element, "precondition");
    let pseudo = NodeId(pseudo);
    age(&mut doc, pseudo, DURING);
    assert!(frame(&mut doc));
    assert!((width_px(&doc, pseudo) - 55.0).abs() < 3.0, "during: 55px");
    age(&mut doc, pseudo, AFTER - DURING);
    frame(&mut doc);
    assert_eq!(
        doc.tree.get(owner.0).unwrap().children[0],
        pseudo.0,
        "the box is kept across its originator's cascade (#1023)"
    );
    assert_at_rest(&mut doc, pseudo, "::before, after the end");
    set_class(&mut doc, owner, "gen other");
    assert_at_rest(&mut doc, pseudo, "::before, then `.other` on its originator");
}
