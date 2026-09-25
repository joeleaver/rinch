//! #414: how a `transform` is interpolated, by a transition and by an
//! `@keyframes` animation alike.
//!
//! CSS Transforms (css-transforms-2 §"Interpolation of Transforms") never
//! interpolates the raw matrix. Two lists whose functions pair up by type are
//! interpolated **function by function** — `rotate(0deg)` → `rotate(90deg)` is
//! `rotate(45deg)` half way, a full-size rotation. A shorter list is padded
//! with identity functions of the other list's types, and `none` is the empty
//! list. From the first pair that does not match, what remains of each list is
//! composed to a matrix and interpolated by **decomposition** (translate,
//! rotation angle, the residual skew, scale), then recomposed.
//!
//! rinch lerped the six matrix entries, which is a correct rotation angle with
//! a uniform scale of `cos(θ/2)`: a 90° rotation shrank to 70.7% at its
//! midpoint and a 180° one to a point.
//!
//! **Every expected matrix below was measured in Chrome 153**
//! (`getComputedStyle(el).transform` on a 100×40 box with `transform-origin:
//! 0 0`, an `Element.animate()` effect paused at `currentTime`, and for case 1
//! also a real CSS transition paused the same way — both gave the same matrix).
//! Chrome prints six significant figures, hence the tolerance. The samples sit
//! at 0.2, 0.35 and 0.8 as well as 0.5: a midpoint alone is a fixed point for
//! more than one wrong interpolation (the elementwise lerp of `rotate(170deg)`
//! and `rotate(-170deg) scale(1.5)` also lands on a diagonal matrix there).

// The expected matrices are Chrome's printed numbers, `0.707107` among them,
// kept verbatim rather than respelled as `FRAC_1_SQRT_2`.
#![allow(clippy::approx_constant)]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::transition::{AnimatableValue, TransformOp, TransitionProperty};

/// A 100×40 box, so a percentage translate is visible in the result.
const BOX: &str = "position: absolute; left: 0; top: 0; width: 100px; height: 40px; \
                   transform-origin: 0 0;";

/// The element's transform as Chrome reports it: the matrix with the
/// percentage translate resolved against the 100×40 border box.
fn resolved(doc: &RinchDocument, id: NodeId) -> [f64; 6] {
    let tf = &doc.tree.nodes[id.0].computed_style.transform;
    let (w, h) = (100.0, 40.0);
    let mut m = tf.matrix;
    m[4] += tf.pct_translate_w[0] * w + tf.pct_translate_h[0] * h;
    m[5] += tf.pct_translate_w[1] * w + tf.pct_translate_h[1] * h;
    m
}

fn assert_matrix(got: [f64; 6], want: [f64; 6], what: &str) {
    let close = got
        .iter()
        .zip(want.iter())
        .all(|(g, w)| (g - w).abs() <= 2e-4 * w.abs().max(1.0));
    assert!(close, "{what}: got {got:?}, Chrome 153 gives {want:?}");
}

/// A document with one `.t` div, `from` as its transform, a 1000ms linear
/// transition, laid out once with transitions armed.
fn transition_doc(from: &str, to: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(&format!(
        ".t {{ {BOX} transition: transform 1000ms linear; transform: {from}; }} \
         .t.on {{ transform: {to}; }}"
    ));
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "t");
    doc.append_child(body, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// Flip the class and return the transform transition's start time.
fn start(doc: &mut RinchDocument, div: NodeId, class: &str) -> f64 {
    doc.set_attribute(div, "class", class);
    doc.resolve_layout(800.0, 600.0);
    doc.tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::Transform))
        .expect("the class change should have started a transform transition")
        .start_time_ms
}

