//! #785 — a fractional `animation-iteration-count` fills where the animation
//! stopped, not at the end of its last whole iteration.
//!
//! An animation's active duration is `duration × iteration-count`, and a
//! `forwards` fill holds the value at the end of it (css-animations-1; Web
//! Animations §4.8.4): the iteration `floor(count)` at progress `fract(count)`,
//! or — when the count is whole and above zero — the last iteration at
//! progress 1. The direction is taken from *that* iteration's index. A count of
//! 0 fills at progress 0 of iteration 0. `ActiveAnimation::end_progress` took
//! the iteration `ceil(count) - 1` at progress 1 for every count, so
//! `2.5 forwards` filled at the `to` value and `0 forwards` at the `to` value
//! too.
//!
//! Every expected value is Chrome 155's (headless, `getComputedStyle` after
//! setting each animation's `currentTime` past its end), on the same
//! `@keyframes` (`from 40px to 100px`) and a 10px box. A negative delay puts
//! each animation past its end at the first tick, so nothing here sleeps.
//!
//! # Mutants, and what kills each
//!
//! | mutant | killed by |
//! |---|---|
//! | `main`: iteration `ceil(n) - 1` at progress 1 | every fixture with a fractional or zero count (5 of 6) |
//! | progress `fract(n)`, iteration always 0 | `alternate_takes_its_direction_from_the_iteration_it_stopped_in` |
//! | progress `fract(n)`, iteration `floor(n) + 1` | that, and `a_count_below_one_fills_inside_the_first_iteration` |
//! | a whole count ends at progress 0 of its last iteration | `whole_counts_still_fill_at_the_end_of_the_last_iteration` |
//! | a count of 0 ends at progress 1 | `a_count_of_zero_fills_at_the_start` |
//! | an infinite (zero-duration) count ends in an odd iteration | `a_zero_duration_fills_where_its_count_ends`, and `reverse_directions_fill_at_the_from_value` in `zero_duration_infinite_animation_tests` |
//!
//! (`ceil(n) - 1` for the iteration with `fract(n)` progress is *not* a
//! mutant: for a fractional `n` it equals `floor(n)`.)

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    @keyframes k785 { from { width: 40px; } to { width: 100px; } }
    .box { width: 10px; height: 10px; }
";

const VP: (f32, f32) = (800.0, 600.0);

fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

/// The width the box shows once `animation` is past its end.
fn filled_width(animation: &str) -> Option<f32> {
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

fn check(cases: &[(&str, f32)]) {
    let mut wrong = Vec::new();
    for &(animation, chrome) in cases {
        let got = filled_width(animation);
        if got.map(|w| (w - chrome).abs() > 0.05).unwrap_or(true) {
            wrong.push(format!("{animation}: rinch {got:?}, Chrome 155 {chrome}px"));
        }
    }
    assert!(wrong.is_empty(), "fill values:\n{}", wrong.join("\n"));
}

/// The issue's own case: 2.5 iterations stop half way through the third.
#[test]
fn a_fractional_count_fills_where_the_animation_stopped() {
    check(&[
        ("k785 40ms linear 2.5 -1000ms forwards", 70.0),
        ("k785 40ms linear 1.25 -1000ms forwards", 55.0),
        ("k785 40ms linear 1.25 -1000ms both", 55.0),
        ("k785 40ms linear 2.5 -1000ms reverse forwards", 70.0),
        // The timing function applies to the progress the fill stops at
        // (ease-in at 0.5; rinch's cubic-bézier solve is 0.015px off Chrome's).
        ("k785 40ms ease-in 2.5 -1000ms forwards", 58.90625),
    ]);
}

/// Off the symmetric 0.5: the third iteration of `alternate` runs backwards,
/// so 1.25 iterations stop a quarter of the way back from `to`.
#[test]
fn alternate_takes_its_direction_from_the_iteration_it_stopped_in() {
    check(&[
        ("k785 40ms linear 1.25 -1000ms alternate forwards", 85.0),
        ("k785 40ms linear 3.75 -1000ms alternate forwards", 55.0),
        ("k785 40ms linear 3.75 -1000ms alternate-reverse forwards", 85.0),
    ]);
}

/// Less than one iteration: the first, at its fraction.
#[test]
fn a_count_below_one_fills_inside_the_first_iteration() {
    check(&[
        ("k785 40ms linear 0.25 -1000ms forwards", 55.0),
        ("k785 40ms linear 0.25 -1000ms reverse forwards", 85.0),
        ("k785 40ms linear 0.25 -1000ms alternate-reverse forwards", 85.0),
    ]);
}

/// A count of 0 has an active duration of 0 and fills at its start.
#[test]
fn a_count_of_zero_fills_at_the_start() {
    check(&[
        ("k785 40ms linear 0 forwards", 40.0),
        ("k785 40ms linear 0 reverse forwards", 100.0),
        ("k785 40ms linear 0 both", 40.0),
        ("k785 0s linear 0 forwards", 40.0),
    ]);
}

/// Whole counts are unchanged: the last iteration, at its end.
#[test]
fn whole_counts_still_fill_at_the_end_of_the_last_iteration() {
    check(&[
        ("k785 40ms linear 2 -1000ms forwards", 100.0),
        ("k785 40ms linear 2 -1000ms alternate forwards", 40.0),
        ("k785 40ms linear 2 -1000ms alternate-reverse forwards", 100.0),
        ("k785 0s linear 2 alternate forwards", 40.0),
    ]);
}

/// A zero duration (#786) fills at the same place: the active duration is 0,
/// and the count still says where it ends.
#[test]
fn a_zero_duration_fills_where_its_count_ends() {
    check(&[
        ("k785 0s linear 2.5 forwards", 70.0),
        ("k785 0s linear 1.5 alternate forwards", 70.0),
        ("k785 0s linear 0.25 forwards", 55.0),
        ("k785 0s linear infinite forwards", 100.0),
        ("k785 0s linear infinite alternate forwards", 100.0),
        ("k785 0s linear infinite alternate-reverse forwards", 40.0),
        ("k785 0s linear infinite reverse forwards", 40.0),
    ]);
}
