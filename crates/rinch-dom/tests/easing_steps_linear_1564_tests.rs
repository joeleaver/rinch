//! #1564 — `steps()`, `step-start`/`step-end` and `linear(<stops>)` ease the
//! way CSS Easing 2 says, in `@keyframes` animations and in transitions.
//!
//! All three converters (`transition::types::convert_timing_function`, the
//! animation extractor's copy and the per-keyframe converter) turned both
//! families into plain `linear`, so a stepped spinner rotated smoothly and a
//! `linear()` curve ran straight.
//!
//! Every expected value is Chrome 155's (headless, `getComputedStyle` of a
//! paused animation after setting its `currentTime`; for a transition, of the
//! running transition paused at a `currentTime`), on `@keyframes` from 40px to
//! 100px over 1000ms. Chrome rounds a computed width to 1/64px, which is the
//! tolerance on the `cubic-bezier` rows and why the rest are compared to
//! 0.01px. A paused rinch animation with a negative delay sits at the active
//! time Chrome's `currentTime` names, so nothing here sleeps.
//!
//! # Mutants, and what kills each
//!
//! Each run against the fix, with this file, `transition_tests`,
//! `transform_3d_tests` and `fractional_iteration_fill_tests`:
//!
//! | mutant | killed by |
//! |---|---|
//! | `steps()` converts to `Linear` (`main`) | 8 fixtures here |
//! | `linear()` converts to `Linear` (`main`) | 5 fixtures here |
//! | per-keyframe converter answers `Linear` | `a_keyframes_own_timing_function_eases_its_interval`, `an_iteration_boundary_is_eased_not_snapped_to_a_keyframe` |
//! | `start` not incremented (`start` ≡ `end`) | `steps_tick_at_the_jumps_chrome_takes`, `step_start_and_step_end`, and 6 more |
//! | before flag ignored | `the_before_flag_holds_a_jump_start_step_back`, the two transition fixtures |
//! | before flag set in every after phase | `step_start_and_step_end` (`step-end` filled forwards) |
//! | after-phase flag from iteration 0's direction | `the_before_flag_holds_a_jump_start_step_back` (`alternate 2`) |
//! | before-phase flag never set | `the_before_flag_holds_a_jump_start_step_back` |
//! | an iteration's start snaps to the keyframe (`main`) | `an_iteration_boundary_is_eased_not_snapped_to_a_keyframe` |
//! | an iteration's end snaps to the keyframe (`main`) | `an_iteration_boundary_is_eased_not_snapped_to_a_keyframe` |
//! | `linear()` input positions ignored | `linear_function_stops_and_positions`, and 4 more |
//! | `jump-none` / `jump-both` divide by `steps` | `jump_positions_follow_css_easing_2` |
//! | a transition's delay always shows `from` (`main`) | `a_transition_eases_with_steps_and_linear_functions`, `reversing_a_reversal_in_its_delay_reads_the_before_flag` |
//! | a transition's output progress in its delay without the flag | `reversing_a_reversal_in_its_delay_reads_the_before_flag` |
//! | an eased 0 / 1 interpolated rather than the keyframe's value | `transform_3d_tests::a_scale_z_meets_a_perspective` |

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DimensionValue;
use rinch_dom::transition::types::{
    AnimatableValue, StepPosition, TimingFunction, TransitionProperty,
};

const CSS: &str = "
    @keyframes k1564 { from { width: 40px; } to { width: 100px; } }
    @keyframes k1564b { from { width: 40px; } 50% { width: 100px; } to { width: 40px; } }
    @keyframes k1564p {
        from { width: 40px; animation-timing-function: steps(2); }
        to { width: 100px; }
    }
    @keyframes k1564q {
        from { width: 40px; animation-timing-function: steps(2, start); }
        50% { width: 100px; animation-timing-function: linear(0, 0.25 50%, 1); }
        to { width: 40px; }
    }
    .box { width: 10px; height: 10px; }
";

const VP: (f32, f32) = (800.0, 600.0);

fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

/// The width the box shows under `animation`.
fn animated_width(animation: &str) -> Option<f32> {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "class", "box");
    doc.set_attribute(node, "style", &format!("animation: {animation}"));
    doc.append_child(body, node);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    doc.tick_animations();
    doc.resolve_layout(VP.0, VP.1);
    width_px(&doc, node)
}