/// Run `from` → `to` as a transition and check every `(fraction, matrix)`.
fn check_transition(from: &str, to: &str, samples: &[(f64, [f64; 6])]) {
    let (mut doc, div) = transition_doc(from, to);
    let t0 = start(&mut doc, div, "t on");
    for (f, want) in samples {
        rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + f * 1000.0);
        assert_matrix(
            resolved(&doc, div),
            *want,
            &format!("transition {from} -> {to} at {f}"),
        );
    }
}

/// The same pair as an `@keyframes` animation, checked through the style the
/// animation tick writes.
fn check_keyframes(from: &str, to: &str, samples: &[(f64, [f64; 6])]) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(&format!(
        "@keyframes k {{ from {{ transform: {from}; }} to {{ transform: {to}; }} }} \
         .a {{ {BOX} animation: k 1000ms linear; }}"
    ));
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "a");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let t0 = doc
        .tree
        .active_animations
        .get(&div.0)
        .and_then(|a| a.first())
        .expect("the animation should be running after layout")
        .start_time_ms;
    for (f, want) in samples {
        rinch_dom::animation::tick_animations(&mut doc.tree, t0 + f * 1000.0);
        assert_matrix(
            resolved(&doc, div),
            *want,
            &format!("@keyframes {from} -> {to} at {f}"),
        );
    }
}

/// The issue's own case: two matching lists, one of them carrying a
/// percentage translate in the rotated frame.
const ROTATE_THEN_PCT: [(f64, [f64; 6]); 4] = [
    (
        0.2,
        [0.951057, 0.309017, -0.309017, 0.951057, 47.5528, 15.4508],
    ),
    (
        0.35,
        [0.85264, 0.522499, -0.522499, 0.85264, 42.632, 26.1249],
    ),
    (
        0.5,
        [0.707107, 0.707107, -0.707107, 0.707107, 35.3553, 35.3553],
    ),
    (
        0.8,
        [0.309017, 0.951057, -0.951057, 0.309017, 15.4508, 47.5528],
    ),
];

#[test]
fn a_rotation_keeps_its_size_and_rotates_its_percentage_translate() {
    check_transition(
        "rotate(0deg) translateX(50%)",
        "rotate(90deg) translateX(50%)",
        &ROTATE_THEN_PCT,
    );
}

#[test]
fn keyframes_rotate_by_function_too() {
    check_keyframes(
        "rotate(0deg) translateX(50%)",
        "rotate(90deg) translateX(50%)",
        &ROTATE_THEN_PCT,
    );
}

/// Two functions, each changing: the translate is interpolated in the frame of
/// the *interpolated* rotation, not lerped as a composed offset.
#[test]
fn rotate_and_translate_interpolate_pairwise() {
    check_transition(
        "rotate(30deg) translate(10px, 20px)",
        "rotate(120deg) translate(40px, -10px)",
        &[
            (
                0.2,
                [0.669131, 0.743145, -0.743145, 0.669131, 0.302062, 21.2581],
            ),
            (
                0.35,
                [0.477159, 0.878817, -0.878817, 0.477159, 1.43299, 22.5488],
            ),
            (
                0.8,
                [-0.207912, 0.978148, -0.978148, -0.207912, -3.15641, 34.0887],
            ),
        ],
    );
}

/// `none` is the identity function of the other list's type.
#[test]
fn none_to_a_rotation_is_a_rotation_from_zero() {
    check_transition(
        "none",
        "rotate(90deg)",
        &[
            (0.2, [0.951057, 0.309017, -0.309017, 0.951057, 0.0, 0.0]),
            (0.35, [0.85264, 0.522499, -0.522499, 0.85264, 0.0, 0.0]),
        ],
    );
}

/// The padding is a `rotate(0deg)` *function*, so `none` → `rotate(270deg)`
/// goes the long way round, by function — decomposing the pair would take
/// the short way, −90°.
#[test]
fn none_pads_by_function_so_a_long_turn_stays_long() {
    check_transition(
        "none",
        "rotate(270deg)",
        &[
            (0.2, [0.587785, 0.809017, -0.809017, 0.587785, 0.0, 0.0]),
            (0.8, [-0.809017, -0.587785, 0.587785, -0.809017, 0.0, 0.0]),
        ],
    );
}

