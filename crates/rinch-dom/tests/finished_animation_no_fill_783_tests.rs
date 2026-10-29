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
//! Each applied to the committed source, then this file,
//! `review2_1409_tests` and `filled_animation_frame_tests` run (#782's pins
//! survive all eight).
//!
//! | mutant | killed by |
//! |---|---|
//! | M1 `main`: the completing tick drops the entry and restyles nothing | 9 here, and `review2_1409_tests::b4_animation_ending_without_fill` |
//! | M2 the entry is kept as finished, but the tick asks for no restyle | the five "returns to its base" / containing-block / transition fixtures, and `b4` |
//! | M3 the tick asks for the restyle, but drops the entry (no marker) | 8 here (every replay fixture, and the base-value ones: the replay re-mints it), and `b4` |
//! | M4 a settled entry is latched (a restyle never clears it) | `a_restyle_that_extends_a_finished_animation_runs_it_again`, alone |
//! | M5 `settled` is kept only while *filling* (#782's old rule) | 6 here: it never stays settled, so `has_running_animations` counts it |
//! | M6 `has_running_animations` counts a settled no-fill entry | the same 6 |
//! | M7 the cascade's transition diff keeps the way back | `a_finished_animation_does_not_start_a_transition_on_its_way_back`, alone |
//! | M8 a generated box's own node is marked, not its originator | `a_finished_animation_on_a_before_box_returns_to_its_base_value`, alone |
//!
//! Red at 49586231 (before the fix): 8 of the first 11 fixtures; the
//! forwards/both control, the rename and the re-display fixtures pass there.
//!
//! # The review of #2000
//!
//! A finished marker lives in `NodeTree::finished_animations`, which no tick
//! walks (F1), so it also drops out of the cascade's `animated_text_measure`
//! (F2); a pause no longer clears `settled` (F4). The fixtures in the last
//! section pin that; the mutants, applied to the committed source with the
//! anchors asserted, and run against this file, `perf_counter_baselines`,
//! `filled_animation_frame_tests` and `zero_duration_infinite_animation_tests`:
//!
//! | mutant | killed by |
//! |---|---|
//! | MF1 markers kept in `active_animations` (the PR's first shape) | 7 here, `perf_counter_baselines::transform_animation_tick_beside_finished_one_shots`, the zero-duration pin |
//! | MC `ended_animations` never cleared | `a_later_real_change_after_the_end_still_transitions`, `the_frames_after_the_end_restyle_nothing`, `resuming_…`, the baseline |
//! | MD every tick restyles every finished node | `the_frames_after_the_end_restyle_nothing`, `resuming_…`, the baseline |
//! | MF4 `settled` recomputed with `is_complete` (cleared by a pause) | `resuming_a_paused_finished_animation_costs_no_frame`, alone |
//! | MF2 `animated_text_measure` counts finished markers | `a_colour_restyle_after_a_finished_font_size_animation_owes_no_layout`, alone |
//! | MG the cascade's animation gate ignores `finished_animations` | `removing_and_redeclaring_a_finished_animation_starts_it_again`, alone |
//!
//! F3 is a known gap, pinned as it stands by
//! `an_end_and_a_real_change_in_one_frame_start_no_transition`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

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
    .wide { width: 20px; }
    @keyframes k783-spin { from { opacity: 0.2; } to { opacity: 0.9; } }
    .spin { animation: k783-spin 1000s linear infinite; }
    .entrance { animation: k783-grow 200ms linear 1; }
    @keyframes k783-size { from { font-size: 30px; } to { font-size: 20px; } }
    .text { font-size: 12px; line-height: 16px; }
    .sized { animation: k783-size 200ms linear 1; }
    .red { color: rgb(200, 0, 0); }
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