fn check_tol(cases: &[(&str, f32)], tol: f32) {
    let mut wrong = Vec::new();
    for &(animation, chrome) in cases {
        let got = animated_width(animation);
        if got.map(|w| (w - chrome).abs() > tol).unwrap_or(true) {
            wrong.push(format!("{animation}: rinch {got:?}, Chrome 155 {chrome}px"));
        }
    }
    assert!(wrong.is_empty(), "animated widths:\n{}", wrong.join("\n"));
}

fn check(cases: &[(&str, f32)]) {
    check_tol(cases, 0.01);
}

/// The issue's own case, and Chrome's `currentTime` samples of
/// `steps(4, end)` and `steps(4, start)` (a paused animation with a negative
/// delay of `t` is at `currentTime` `t`).
#[test]
fn steps_tick_at_the_jumps_chrome_takes() {
    check(&[
        ("k1564 40ms steps(4, end) 1.3 -1000ms forwards", 55.0),
        ("k1564 40ms steps(4, start) 1.3 -1000ms forwards", 70.0),
        ("k1564 1000ms steps(4, end) -300ms forwards paused", 55.0),
        ("k1564 1000ms steps(4, end) -500ms forwards paused", 70.0),
        ("k1564 1000ms steps(4, end) -999ms forwards paused", 85.0),
        ("k1564 1000ms steps(4, start) -300ms forwards paused", 70.0),
        ("k1564 1000ms steps(4, start) -750ms forwards paused", 100.0),
        ("k1564 1000ms steps(4, start) -999ms forwards paused", 100.0),
        ("k1564 1000ms steps(4) -300ms forwards paused", 55.0),
        // A reversed iteration feeds the step its directed progress.
        (
            "k1564 1000ms steps(4, start) -300ms reverse forwards paused",
            85.0,
        ),
        (
            "k1564 1000ms steps(4, start) -999ms reverse forwards paused",
            55.0,
        ),
        (
            "k1564 1000ms steps(4, end) -300ms reverse forwards paused",
            70.0,
        ),
        (
            "k1564 1000ms steps(4, start) -1300ms alternate 2 forwards paused",
            85.0,
        ),
        (
            "k1564 1000ms steps(4, end) -1300ms alternate 2 forwards paused",
            70.0,
        ),
    ]);
}

#[test]
fn step_start_and_step_end() {
    check(&[
        ("k1564 1000ms step-start -500ms forwards paused", 100.0),
        ("k1564 1000ms step-end -500ms forwards paused", 40.0),
        ("k1564 1000ms step-end -999ms forwards paused", 40.0),
        ("k1564 1000ms step-end -2000ms forwards paused", 100.0),
    ]);
}

/// CSS Easing 2's before flag: in the before phase going forwards, and in the
/// after phase going backwards, a step at a jump point is the one before it.
/// So `steps(4, start)` shows its `from` value through a backwards-filled
/// delay (55px without the flag), and at the end of a reversed animation.
#[test]
fn the_before_flag_holds_a_jump_start_step_back() {
    check(&[
        ("k1564 1000ms steps(4, start) 1000ms backwards paused", 40.0),
        ("k1564 1000ms steps(4, start) 1000ms both paused", 40.0),
        // Before phase going backwards: no flag, directed progress 1.
        (
            "k1564 1000ms steps(4, start) reverse 1000ms both paused",
            100.0,
        ),
        (
            "k1564 1000ms steps(4, end) reverse 1000ms both paused",
            100.0,
        ),
        // After phase going backwards: the flag.
        (
            "k1564 1000ms steps(4, start) -2000ms reverse forwards paused",
            40.0,
        ),
        (
            "k1564 1000ms steps(4, start) -5000ms alternate 2 forwards paused",
            40.0,
        ),
        (
            "k1564 1000ms steps(4, end) -5000ms alternate 2 forwards paused",
            40.0,
        ),
    ]);
}