/// A shorter list is padded with identity functions: `scale` runs 1 → 2
/// alongside the rotation.
#[test]
fn a_shorter_list_is_padded_with_identity_functions() {
    let samples = [
        (0.2, [1.07855, 0.526045, -0.526045, 1.07855, 0.0, 0.0]),
        (0.35, [1.06381, 0.831143, -0.831143, 1.06381, 0.0, 0.0]),
        (0.8, [0.496147, 1.73027, -1.73027, 0.496147, 0.0, 0.0]),
    ];
    check_transition("rotate(10deg)", "rotate(90deg) scale(2)", &samples);
    check_keyframes("rotate(10deg)", "rotate(90deg) scale(2)", &samples);
}

/// Per-function interpolation does not take the shortest way round:
/// 170° → −170° passes through 0°, not 180°.
#[test]
fn a_rotation_angle_is_interpolated_linearly_not_shortest_path() {
    check_transition(
        "rotate(170deg)",
        "rotate(-170deg) scale(1.5)",
        &[
            (0.2, [-0.228703, 1.07596, -1.07596, -0.228703, 0.0, 0.0]),
            (0.35, [0.739451, 0.913146, -0.913146, 0.739451, 0.0, 0.0]),
            (0.5, [1.25, 0.0, 0.0, 1.25, 0.0, 0.0]),
            (0.8, [-0.291076, -1.36941, 1.36941, -0.291076, 0.0, 0.0]),
        ],
    );
}

/// The matching prefix is interpolated by function and the rest by matrix.
#[test]
fn a_matching_prefix_then_a_matrix_remainder() {
    check_transition(
        "rotate(10deg) scale(1)",
        "rotate(90deg) translateX(20px)",
        &[
            (
                0.2,
                [0.898794, 0.438371, -0.438371, 0.898794, 3.59518, 1.75348],
            ),
            (
                0.35,
                [0.788011, 0.615661, -0.615661, 0.788011, 5.51608, 4.30963],
            ),
            (
                0.8,
                [0.275637, 0.961262, -0.961262, 0.275637, 4.4102, 15.3802],
            ),
        ],
    );
}

/// `matrix()` pairs with `matrix()` — by decomposition — and the functions
/// after it go on pairing.
#[test]
fn a_matrix_pair_does_not_end_the_matching_prefix() {
    check_transition(
        "matrix(1, 0, 0, 1, 0, 0) rotate(0deg)",
        "matrix(2, 0, 0, 1, 10, 0) rotate(90deg)",
        &[
            (0.2, [1.14127, 0.309017, -0.37082, 0.951057, 2.0, 0.0]),
            (0.35, [1.15106, 0.522499, -0.705373, 0.85264, 3.5, 0.0]),
            (0.8, [0.556231, 0.951057, -1.7119, 0.309017, 8.0, 0.0]),
        ],
    );
}

// ── Mismatched lists: matrix decomposition ──

#[test]
fn a_rotation_to_a_scale_decomposes() {
    let samples = [
        (0.2, [1.2, 0.0, 0.0, 1.2, 0.0, 0.0]),
        (0.35, [1.35, 0.0, 0.0, 1.35, 0.0, 0.0]),
        (0.8, [1.8, 0.0, 0.0, 1.8, 0.0, 0.0]),
    ];
    check_transition("rotate(0deg)", "scale(2)", &samples);
    check_keyframes("rotate(0deg)", "scale(2)", &samples);
}