/// The animations on `node`, running or finished.
fn names(doc: &RinchDocument, node: NodeId) -> Vec<String> {
    let running = doc.tree.active_animations.get(&node.0);
    let finished = doc.tree.finished_animations.get(&node.0);
    running
        .into_iter()
        .chain(finished)
        .flatten()
        .map(|a| a.name.clone())
        .collect()
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
    assert_eq!(
        layout_width(doc, node),
        10.0,
        "{what}: and so is its layout"
    );
    assert!(
        !doc.tree.has_running_animations(),
        "{what}: a finished animation has nothing to advance"
    );
    assert!(!frame(doc), "{what}: and asks the shell for no frame");
    assert_eq!(
        width_px(doc, node),
        10.0,
        "{what}: a later tick moves nothing"
    );
}

// ── half 1: the last sample is removed ──────────────────────────────────────

#[test]
fn a_finished_animation_without_fill_returns_to_its_base_value() {
    let (mut doc, node) = mount("box none");
    // In the delay, `none` has no effect: the base.
    age(&mut doc, node, DELAY / 2.0);
    assert!(frame(&mut doc));
    assert_eq!(
        width_px(&doc, node),
        10.0,
        "in the delay, no fill: the base"
    );

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

/// Taking the animation away drops its finished marker, so declaring it again
/// starts a new one — even though the node then declares no animation and the
/// marker is in `finished_animations`, which the cascade has to look at too.
#[test]
fn removing_and_redeclaring_a_finished_animation_starts_it_again() {
    let (mut doc, node) = finished("box backwards");
    set_class(&mut doc, node, "box");
    assert!(
        names(&doc, node).is_empty(),
        "no animation declared: none kept"
    );
    set_class(&mut doc, node, "box backwards");
    assert!(doc.tree.has_running_animations(), "declared again: it runs");
    assert_eq!(width_px(&doc, node), 40.0, "from t=0, in its delay");
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
    assert!(
        doc.tree.has_running_animations(),
        "more iterations: running"
    );
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
    assert!(
        doc.tree.get(pseudo).unwrap().is_pseudo_element,
        "precondition"
    );
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
    assert_at_rest(
        &mut doc,
        pseudo,
        "::before, then `.other` on its originator",
    );
}

// ── review of #2000: what a finished marker costs, and the edges ────────────

/// F1: a finished one-shot leaves a marker for the node's life, and a list of
/// rows with an entrance animation leaves one per row. No tick may walk them:
/// beside a running spinner, the tick sees the spinner alone.
#[test]
fn finished_one_shots_are_not_walked_by_the_tick() {
    const ROWS: usize = 50;
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let rows: Vec<NodeId> = (0..ROWS)
        .map(|_| {
            let r = doc.create_element("div");
            doc.set_attribute(r, "class", "box entrance");
            doc.append_child(body, r);
            r
        })
        .collect();
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "box spin");
    doc.append_child(body, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    for r in &rows {
        age(&mut doc, *r, 1000.0);
    }
    assert!(frame(&mut doc), "the spinner runs");
    for r in &rows {
        assert_eq!(width_px(&doc, *r), 10.0, "each row is back at its base");
        assert_eq!(names(&doc, *r), ["k783-grow"], "and keeps its marker");
    }
    assert_eq!(doc.tree.finished_animations.len(), ROWS);
    assert_eq!(
        doc.tree
            .active_animations
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [spinner.0],
        "the tick's map holds the spinner alone"
    );
    // An unrelated restyle of a row keeps it finished and off the tick's map.
    set_class(&mut doc, rows[7], "box entrance other");
    assert_eq!(doc.tree.active_animations.len(), 1);
    assert_eq!(width_px(&doc, rows[7]), 10.0);
}

/// The review's mutant MD: the finishing tick restyles the node once, and the
/// idle frames after it restyle nothing, while a spinner keeps them coming.
#[test]
fn the_frames_after_the_end_restyle_nothing() {
    let (mut doc, node) = mount("box none");
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "box spin");
    let body = doc.body();
    doc.append_child(body, spinner);
    doc.resolve_layout(VP.0, VP.1);
    age(&mut doc, node, DURING);
    assert!(frame(&mut doc));
    age(&mut doc, node, AFTER - DURING);
    doc.tree.perf.reset();
    assert!(frame(&mut doc), "the spinner keeps frames coming");
    assert_eq!(
        doc.tree.perf.end_frame().get(Counter::ElementsCascaded),
        1,
        "positive control: the frame that ends it restyles its node"
    );
    assert_eq!(width_px(&doc, node), 10.0);
    for i in 0..4 {
        assert!(frame(&mut doc));
        assert_eq!(
            doc.tree.perf.end_frame().get(Counter::ElementsCascaded),
            0,
            "idle frame {i} after the end restyles nothing"
        );
    }
}