/// The after phase going forwards sets no flag.
#[test]
fn the_before_flag_is_not_set_in_the_after_phase_going_forwards() {
    check(&[
        (
            "k1564 1000ms steps(3, start) 1.5 -5000ms forwards paused",
            80.0,
        ),
        (
            "k1564 1000ms steps(4, start) -2000ms forwards paused",
            100.0,
        ),
    ]);
}

/// Web Animations picks an *interval* at an iteration's start and end and
/// eases its distance (0 or 1); it used to snap to the keyframe there, which
/// only an easing with f(0) = 0 and f(1) = 1 agrees with.
#[test]
fn an_iteration_boundary_is_eased_not_snapped_to_a_keyframe() {
    check(&[
        ("k1564 1000ms steps(4, start) forwards paused", 55.0),
        ("k1564 1000ms step-start forwards paused", 100.0),
        ("k1564 1000ms linear(0.5, 1) forwards paused", 70.0),
        ("k1564 1000ms linear(0, 0.5) -2000ms forwards paused", 70.0),
        // A backwards fill's progress 0 eases too.
        ("k1564 1000ms linear(0.5, 1) 1000ms backwards paused", 70.0),
        ("k1564q 1000ms linear forwards paused", 70.0),
    ]);
}

#[test]
fn linear_function_stops_and_positions() {
    check(&[
        (
            "k1564 1000ms linear(0, 0.25, 1) -250ms forwards paused",
            47.5,
        ),
        (
            "k1564 1000ms linear(0, 0.25, 1) -500ms forwards paused",
            55.0,
        ),
        (
            "k1564 1000ms linear(0, 0.25, 1) -750ms forwards paused",
            77.5,
        ),
        (
            "k1564 1000ms linear(0, 0.5 25%, 1) -100ms forwards paused",
            52.0,
        ),
        (
            "k1564 1000ms linear(0, 0.5 25%, 1) -250ms forwards paused",
            70.0,
        ),
        (
            "k1564 1000ms linear(0, 0.5 25%, 1) -500ms forwards paused",
            80.0,
        ),
        // Two positions hold the output flat between them.
        (
            "k1564 1000ms linear(0, 0.2 20% 40%, 1) -100ms forwards paused",
            46.0,
        ),
        (
            "k1564 1000ms linear(0, 0.2 20% 40%, 1) -300ms forwards paused",
            52.0,
        ),
        (
            "k1564 1000ms linear(0, 0.2 20% 40%, 1) -500ms forwards paused",
            60.0,
        ),
        (
            "k1564 1000ms linear(0, 0.2 20% 40%, 1) -700ms forwards paused",
            76.0,
        ),
        // A stop with no position is spread evenly between its neighbours
        // (0.3 lands at 40%).
        (
            "k1564 1000ms linear(0, 0.3, 0.6 80%, 1) -200ms forwards paused",
            49.0,
        ),
        (
            "k1564 1000ms linear(0, 0.3, 0.6 80%, 1) -400ms forwards paused",
            58.0,
        ),
        (
            "k1564 1000ms linear(0, 0.3, 0.6 80%, 1) -600ms forwards paused",
            67.0,
        ),
        (
            "k1564 1000ms linear(0, 0.3, 0.6 80%, 1) -900ms forwards paused",
            88.0,
        ),
        ("k1564 1000ms linear(0.5, 1) -500ms forwards paused", 85.0),
        ("k1564 1000ms linear(0, 0.5) -500ms forwards paused", 55.0),
        // Outputs past 1 extrapolate the value.
        (
            "k1564 1000ms linear(0, 1.5 50%, 1) -250ms forwards paused",
            85.0,
        ),
        (
            "k1564 1000ms linear(0, 1.5 50%, 1) -500ms forwards paused",
            130.0,
        ),
        (
            "k1564 1000ms linear(0, 1.5 50%, 1) -750ms forwards paused",
            115.0,
        ),
        // Two stops at one input: a jump, and the later stop wins at it.
        (
            "k1564 1000ms linear(0, 0.9 50%, 0.1 50%, 1) -250ms forwards paused",
            67.0,
        ),
        (
            "k1564 1000ms linear(0, 0.9 50%, 0.1 50%, 1) -500ms forwards paused",
            46.0,
        ),
        (
            "k1564 1000ms linear(0, 0.9 50%, 0.1 50%, 1) -750ms forwards paused",
            73.0,
        ),
    ]);
}