/// A decomposed rotation takes the short way: 170° → −170° through 180°,
/// where the same pair by function goes through 0°
/// (`a_rotation_angle_is_interpolated_linearly_not_shortest_path`).
#[test]
fn a_decomposed_rotation_takes_the_short_way() {
    check_transition(
        "rotate(170deg)",
        "scale(1) rotate(-170deg)",
        &[
            (0.2, [-0.994522, 0.104528, -0.104528, -0.994522, 0.0, 0.0]),
            (0.8, [-0.994522, -0.104528, 0.104528, -0.994522, 0.0, 0.0]),
        ],
    );
    // And the other way round, which wraps the other way.
    check_transition(
        "rotate(-170deg)",
        "scale(1) rotate(170deg)",
        &[
            (0.2, [-0.994522, -0.104528, 0.104528, -0.994522, 0.0, 0.0]),
            (0.8, [-0.994522, 0.104528, -0.104528, -0.994522, 0.0, 0.0]),
        ],
    );
}

/// A reflection against a rotation: the decomposition's flipped axis.
#[test]
fn a_reflection_to_a_rotation_decomposes() {
    let samples = [
        (0.2, [-0.570634, -0.18541, -0.309017, 0.951057, 0.0, 0.0]),
        (0.35, [-0.255792, -0.15675, -0.522499, 0.85264, 0.0, 0.0]),
        (0.8, [0.18541, 0.570634, -0.951057, 0.309017, 0.0, 0.0]),
    ];
    check_transition("scale(-1, 1)", "rotate(90deg)", &samples);
    check_keyframes("scale(-1, 1)", "rotate(90deg)", &samples);
}

/// Skew, a non-uniform scale and a rotation past 180° all at once.
#[test]
fn skew_scale_and_rotation_decompose_together() {
    check_transition(
        "skewX(30deg) rotate(20deg)",
        "scale(2, 0.5) rotate(200deg)",
        &[
            (0.2, [0.845563, 1.02325, -0.502448, 0.615945, 0.0, 0.0]),
            (0.35, [0.353782, 1.38799, -0.721852, 0.13505, 0.0, 0.0]),
            (0.5, [-0.292527, 1.50923, -0.630672, -0.351376, 0.0, 0.0]),
            (0.8, [-1.53559, 0.833505, 0.16217, -0.762002, 0.0, 0.0]),
        ],
    );
}

/// `skewX` and `skewY` are not one primitive in Chrome: the pair decomposes.
#[test]
fn skew_x_against_skew_y_decomposes() {
    check_transition(
        "skewX(10deg)",
        "skewY(20deg)",
        &[
            (0.2, [1.01037, 0.0706518, 0.141846, 1.00027, 0.0, 0.0]),
            (0.35, [1.01484, 0.124607, 0.115831, 1.00047, 0.0, 0.0]),
            (0.8, [1.01062, 0.289789, 0.0363174, 1.00052, 0.0, 0.0]),
        ],
    );
}

/// A decomposed pair still carries a percentage translate, linearly.
#[test]
fn a_decomposed_pair_carries_its_percentage_translate() {
    check_transition(
        "translateX(50%) rotate(0deg)",
        "scale(2) rotate(90deg)",
        &[
            (0.2, [1.14127, 0.37082, -0.37082, 1.14127, 40.0, 0.0]),
            (0.35, [1.15106, 0.705373, -0.705373, 1.15106, 32.5, 0.0]),
            (0.8, [0.556231, 1.7119, -1.7119, 0.556231, 10.0, 0.0]),
        ],
    );
}

/// Chrome decomposes by Gram–Schmidt: a shear, not the residual 2×2 matrix of
/// the css-transforms-1 pseudo-code. The translation stays linear, sheared or
/// not.
#[test]
fn a_sheared_pair_with_translations_decomposes_like_chrome() {
    check_transition(
        "skewX(40deg) translate(10px, 5px)",
        "rotate(50deg) scale(1.5) translate(-20px, 30px)",
        &[
            (
                0.2,
                [1.08329, 0.191013, 0.536177, 1.21151, 0.605273, 5.18882],
            ),
            (
                0.5,
                [1.13288, 0.528273, -0.0529712, 1.35452, -19.7801, 5.47205],
            ),
            (
                0.8,
                [1.07246, 0.899903, -0.719922, 1.22348, -40.1654, 5.75529],
            ),
        ],
    );
}