/// The review's mutant MC (`ended_animations` never cleared): the stale entry
/// would drop a later, real `width` change from the transition diff.
#[test]
fn a_later_real_change_after_the_end_still_transitions() {
    let (mut doc, node) = finished("box tr none");
    assert_at_rest(&mut doc, node, "precondition");
    set_class(&mut doc, node, "box tr none wide");
    assert!(
        doc.tree.active_transitions.contains_key(&node.0),
        "10px → 20px under `transition: width` starts a transition"
    );
}

/// Known gap (F3), pinned so a fix shows up here: the end and a real change of
/// the same property land in one frame. css-transitions-1 would start a 10px →
/// 20px transition from the previous cascaded value; rinch keeps no copy of
/// it, drops the `width` diff with the way back, and the box jumps to 20px.
#[test]
fn an_end_and_a_real_change_in_one_frame_start_no_transition() {
    let (mut doc, node) = mount("box tr none");
    age(&mut doc, node, DURING);
    assert!(frame(&mut doc));
    age(&mut doc, node, AFTER - DURING);
    doc.set_attribute(node, "class", "box tr none wide");
    frame(&mut doc);
    assert!(
        !doc.tree.active_transitions.contains_key(&node.0),
        "the known gap: no transition (the spec starts 10px → 20px)"
    );
    assert_eq!(width_px(&doc, node), 20.0, "the box is at the new value");
}

/// F4: pausing a finished animation and resuming it costs nothing. A pause
/// used to clear `settled` (`is_complete` answers `false` while paused), so the
/// resume counted it as running and its node was restyled once more.
#[test]
fn resuming_a_paused_finished_animation_costs_no_frame() {
    let (mut doc, node) = finished("box none");
    set_class(&mut doc, node, "box none paused");
    assert_at_rest(&mut doc, node, "paused after the end");
    set_class(&mut doc, node, "box none");
    assert!(
        !doc.tree.has_running_animations(),
        "resumed, it is still finished"
    );
    doc.tree.perf.reset();
    assert!(!frame(&mut doc), "and asks for no frame");
    assert_eq!(
        doc.tree.perf.end_frame().get(Counter::ElementsCascaded),
        0,
        "nor a restyle"
    );
    assert_at_rest(&mut doc, node, "resumed after the end");
}

/// F2: a finished `font-size` one-shot writes nothing, so a later colour-only
/// restyle of its node (a hover) owes no re-measure and no layout.
#[test]
fn a_colour_restyle_after_a_finished_font_size_animation_owes_no_layout() {
    for (class, what) in [
        ("text", "control, no animation"),
        ("text sized", "after the end"),
    ] {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let p = doc.create_element("p");
        doc.set_attribute(p, "class", class);
        let t = doc.create_text("hello world");
        doc.append_child(p, t);
        doc.append_child(body, p);
        doc.tree.transitions_enabled = true;
        doc.resolve_layout(VP.0, VP.1);
        if doc.tree.active_animations.contains_key(&p.0) {
            age(&mut doc, p, 1000.0);
        }
        frame(&mut doc);
        frame(&mut doc);
        assert!(!doc.tree.layout_dirty, "{what}: precondition, at rest");
        if class == "text sized" {
            assert_eq!(names(&doc, p), ["k783-size"], "positive control: kept");
        }
        doc.set_attribute(p, "class", &format!("{class} red"));
        doc.resolve_styles();
        doc.apply_stylo_styles_to_taffy();
        assert!(
            !doc.tree.layout_dirty,
            "{what}: a colour-only restyle owes no layout"
        );
    }
}