/// `animation-timing-function` inside a keyframe eases the interval that
/// keyframe starts, over the animation's own.
#[test]
fn a_keyframes_own_timing_function_eases_its_interval() {
    check(&[
        ("k1564p 1000ms linear -300ms forwards paused", 40.0),
        ("k1564p 1000ms linear -600ms forwards paused", 70.0),
        ("k1564p 1000ms ease -300ms forwards paused", 40.0),
        ("k1564p 1000ms ease -600ms forwards paused", 70.0),
        ("k1564b 1000ms steps(2) -300ms forwards paused", 70.0),
        ("k1564b 1000ms steps(2) -800ms forwards paused", 70.0),
        ("k1564q 1000ms linear -100ms forwards paused", 70.0),
        ("k1564q 1000ms linear -300ms forwards paused", 100.0),
        ("k1564q 1000ms linear -600ms forwards paused", 94.0),
        ("k1564q 1000ms linear -750ms forwards paused", 85.0),
        ("k1564q 1000ms linear -900ms forwards paused", 58.0),
    ]);
}

/// Already right before #1564; pinned so the rewrite keeps it so, to Chrome's
/// 1/64px.
#[test]
fn cubic_bezier_matches_chrome() {
    check_tol(
        &[
            (
                "k1564 1000ms cubic-bezier(0.1, 0.7, 1, 0.1) -300ms forwards paused",
                62.140625,
            ),
            (
                "k1564 1000ms cubic-bezier(0.1, 0.7, 1, 0.1) -600ms forwards paused",
                66.34375,
            ),
            (
                "k1564 1000ms cubic-bezier(0.5, -0.5, 0.5, 1.5) -200ms forwards paused",
                35.734375,
            ),
            (
                "k1564 1000ms cubic-bezier(0.5, -0.5, 0.5, 1.5) -800ms forwards paused",
                104.25,
            ),
        ],
        1.0 / 64.0,
    );
}