/// Two reflections on opposite axes interpolate their scales straight through
/// zero — no half turn is inserted (the pseudo-code's sign swap is not what
/// Chrome does).
#[test]
fn opposite_reflections_interpolate_their_scales() {
    check_transition(
        "scale(-1, 1) skewX(0deg)",
        "matrix(1, 0, 0, -1, 0, 0)",
        &[
            (0.2, [-0.6, 0.0, 0.0, 0.6, 0.0, 0.0]),
            (0.8, [0.6, 0.0, 0.0, -0.6, 0.0, 0.0]),
        ],
    );
}

/// A half turn against the identity goes the way the angles are written —
/// 180° down to 0° — the quaternion slerp's tie.
#[test]
fn a_half_turn_decomposes_through_ninety_degrees() {
    check_transition(
        "matrix(-1, 0, 0, -1, 0, 0)",
        "matrix(1, 0, 0, 1, 0, 0) skewX(0deg)",
        &[
            (0.2, [-0.809017, 0.587785, -0.587785, -0.809017, 0.0, 0.0]),
            (0.8, [0.809017, 0.587785, -0.587785, 0.809017, 0.0, 0.0]),
        ],
    );
}

/// A singular matrix has no decomposition; Chrome flips from one end to the
/// other half way.
#[test]
fn a_singular_endpoint_flips_half_way() {
    check_transition(
        "matrix(0, 0, 0, 1, 0, 0)",
        "rotate(90deg)",
        &[
            (0.2, [0.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            (0.45, [0.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            (0.55, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
            (0.8, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
        ],
    );
}

// ── Starting and reversing ──

/// `rotate(0deg)` and `rotate(360deg)` compose to the same matrix, and they
/// are still different transforms: a full turn is the classic spinner, and
/// Chrome starts a transition for it.
#[test]
fn a_full_turn_is_a_change_and_spins() {
    check_transition(
        "rotate(0deg)",
        "rotate(360deg)",
        &[
            (0.2, [0.309017, 0.951057, -0.951057, 0.309017, 0.0, 0.0]),
            (0.35, [-0.587785, 0.809017, -0.809017, -0.587785, 0.0, 0.0]),
        ],
    );
}

/// Reversing half way: the new transition starts from the *interpolated*
/// `rotate(45deg)`, so it too goes by function. Chrome: 500ms back, and a
/// fifth of the way in (100ms) it is `rotate(36deg)`.
///
/// Style resolution stamps a transition with the wall clock, so the forward
/// transition is back-dated half its duration rather than ticked there: the
/// reversal measures its shortening against the clock too.
#[test]
fn a_reversal_starts_from_the_interpolated_function_list() {
    let now = || {
        web_time::SystemTime::now()
            .duration_since(web_time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            * 1000.0
    };
    let (mut doc, div) = transition_doc("rotate(0deg)", "rotate(90deg)");
    start(&mut doc, div, "t on");
    let back_dated = now() - 500.0;
    doc.tree
        .active_transitions
        .get_mut(&div.0)
        .and_then(|t| t.get_mut(&TransitionProperty::Transform))
        .unwrap()
        .start_time_ms = back_dated;
    rinch_dom::transition::tick_transitions(&mut doc.tree, back_dated + 500.0);
    assert_matrix(
        resolved(&doc, div),
        [0.707107, 0.707107, -0.707107, 0.707107, 0.0, 0.0],
        "half way there",
    );

    let t1 = start(&mut doc, div, "t");
    let reversal = &doc.tree.active_transitions[&div.0][&TransitionProperty::Transform];
    let duration = reversal.duration_ms;
    // The reversal starts from the style at the wall-clock moment the class
    // changed, a little past 45° under a loaded test run. What matters is its
    // *shape*: still a single `rotate()`, so it goes on pairing by function.
    let AnimatableValue::Transform(from) = &reversal.from else {
        panic!("a transform transition starts from a transform");
    };
    let [TransformOp::Rotate(from_rad)] = from.functions[..] else {
        panic!(
            "the reversal must start from the interpolated function list, got {:?}",
            from.functions
        );
    };
    let from_deg = from_rad.to_degrees();
    assert!(
        (from_deg - 45.0).abs() < 5.0 && (duration - 500.0).abs() < 60.0,
        "reversing half way: from about 45° over about half the duration, \
         got {from_deg}° over {duration}ms"
    );

    rinch_dom::transition::tick_transitions(&mut doc.tree, t1 + 0.2 * duration);
    // A fifth of the way back — Chrome 153 gives exactly `rotate(36deg)` from
    // 45°. The size is exact: it is what the elementwise lerp got wrong.
    let m = resolved(&doc, div);
    let det = m[0] * m[3] - m[1] * m[2];
    assert!(
        (det - 1.0).abs() < 1e-9,
        "a reversal of a rotation is still a rotation: {m:?}"
    );
    let angle = m[1].atan2(m[0]).to_degrees();
    assert!(
        // Progress is an `f32`, hence a thousandth of a degree.
        (angle - from_deg * 0.8).abs() < 1e-3,
        "a fifth of the way back from {from_deg}° is {}°, got {angle}°",
        from_deg * 0.8
    );
}

/// Equal lists are not a change: nothing starts.
#[test]
fn an_unchanged_function_list_starts_nothing() {
    let (mut doc, div) = transition_doc("rotate(30deg)", "rotate(30deg)");
    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree
            .active_transitions
            .get(&div.0)
            .is_none_or(|t| !t.contains_key(&TransitionProperty::Transform)),
        "an identical transform must not start a transition"
    );
}

/// `none` against `rotate(0deg)` starts nothing in rinch. Chrome does start a
/// transition there — invisibly — but in rinch a transitioning transform is a
/// stacking context for its whole run while `rotate(0deg)` at rest is not
/// (#415), so starting one would make the element's stacking flicker.
#[test]
fn none_against_a_zero_rotation_starts_nothing() {
    let (mut doc, div) = transition_doc("none", "rotate(0deg)");
    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree
            .active_transitions
            .get(&div.0)
            .is_none_or(|t| !t.contains_key(&TransitionProperty::Transform)),
        "none -> rotate(0deg) must not start a transition"
    );
}

/// Review of #983: `skewX` and `skew` do not pair either (Chrome 153,
/// `Element.animate` paused at `currentTime`). A mutant pairing them panics in
/// `interpolate_pair`'s `unreachable!` and no other fixture reaches it.
#[test]
fn skew_x_against_skew_decomposes() {
    let samples = [
        (0.2, [1.01037, 0.0706518, 0.493977, 1.0128, 0.0, 0.0]),
        (0.5, [1.01641, 0.17922, 0.371114, 1.01963, 0.0, 0.0]),
        (0.8, [1.01062, 0.289789, 0.252393, 1.01229, 0.0, 0.0]),
    ];
    check_transition("skewX(30deg)", "skew(10deg, 20deg)", &samples);
    check_keyframes("skewX(30deg)", "skew(10deg, 20deg)", &samples);
}

/// Review of #983: a singular endpoint flips *at* 0.5 to the end value —
/// Chrome 153 gives `matrix(1, 0, 0, 1, 0, 0)` at exactly half way. The
/// existing fixture samples 0.49/0.51 only, the fixed point either side of the
/// `<` / `<=` choice.
#[test]
fn a_singular_endpoint_is_the_end_value_at_exactly_half_way() {
    check_keyframes(
        "matrix(0, 0, 0, 0, 10, 10)",
        "matrix(1, 0, 0, 1, 0, 0)",
        &[(0.5, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0])],
    );
}