/// The `jump-*` keywords do not parse in Stylo's servo build (the parser's
/// `step_position_jump_enabled` answers `false` there), so the declaration is
/// dropped before rinch sees it; the easing itself is pinned here, as output
/// progress, against Chrome 155's widths (40 + 60 × output).
#[test]
fn jump_positions_follow_css_easing_2() {
    let cases: &[(StepPosition, f32, f32)] = &[
        (StepPosition::JumpNone, 0.0, 40.0),
        (StepPosition::JumpNone, 0.3, 60.0),
        (StepPosition::JumpNone, 0.5, 80.0),
        (StepPosition::JumpNone, 0.999, 100.0),
        (StepPosition::JumpNone, 1.0, 100.0),
        (StepPosition::JumpBoth, 0.0, 52.0),
        (StepPosition::JumpBoth, 0.3, 64.0),
        (StepPosition::JumpBoth, 0.5, 76.0),
        (StepPosition::JumpBoth, 0.999, 88.0),
        (StepPosition::JumpBoth, 1.0, 100.0),
        (StepPosition::JumpStart, 0.0, 55.0),
        (StepPosition::JumpStart, 0.3, 70.0),
        (StepPosition::JumpEnd, 0.3, 55.0),
    ];
    let mut wrong = Vec::new();
    for &(pos, t, chrome) in cases {
        let got = 40.0 + 60.0 * TimingFunction::Steps(4, pos).apply(t);
        if (got - chrome).abs() > 0.01 {
            wrong.push(format!(
                "steps(4, {pos:?}) at {t}: rinch {got}, Chrome {chrome}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The `jump-*` keywords parse since stylo 0.21 (#1581): with stylo 0.11 the
/// whole `animation` shorthand was dropped and the box kept its 10px. Chrome
/// 155 animates it, 60px here, as `steps(4, jump-none)` at 0.3 above.
#[test]
fn a_jump_keyword_animates_as_chrome_does() {
    check(&[(
        "k1564 1000ms steps(4, jump-none) -300ms forwards paused",
        60.0,
    )]);
}

// ── Transitions ─────────────────────────────────────────────────────────────

/// A box at 40px with `transition`, moved to 100px: the running width
/// transition.
fn start_transition(transition: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "class", "box");
    doc.set_attribute(
        node,
        "style",
        &format!("width: 40px; transition: {transition}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.transitions_enabled = true;
    doc.set_style(node, "width", "100px");
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    assert!(
        doc.tree
            .active_transitions
            .get(&node.0)
            .is_some_and(|m| m.contains_key(&TransitionProperty::Width)),
        "precondition: a width transition runs for `{transition}`"
    );
    (doc, node)
}

fn px(v: Option<AnimatableValue>) -> Option<f32> {
    match v? {
        AnimatableValue::Dimension(DimensionValue::Length(px)) => Some(px),
        _ => None,
    }
}

fn transition_width_at(doc: &RinchDocument, node: NodeId, t: f64) -> Option<f32> {
    let tr = &doc.tree.active_transitions[&node.0][&TransitionProperty::Width];
    px(tr.value_at(tr.start_time_ms + t))
}

#[test]
fn a_transition_eases_with_steps_and_linear_functions() {
    let cases: &[(&str, f64, f32)] = &[
        ("width 1000ms steps(4, end)", 0.0, 40.0),
        ("width 1000ms steps(4, end)", 300.0, 55.0),
        ("width 1000ms steps(4, end)", 500.0, 70.0),
        ("width 1000ms steps(4, end)", 999.0, 85.0),
        ("width 1000ms steps(4, start)", 0.0, 55.0),
        ("width 1000ms steps(4, start)", 300.0, 70.0),
        ("width 1000ms steps(4, start)", 999.0, 100.0),
        // A transition fills backwards through its delay.
        ("width 1000ms steps(4, start) 1000ms", 0.0, 40.0),
        ("width 1000ms steps(4, start) 1000ms", 500.0, 40.0),
        ("width 1000ms steps(4, start) 1000ms", 1300.0, 70.0),
        ("width 1000ms linear(0, 0.25, 1)", 250.0, 47.5),
        ("width 1000ms linear(0, 0.25, 1)", 500.0, 55.0),
        ("width 1000ms linear(0, 0.25, 1)", 750.0, 77.5),
        // Through the delay the easing is sampled at 0 with the before
        // flag: `linear(0.5, 1)` is half way there before it starts.
        ("width 1000ms linear(0.5, 1) 1000ms", 500.0, 70.0),
        ("width 1000ms linear(0.5, 1) 1000ms", 1000.0, 70.0),
    ];
    let mut wrong = Vec::new();
    for &(transition, t, chrome) in cases {
        let (doc, node) = start_transition(transition);
        let got = transition_width_at(&doc, node, t);
        if got.map(|w| (w - chrome).abs() > 0.01).unwrap_or(true) {
            wrong.push(format!(
                "{transition} at {t}ms: rinch {got:?}, Chrome 155 {chrome}px"
            ));
        }
    }
    assert!(wrong.is_empty(), "transition widths:\n{}", wrong.join("\n"));
}

/// css-transitions-1 §3 shortens a reversal by the running transition's
/// *output* progress: `steps(4, end)` at 600ms has output 0.5, so going back
/// takes 500ms (Chrome 155: 500), from 70px; `steps(4, start)` has 0.75 there,
/// so 750ms from 85px. A step's output is flat around 600ms, which keeps the
/// fixture off the wall clock the transition is started on.
#[test]
fn a_stepped_transition_reverses_from_its_output_progress() {
    /// (transition, Chrome's reversed duration, samples of the reversal)
    type Case<'a> = (&'a str, f64, &'a [(f64, f32)]);
    let cases: &[Case] = &[
        (
            "width 1000ms steps(4, end)",
            500.0,
            &[(0.0, 70.0), (200.0, 62.5), (300.0, 55.0)],
        ),
        (
            "width 1000ms steps(4, start)",
            750.0,
            &[(0.0, 73.75), (300.0, 62.5)],
        ),
    ];
    for &(transition, chrome_duration, samples) in cases {
        let (mut doc, node) = start_transition(transition);
        doc.tree
            .active_transitions
            .get_mut(&node.0)
            .unwrap()
            .get_mut(&TransitionProperty::Width)
            .unwrap()
            .start_time_ms -= 600.0;
        // The before-change style is the running transition's current value.
        doc.tick_transitions();
        doc.resolve_layout(VP.0 + 3.0, VP.1);
        doc.set_style(node, "width", "40px");
        doc.resolve_layout(VP.0 + 2.0, VP.1);
        let tr = &doc.tree.active_transitions[&node.0][&TransitionProperty::Width];
        assert!(
            (tr.duration_ms - chrome_duration).abs() < 0.01,
            "{transition}: reversed duration {} where Chrome 155 takes {chrome_duration}",
            tr.duration_ms
        );
        for &(t, chrome) in samples {
            let got = transition_width_at(&doc, node, t);
            assert!(
                got.is_some_and(|w| (w - chrome).abs() < 0.01),
                "{transition} reversed, at {t}ms: rinch {got:?}, Chrome 155 {chrome}px"
            );
        }
    }
}

/// Reversing a reversal while it is still in its (unshortened, positive)
/// delay: the factor folds in the running transition's output progress there,
/// which is its easing at 0 with the before flag set. Chrome 155, on
/// `width 1000ms <easing> 500ms` sent 40 → 100, back to 40 at `currentTime`
/// 1100, and to 100 again 200ms into that reversal's delay:
///
/// | easing | first reversal | value in its delay | second reversal |
/// |---|---|---|---|
/// | `steps(4, start)` | 750ms | 85px | 250ms (`apply(0)` without the flag: 437.5) |
/// | `steps(4, end)` | 500ms | 70px | 500ms |
/// | `linear(0.5, 1)` | 800ms | 64px | 600ms |
///
/// Driven on an explicit clock through `start_transitions`, with each easing
/// taken from a transition the document converted.
#[test]
fn reversing_a_reversal_in_its_delay_reads_the_before_flag() {
    use rinch_dom::transition::types::{PropertyChange, TransitionSpec};
    use std::collections::HashMap;

    let lp = |v: f32| AnimatableValue::Dimension(DimensionValue::Length(v));
    let cases: &[(&str, f64, f32, f64)] = &[
        ("steps(4, start)", 750.0, 85.0, 250.0),
        ("steps(4, end)", 500.0, 70.0, 500.0),
        ("linear(0.5, 1)", 800.0, 64.0, 600.0),
    ];
    for &(easing, chrome_first, chrome_in_delay, chrome_second) in cases {
        let (doc, node) = start_transition(&format!("width 1000ms {easing} 500ms"));
        let timing = doc.tree.active_transitions[&node.0][&TransitionProperty::Width]
            .timing
            .clone();
        let spec = TransitionSpec {
            property: TransitionProperty::Width,
            duration_ms: 1000.0,
            delay_ms: 500.0,
            timing,
        };
        let change = |old: f32, new: f32| PropertyChange {
            property: TransitionProperty::Width,
            old_value: lp(old),
            new_value: lp(new),
        };
        let mut active = HashMap::new();
        rinch_dom::transition::start_transitions(
            &mut active,
            std::slice::from_ref(&spec),
            &[change(40.0, 100.0)],
            0.0,
        );
        let now = px(active[&TransitionProperty::Width].value_at(1100.0)).unwrap();
        rinch_dom::transition::start_transitions(
            &mut active,
            std::slice::from_ref(&spec),
            &[change(now, 40.0)],
            1100.0,
        );
        let first = &active[&TransitionProperty::Width];
        assert!(
            (first.duration_ms - chrome_first).abs() < 0.01,
            "{easing}: first reversal {} where Chrome 155 takes {chrome_first}",
            first.duration_ms
        );
        let in_delay = px(first.value_at(1300.0)).unwrap();
        assert!(
            (in_delay - chrome_in_delay).abs() < 0.01,
            "{easing}: {in_delay}px in the reversal's delay where Chrome 155 shows {chrome_in_delay}px"
        );
        rinch_dom::transition::start_transitions(
            &mut active,
            std::slice::from_ref(&spec),
            &[change(in_delay, 100.0)],
            1300.0,
        );
        let second = &active[&TransitionProperty::Width];
        assert!(
            (second.duration_ms - chrome_second).abs() < 0.01,
            "{easing}: second reversal {} where Chrome 155 takes {chrome_second}",
            second.duration_ms
        );
    }
}
