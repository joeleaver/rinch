//! #405: the 3D transform functions, drawn flat.
//!
//! rinch paints in 2D. It used to flatten every 3D function except
//! `translate3d` to the **identity**, so `rotateZ(45deg)` — the same rotation
//! as `rotate(45deg)` — did not rotate, `scale3d(2, 0.5, 1)` did not scale and
//! `rotateX(60deg)` did not squash.
//!
//! What Chrome draws for an element with a 3D transform and no `perspective`
//! or `transform-style: preserve-3d` on an ancestor is the element's full 4×4
//! transform — its `perspective()` functions included — applied to the plane
//! `z = 0` and projected orthographically back onto the page: the resulting
//! `z` is dropped and nothing else is. So a list is composed as 4×4 matrices
//! and only the **composed** matrix is flattened. Flattening function by
//! function is wrong, which `rotateX(90deg) scaleY(2) rotateX(-90deg)` shows:
//! Chrome draws it unchanged, while each function's own flattening is
//! singular.
//!
//! **Every expected matrix below was measured in Chrome 153** on a 100×40 box
//! with `transform-origin: 0 0`: `getComputedStyle(el).transform` and
//! `getBoundingClientRect()`, the interpolated ones through an `Element.animate()`
//! effect paused at `currentTime`. A `matrix3d(m11, …, m44)` Chrome printed is
//! written here flattened, `[m11, m12, m21, m22, m41, m42] / m44`; where
//! Chrome printed a `matrix()` it is copied verbatim.
//!
//! One thing is **not** Chrome's, and has a fixture below saying so: a
//! projective transform (a `perspective()` acting on a rotation out of the
//! page, which draws a trapezoid) is drawn with its perspective's `x`/`y`
//! terms dropped.

// The expected matrices are Chrome's printed numbers, `0.707107` among them.
#![allow(clippy::approx_constant)]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::transition::TransitionProperty;

const BOX: &str = "position: absolute; left: 0; top: 0; width: 100px; height: 40px; \
                   transform-origin: 0 0;";

/// The element's transform with its percentage translate resolved against the
/// 100×40 border box.
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

/// `transform: css` on one box, laid out.
fn static_matrix(css: &str) -> [f64; 6] {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", &format!("{BOX} transform: {css};"));
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    resolved(&doc, div)
}

fn check_static(css: &str, want: [f64; 6]) {
    assert_matrix(static_matrix(css), want, css);
}

const I: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

// ---------------------------------------------------------------- tier 1

#[test]
fn translate3d_moves_by_its_x_and_y() {
    check_static(
        "translate3d(10px, 20px, 0)",
        [1.0, 0.0, 0.0, 1.0, 10.0, 20.0],
    );
    check_static(
        "translate3d(50%, 25%, 30px)",
        [1.0, 0.0, 0.0, 1.0, 50.0, 10.0],
    );
}

#[test]
fn translate_z_alone_moves_nothing() {
    check_static("translateZ(10px)", I);
}

// ---------------------------------------------------------------- tier 2

#[test]
fn rotate_z_is_rotate() {
    check_static(
        "rotateZ(45deg)",
        [0.707107, 0.707107, -0.707107, 0.707107, 0.0, 0.0],
    );
}

#[test]
fn rotate3d_about_the_z_axis_is_rotate_either_way_round() {
    check_static(
        "rotate3d(0, 0, 1, 30deg)",
        [0.866025, 0.5, -0.5, 0.866025, 0.0, 0.0],
    );
    // A negative, unnormalised axis turns the other way.
    check_static(
        "rotate3d(0, 0, -2, 30deg)",
        [0.866025, -0.5, 0.5, 0.866025, 0.0, 0.0],
    );
}

#[test]
fn scale3d_scales_x_and_y_and_scale_z_alone_is_nothing() {
    check_static("scale3d(2, 0.5, 3)", [2.0, 0.0, 0.0, 0.5, 0.0, 0.0]);
    check_static("scaleZ(3)", I);
}

#[test]
fn a_zero_axis_rotates_nothing() {
    check_static("rotate3d(0, 0, 0, 30deg)", I);
}

// ------------------------------------------------- tier 3: the flat projection

/// Chrome's rect for `rotateX(60deg)` is 100×20: the box seen edge-on at 60°.
#[test]
fn rotate_x_squashes_and_rotate_y_narrows() {
    check_static("rotateX(60deg)", [1.0, 0.0, 0.0, 0.5, 0.0, 0.0]);
    check_static("rotateY(60deg)", [0.5, 0.0, 0.0, 1.0, 0.0, 0.0]);
    check_static(
        "rotate3d(1, 1, 0, 60deg)",
        [0.75, 0.25, 0.25, 0.75, 0.0, 0.0],
    );
}

/// The list is composed in 3D and only the result flattened: flattening each
/// function first would give `scaleY(0) scaleY(2) scaleY(0)`, a line.
#[test]
fn a_list_is_flattened_after_it_is_composed() {
    check_static("rotateX(90deg) scaleY(2) rotateX(-90deg)", I);
    // A `translateZ` turned into the page by a rotation moves the box.
    check_static(
        "rotateY(90deg) translateZ(50px)",
        [0.0, 0.0, 0.0, 1.0, 50.0, 0.0],
    );
}

#[test]
fn a_percentage_translate_after_a_3d_rotation() {
    check_static(
        "rotateX(60deg) translate(50%, 50%)",
        [1.0, 0.0, 0.0, 0.5, 50.0, 10.0],
    );
}

#[test]
fn matrix3d_is_flattened() {
    check_static(
        "matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 10,20,30,1)",
        [1.0, 0.0, 0.0, 1.0, 10.0, 20.0],
    );
    // m44 divides the whole matrix.
    check_static(
        "matrix3d(2,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,2)",
        [1.0, 0.0, 0.0, 0.5, 0.0, 0.0],
    );
    check_static(
        "matrix3d(1,0.5,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1)",
        [1.0, 0.5, 0.0, 1.0, 0.0, 0.0],
    );
}

/// `perspective()` scales whatever a later `translateZ` moves toward the
/// viewer: Chrome's rect for the first is 111.1111×44.4444.
#[test]
fn perspective_magnifies_a_translate_toward_the_viewer() {
    let k = 1.0 / 0.9;
    check_static(
        "perspective(100px) translateZ(10px)",
        [k, 0.0, 0.0, k, 0.0, 0.0],
    );
    check_static(
        "perspective(100px) translateZ(10px) translate(10px, 5px)",
        [k, 0.0, 0.0, k, 11.1111, 5.55556],
    );
    // A percentage translate is divided by `w` like the rest of the matrix…
    check_static(
        "perspective(100px) translateZ(10px) translate(50%, 25%)",
        [k, 0.0, 0.0, k, 55.5556, 11.1111],
    );
    // …and one *before* the perspective is not magnified by it (Chrome's
    // rect starts at 150, 110 — the untransformed translate).
    check_static(
        "translate(50%, 25%) perspective(100px) translateZ(10px)",
        [k, 0.0, 0.0, k, 50.0, 10.0],
    );
    // A depth under 1px is clamped to 1px: w = 1 - 0.2 / 1.
    check_static(
        "perspective(0.5px) translateZ(0.2px)",
        [1.25, 0.0, 0.0, 1.25, 0.0, 0.0],
    );
    check_static("perspective(100px)", I);
    check_static("perspective(none) translateZ(10px)", I);
}

/// **Not Chrome's.** `perspective(200px) rotateY(30deg)` is projective: the far
/// edge is shorter than the near one, and Chrome's rect is 69.282×40. An affine
/// painter cannot draw a trapezoid, so rinch drops the perspective's `x`/`y`
/// terms (`m14`, `m24`) and draws `rotateY(30deg)` flat, 86.6025 wide.
#[test]
fn a_projective_transform_is_drawn_without_its_keystone() {
    check_static(
        "perspective(200px) rotateY(30deg)",
        [0.866025, 0.0, 0.0, 1.0, 0.0, 0.0],
    );
}

// ------------------------------------------------------------ interpolation

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

fn check_transition(from: &str, to: &str, samples: &[(f64, [f64; 6])]) {
    let (mut doc, div) = transition_doc(from, to);
    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(800.0, 600.0);
    let t0 = doc
        .tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::Transform))
        .unwrap_or_else(|| panic!("{from} -> {to} should start a transform transition"))
        .start_time_ms;
    for (f, want) in samples {
        rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + f * 1000.0);
        assert_matrix(
            resolved(&doc, div),
            *want,
            &format!("transition {from} -> {to} at {f}"),
        );
    }
}

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

const ROTATE_X_0_60: [(f64, [f64; 6]); 2] = [
    (0.2, [1.0, 0.0, 0.0, 0.978148, 0.0, 0.0]),
    (0.5, [1.0, 0.0, 0.0, 0.866025, 0.0, 0.0]),
];

/// A rotation out of the page interpolates its angle, so the squash follows
/// `cos`: half way is 0.866, not the 0.75 a lerp of the flattened matrices
/// gives.
#[test]
fn rotate_x_interpolates_its_angle() {
    check_transition("rotateX(0deg)", "rotateX(60deg)", &ROTATE_X_0_60);
}

#[test]
fn keyframes_rotate_x_interpolates_its_angle() {
    check_keyframes("rotateX(0deg)", "rotateX(60deg)", &ROTATE_X_0_60);
}

/// Every `rotate*` is one primitive, so `rotate` pairs with `rotateX`.
#[test]
fn rotate_pairs_with_rotate_x() {
    check_transition("rotate(0deg)", "rotateX(60deg)", &ROTATE_X_0_60);
    check_transition(
        "none",
        "rotateY(60deg)",
        &[(0.25, [0.965926, 0.0, 0.0, 1.0, 0.0, 0.0])],
    );
}

#[test]
fn rotate_z_pairs_with_rotate_and_goes_the_way_it_is_written() {
    check_transition(
        "rotateZ(0deg)",
        "rotate(90deg)",
        &[
            (0.2, [0.951057, 0.309017, -0.309017, 0.951057, 0.0, 0.0]),
            (0.5, [0.707107, 0.707107, -0.707107, 0.707107, 0.0, 0.0]),
        ],
    );
    // 170° to -170° goes through 0°, the long way, as `rotate` does.
    check_transition(
        "rotate3d(0, 0, 1, 170deg)",
        "rotateZ(-170deg)",
        &[
            (0.25, [0.0871557, 0.996195, -0.996195, 0.0871557, 0.0, 0.0]),
            (0.5, I),
        ],
    );
    check_transition(
        "rotate(0deg)",
        "rotate3d(0, 0, -1, 90deg)",
        &[(0.2, [0.951057, -0.309017, 0.309017, 0.951057, 0.0, 0.0])],
    );
}

/// Two rotations about different axes are interpolated as quaternions.
#[test]
fn rotations_about_different_axes_slerp() {
    check_transition(
        "rotateX(60deg)",
        "rotateY(60deg)",
        &[
            (0.2, [0.976287, 0.0899669, 0.0899669, 0.658663, 0.0, 0.0]),
            (0.5, [0.857143, 0.142857, 0.142857, 0.857143, 0.0, 0.0]),
        ],
    );
    check_transition(
        "rotate(90deg)",
        "rotateX(90deg)",
        &[(0.3, [0.403019, 0.872678, -0.872678, 0.275697, 0.0, 0.0])],
    );
    check_transition(
        "rotate3d(1, 1, 0, 0deg)",
        "rotate3d(1, 1, 0, 90deg)",
        &[(0.35, [0.92632, 0.0736799, 0.0736799, 0.92632, 0.0, 0.0])],
    );
}

/// A common axis interpolates the angle, whatever length the axis is written
/// with — so a turn goes the way it is written, through 0°, where a slerp
/// would take the short way through 180°.
#[test]
fn rotations_about_one_axis_interpolate_their_angle() {
    check_transition(
        "rotateX(-170deg)",
        "rotateX(170deg)",
        &[(0.25, [1.0, 0.0, 0.0, 0.0871557, 0.0, 0.0])],
    );
    check_transition(
        "rotate3d(1, 2, 0, 20deg)",
        "rotate3d(2, 4, 0, 80deg)",
        &[(0.5, [0.71423, 0.142885, 0.142885, 0.928558, 0.0, 0.0])],
    );
}

/// Two quaternions whose dot product is negative: Chrome's slerp negates one
/// and takes the shorter arc, which css-transforms-2's pseudo-code does not.
#[test]
fn a_slerp_takes_the_shorter_arc() {
    check_transition(
        "rotateX(300deg)",
        "rotateY(60deg)",
        &[(0.3, [0.94711, -0.119144, -0.119144, 0.731607, 0.0, 0.0])],
    );
}

#[test]
fn translate_z_pairs_with_translate() {
    check_transition(
        "translateZ(10px)",
        "translateX(20px)",
        &[
            (0.2, [1.0, 0.0, 0.0, 1.0, 4.0, 0.0]),
            (0.5, [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
        ],
    );
    // The z that is interpolated is turned into x by the rotation before it.
    check_transition(
        "rotateY(90deg) translateZ(50px)",
        "rotateY(0deg) translateZ(50px)",
        &[(0.5, [0.707107, 0.0, 0.0, 1.0, 35.3553, 0.0])],
    );
    check_transition(
        "rotateY(90deg) translateZ(0px)",
        "rotateY(90deg) translateZ(40px)",
        &[(0.25, [0.0, 0.0, 0.0, 1.0, 10.0, 0.0])],
    );
}

#[test]
fn scale3d_pairs_with_scale() {
    check_transition(
        "scale3d(1, 1, 1)",
        "scale(2)",
        &[
            (0.2, [1.2, 0.0, 0.0, 1.2, 0.0, 0.0]),
            (0.5, [1.5, 0.0, 0.0, 1.5, 0.0, 0.0]),
        ],
    );
    check_transition(
        "scaleZ(2)",
        "scaleX(3)",
        &[(0.35, [1.7, 0.0, 0.0, 1.0, 0.0, 0.0])],
    );
}

#[test]
fn percentage_translates_interpolate_in_a_3d_frame() {
    check_transition(
        "rotateX(10deg) translate(10%, 5px)",
        "rotateX(50deg) translate(30%, 15px)",
        &[(0.35, [1.0, 0.0, 0.0, 0.913545, 17.0, 7.76514])],
    );
    check_transition(
        "rotateX(20deg) translate(10px, 25%)",
        "rotateX(40deg) translate(30px, 50%)",
        &[(0.5, [1.0, 0.0, 0.0, 0.866025, 20.0, 12.9904])],
    );
}

/// Chrome interpolates `perspective()` by its matrix entry `-1/d`, not by `d`.
#[test]
fn perspective_interpolates_its_reciprocal() {
    check_transition(
        "perspective(100px) translateZ(10px)",
        "perspective(200px) translateZ(10px)",
        &[
            (0.2, [1.098901, 0.0, 0.0, 1.098901, 0.0, 0.0]),
            (0.5, [1.081081, 0.0, 0.0, 1.081081, 0.0, 0.0]),
        ],
    );
    // `none` pads with `perspective(none)`, whose reciprocal is 0.
    check_transition(
        "none",
        "perspective(100px) translateZ(10px)",
        &[(0.5, [1.025641, 0.0, 0.0, 1.025641, 0.0, 0.0])],
    );
    check_transition(
        "perspective(100px) translateZ(10px) rotate(10deg)",
        "perspective(100px) translateZ(20px) rotate(50deg)",
        &[(0.5, [1.018853, 0.588235, -0.588235, 1.018853, 0.0, 0.0])],
    );
}

/// `matrix3d()` pairs with `matrix3d()` — and, in Chrome, not with `matrix()`.
#[test]
fn matrix3d_pairs_with_matrix3d_only() {
    let to = "matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 0,20,0,1) translate(3px)";
    check_transition(
        "matrix3d(2,0,0,0, 0,1,0,0, 0,0,1,0, 10,0,0,1) translate(1px)",
        to,
        &[
            (0.2, [1.8, 0.0, 0.0, 1.0, 10.52, 4.0]),
            (0.5, [1.5, 0.0, 0.0, 1.0, 8.0, 10.0]),
        ],
    );
    // Unpaired, the whole of each list is decomposed: the translate(2px) a
    // pairing would scale by 1.5 is not there, so e is 7.5 rather than 8.
    check_transition(
        "matrix(2, 0, 0, 1, 10, 0) translate(1px)",
        to,
        &[(0.5, [1.5, 0.0, 0.0, 1.0, 7.5, 10.0])],
    );
}

#[test]
fn a_mixed_3d_pair_interpolates_by_function() {
    check_transition(
        "rotateZ(30deg) translateZ(5px) scale3d(2, 1, 4)",
        "rotate(90deg) translate(10px, 4px) scale(1, 3)",
        &[(
            0.25,
            [1.23744, 1.23744, -1.06066, 1.06066, 1.06066, 2.47487],
        )],
    );
}

/// #989: a pair of lists that stops pairing is composed and interpolated by
/// **decomposition**, and when either remainder is not a 2D matrix the
/// decomposition is css-transforms-2's 3D one (translation, scale, three
/// shears, perspective, a quaternion slerped the shorter way), as Chrome does.
/// It used to be the flattened 2D matrices', which see only a y scale of 0.5
/// in `rotateX(60deg)` and lerp it (0.75 half way, where Chrome recovers the
/// rotation and draws `rotateX(30deg)`, 0.866025). The numbers from here to
/// `keyframes_carry_a_translate_z_and_a_perspective` were measured in **Chrome
/// 155**, the same way as the rest.
#[test]
fn a_mismatched_3d_pair_decomposes_in_3d() {
    check_transition(
        "rotateX(60deg)",
        "translate(10px)",
        &[
            (0.2, [1.0, 0.0, 0.0, 0.669131, 2.0, 0.0]),
            (0.5, [1.0, 0.0, 0.0, 0.866025, 5.0, 0.0]),
            (0.8, [1.0, 0.0, 0.0, 0.978148, 8.0, 0.0]),
        ],
    );
    check_keyframes(
        "rotateX(60deg)",
        "translate(10px)",
        &[(0.2, [1.0, 0.0, 0.0, 0.669131, 2.0, 0.0])],
    );
    // A rotation out of the page against one in it: one quaternion slerp.
    check_transition(
        "rotateY(150deg)",
        "rotate(170deg) translate(5px)",
        &[
            (
                0.3,
                [-0.856529, 0.239101, -0.239101, 0.601527, -1.28479, 0.358651],
            ),
            (
                0.5,
                [
                    -0.882942, 0.337055, -0.337055, 0.0294885, -2.20735, 0.842638,
                ],
            ),
        ],
    );
}

/// A 4×4 does not remember which way it was turned: `rotateX(300deg)` is
/// decomposed as `rotateX(-60deg)`, so it comes back to the plane through
/// `rotateX(330deg)`, not through `rotateX(150deg)` as a function pair would.
#[test]
fn a_3d_decomposition_forgets_the_turns_direction() {
    check_transition(
        "rotateX(300deg)",
        "translate(10px)",
        &[
            (0.2, [1.0, 0.0, 0.0, 0.669131, 2.0, 0.0]),
            (0.5, [1.0, 0.0, 0.0, 0.866025, 5.0, 0.0]),
        ],
    );
}

/// A 3D end that reflects is decomposed with every scale negated and a half
/// turn in its rotation, the 3D decomposition's rule.
#[test]
fn a_3d_reflection_negates_every_scale() {
    check_transition(
        "scaleX(-1) rotateX(60deg)",
        "translate(10px)",
        &[
            (0.2, [-0.6, 0.0, 0.0, 0.0627171, 2.0, 0.0]),
            (0.4, [-0.2, 0.0, 0.0, -0.0618034, 4.0, 0.0]),
        ],
    );
}

/// The interpolated remainder keeps its z: a `translateZ` interpolated after a
/// paired `rotateY` is turned into x by it, where a remainder flattened before
/// the prefix met it had lost the z (0.6 and 1.2 are the scale; 34.641 is the
/// z of 40px times `sin 60°`).
#[test]
fn a_remainder_keeps_its_z_for_the_prefix() {
    check_transition(
        "rotateY(60deg) translateZ(50px)",
        "rotateY(60deg) scale(2)",
        &[
            (0.2, [0.6, 0.0, 0.0, 1.2, 34.641, 0.0]),
            (0.5, [0.75, 0.0, 0.0, 1.5, 21.6506, 0.0]),
        ],
    );
}

/// A paired `matrix3d()` that is not a 2D matrix is decomposed in 3D too.
#[test]
fn a_matrix3d_pair_decomposes_in_3d() {
    check_transition(
        "matrix3d(1,0,0,0, 0,0.5,0.866025,0, 0,-0.866025,0.5,0, 0,0,0,1)",
        "matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 10,0,0,1)",
        &[
            (0.2, [1.0, 0.0, 0.0, 0.669131, 2.0, 0.0]),
            (0.5, [1.0, 0.0, 0.0, 0.866025, 5.0, 0.0]),
        ],
    );
}

/// Each end is decomposed on its own, and a 2D end by the 2D decomposition: a
/// reflection is one negative scale (the x scale when `a < d`, else the y
/// one), not the 3D decomposition's three negated scales and a half turn. So
/// `scaleX(-1)` against `rotateX(60deg)` passes through an x scale of 0. (Not
/// sampled at that point: a singular 4×4 is drawn as nothing, #1051, while
/// Chrome's computed value still prints it.)
#[test]
fn a_2d_end_is_decomposed_in_2d() {
    check_transition(
        "scaleX(-1)",
        "rotateX(60deg)",
        &[
            (0.2, [-0.6, 0.0, 0.0, 0.978148, 0.0, 0.0]),
            (0.8, [0.6, 0.0, 0.0, 0.669131, 0.0, 0.0]),
        ],
    );
    check_transition(
        "scaleY(-1) translate(5px)",
        "rotateY(60deg)",
        &[(0.3, [0.951057, 0.0, 0.0, -0.4, 3.5, 0.0])],
    );
}

/// The shear and perspective parts of the 3D decomposition.
#[test]
fn a_3d_decomposition_interpolates_shear_and_perspective() {
    check_transition(
        "rotateX(60deg) skewX(20deg)",
        "scale(2, 0.5)",
        &[
            (0.2, [1.2, 0.0, 0.262059, 0.602218, 0.0, 0.0]),
            (0.5, [1.5, 0.0, 0.136489, 0.649519, 0.0, 0.0]),
        ],
    );
    check_transition(
        "perspective(100px) translateZ(10px) rotateX(30deg)",
        "rotate(30deg)",
        &[
            (0.2, [1.06387, 0.110544, -0.110544, 0.970628, 0.0, 0.0]),
            (0.5, [0.991438, 0.265654, -0.265654, 0.955843, 0.0, 0.0]),
        ],
    );
}

/// A percentage translate interpolates linearly beside the decomposition, as
/// in 2D: `translate(50%, 25%)` of the 100×40 box is (50, 10) at the start.
#[test]
fn a_percentage_translate_rides_a_3d_decomposition() {
    check_transition(
        "translate(50%, 25%) rotateX(60deg)",
        "scale(2) rotate(10deg)",
        &[
            (0.2, [1.1992, 0.0399823, -0.0399823, 0.801819, 40.0, 8.0]),
            (0.5, [1.49388, 0.130698, -0.130698, 1.29257, 25.0, 5.0]),
        ],
    );
    // Behind a perspective, which the box-relative translation must follow.
    check_transition(
        "perspective(100px) translate(50%, 25%) rotateX(30deg)",
        "rotate(30deg)",
        &[
            (0.2, [0.994402, 0.103325, -0.103325, 0.907242, 40.0, 8.0]),
            (0.5, [0.965346, 0.258664, -0.258664, 0.930691, 25.0, 5.0]),
        ],
    );
    // With a z translation behind the perspective the decomposed perspective
    // has a `w` other than 1, so the translation must come after it, not in
    // front of it (which would scale it by that `w`).
    check_transition(
        "perspective(100px) translate(50%, 25%) translateZ(10px) rotateX(30deg)",
        "rotate(30deg)",
        &[
            (
                0.2,
                [1.06387, 0.110544, -0.110544, 0.970628, 43.668, 8.73361],
            ),
            (
                0.5,
                [0.991438, 0.265654, -0.265654, 0.955843, 27.027, 5.4054],
            ),
        ],
    );
}

/// `scaleY(0.5)` and `rotateX(60deg)` flatten to the same matrix, but they are
/// different transforms and Chrome runs a transition between them, which
/// leaves the plane (half way it is `rotateX(30deg)` at a y scale of 0.75).
/// "Holds still" is asked of the 4×4s, not of their flattenings.
#[test]
fn equal_flattenings_of_different_3d_transforms_transition() {
    check_transition(
        "scaleY(0.5)",
        "rotateX(60deg)",
        &[
            (0.2, [1.0, 0.0, 0.0, 0.586889, 0.0, 0.0]),
            (0.5, [1.0, 0.0, 0.0, 0.649519, 0.0, 0.0]),
        ],
    );
}

/// An end with a singular 3×3 has no decomposition, and the pair flips from
/// one end to the other half way, as in 2D.
#[test]
fn a_singular_3d_end_flips_half_way() {
    check_transition(
        "rotateX(60deg) scale(0)",
        "translate(10px)",
        &[
            (0.3, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            (0.7, [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
        ],
    );
}

/// `@keyframes` stops are extracted from the specified value by a converter
/// of their own, so the z of a translate and a perspective depth need pins of
/// their own there.
#[test]
fn keyframes_carry_a_translate_z_and_a_perspective() {
    check_keyframes(
        "perspective(100px) translateZ(10px)",
        "perspective(200px) translateZ(10px)",
        &[(0.2, [1.098901, 0.0, 0.0, 1.098901, 0.0, 0.0])],
    );
    check_keyframes(
        "rotateY(90deg) translateZ(50px)",
        "rotateY(0deg) translateZ(50px)",
        &[(0.5, [0.707107, 0.0, 0.0, 1.0, 35.3553, 0.0])],
    );
    check_keyframes(
        "rotateX(60deg)",
        "rotateY(60deg)",
        &[(0.2, [0.976287, 0.0899669, 0.0899669, 0.658663, 0.0, 0.0])],
    );
}

/// A rotation of zero angle has no axis of its own, so the pair interpolates
/// about the other one — the long way, as written: `rotateY(0deg)` →
/// `rotateX(300deg)` half way is `rotateX(150deg)` in Chrome 153, where a
/// slerp would take the short way to `rotateX(-30deg)`.
#[test]
fn a_zero_angle_end_lerps_about_the_other_axis() {
    let want = [(0.5, [1.0, 0.0, 0.0, -0.866025, 0.0, 0.0])];
    check_transition("rotateY(0deg)", "rotateX(300deg)", &want);
    check_transition("rotateX(300deg)", "rotateY(0deg)", &want);
    check_keyframes("rotateY(0deg)", "rotateX(300deg)", &want);
}

/// A `scaleZ` only shows once a later `translateZ` meets a `perspective()`:
/// Chrome 153's `perspective(100px) scaleZ(2) translateZ(10px)` is
/// `matrix3d(…, 20, 0.8)`, a scale of 1.25. Pinned through the `@keyframes`
/// converter too, whose `scaleZ`, `scale3d` and `matrix3d` arms nothing else
/// reaches.
#[test]
fn a_scale_z_meets_a_perspective() {
    let s = [1.25, 0.0, 0.0, 1.25, 0.0, 0.0];
    check_static("perspective(100px) scaleZ(2) translateZ(10px)", s);
    check_keyframes(
        "perspective(100px) scaleZ(2) translateZ(10px)",
        "none",
        &[(0.0, s)],
    );
    check_keyframes(
        "perspective(100px) scale3d(3, 0.5, 2) translateZ(10px)",
        "none",
        &[(0.0, [3.75, 0.0, 0.0, 0.625, 0.0, 0.0])],
    );
    check_keyframes(
        "matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,-0.01, 0,0,0,1) translateZ(10px)",
        "none",
        &[(0.0, [1.0 / 0.9, 0.0, 0.0, 1.0 / 0.9, 0.0, 0.0])],
    );
    check_keyframes(
        "perspective(100px) scaleZ(1) translateZ(10px)",
        "perspective(100px) scaleZ(3) translateZ(10px)",
        &[(0.5, s)],
    );
}

/// An element whose plane is at or behind the viewer (`w <= 0`) is neither
/// drawn nor hit in Chrome 153 — measured on a box at (200, 100), 100×40:
/// every pixel stays white and `elementFromPoint` answers the body. Dividing by
/// a negative `w` drew it point-mirrored instead (20× magnified for
/// `translateZ(10.5px)`) and let it swallow clicks, so a "fly through the
/// screen" animation became a giant inverted copy as it passed the viewer.
#[test]
fn an_element_behind_the_viewer_is_neither_drawn_nor_hit() {
    let mut bad = vec![];
    for tf in [
        "perspective(10px) translateZ(20px)",
        "perspective(10px) translateZ(10px)",
        "perspective(10px) translateZ(10.5px)",
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            &format!(
                "position: absolute; left: 200px; top: 100px; width: 100px; height: 40px; \
                 background: red; transform-origin: 0 0; transform: {tf};"
            ),
        );
        doc.append_child(body, div);
        doc.resolve_layout(400.0, 300.0);
        let mut painter = TinySkiaPainter::new(400, 300);
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (400.0, 300.0),
            &mut doc.font_cx,
            &mut lcx,
        );
        let red = painter
            .pixels()
            .chunks(4)
            .filter(|p| p[0] > 0 && p[3] > 0)
            .count();
        let hit = [(150.0, 80.0), (250.0, 120.0)].iter().any(|&(x, y)| {
            rinch_dom::paint::point_in_painted_box(&doc.tree, div.0, 1.0, x, y)
                .is_some_and(|(lx, ly)| (0.0..100.0).contains(&lx) && (0.0..40.0).contains(&ly))
        });
        if red > 0 || hit {
            bad.push(format!("{tf}: {red} red px, hit={hit}"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

// ------------------------------------ transform-origin's z, backface-visibility

/// One red 100×40 box at `left`/`top` with `style` on top, laid out in an
/// 800×600 viewport, plus an optional blue 50×20 child with `child` as its
/// style.
struct Scene {
    doc: RinchDocument,
    parent: NodeId,
    child: Option<NodeId>,
}

fn scene(left: f64, top: f64, style: &str, child: Option<&str>) -> Scene {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = doc.create_element("div");
    doc.set_attribute(
        parent,
        "style",
        &format!(
            "position: absolute; left: {left}px; top: {top}px; width: 100px; height: 40px; \
             background: red; {style}"
        ),
    );
    doc.append_child(body, parent);
    let child = child.map(|css| {
        let c = doc.create_element("div");
        doc.set_attribute(
            c,
            "style",
            &format!("width: 50px; height: 20px; background: blue; {css}"),
        );
        doc.append_child(parent, c);
        c
    });
    doc.resolve_layout(800.0, 600.0);
    Scene { doc, parent, child }
}

impl Scene {
    /// The painted bounding box of `id`: `(x, y, width, height)`.
    fn rect(&self, id: NodeId) -> [f64; 4] {
        let r = rinch_dom::paint::painted_border_box(&self.doc.tree, id.0, 1.0);
        [r.x0, r.y0, r.width(), r.height()]
    }

    /// Pixels painted `(red, blue)`.
    fn painted(&mut self) -> (usize, usize) {
        let mut painter = TinySkiaPainter::new(800, 600);
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &self.doc.tree,
            &mut painter,
            1.0,
            (800.0, 600.0),
            &mut self.doc.font_cx,
            &mut lcx,
        );
        let px = painter.pixels();
        let red = px
            .chunks(4)
            .filter(|p| p[3] > 0 && p[0] > 128 && p[2] < 64)
            .count();
        let blue = px
            .chunks(4)
            .filter(|p| p[3] > 0 && p[2] > 128 && p[0] < 64)
            .count();
        (red, blue)
    }

    /// Whether the point `(x, y)` lands inside `id`'s own box.
    fn hits(&self, id: NodeId, x: f64, y: f64) -> bool {
        let (w, h) = {
            let l = self.doc.tree.nodes[id.0].layout;
            (l.width as f64, l.height as f64)
        };
        rinch_dom::paint::point_in_painted_box(&self.doc.tree, id.0, 1.0, x, y)
            .is_some_and(|(lx, ly)| (0.0..w).contains(&lx) && (0.0..h).contains(&ly))
    }
}

fn assert_rect(got: [f64; 4], want: [f64; 4], what: &str) {
    let close = got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.01);
    assert!(close, "{what}: got {got:?}, Chrome 153 gives {want:?}");
}

/// `transform-origin: <x> <y> <z>` moves a rotation out of the page by its
/// z (#997). Every rect is Chrome 153's `getBoundingClientRect()` of a 100×40
/// box; rinch dropped the z and drew each at its `z = 0` origin — the first at
/// x = 214.64, which is where the issue started.
#[test]
fn transform_origin_z_moves_a_rotation_out_of_the_page() {
    for (left, top, style, want) in [
        (
            200.0,
            100.0,
            "transform: rotateY(45deg); transform-origin: 50% 50% 100px",
            [143.934, 100.0, 70.711, 40.0],
        ),
        (
            200.0,
            100.0,
            "transform: rotateY(45deg); transform-origin: 0 0 100px",
            [129.289, 100.0, 70.711, 40.0],
        ),
        (
            600.0,
            100.0,
            "transform: rotateY(30deg); transform-origin: 10px 20px -40px",
            [621.340, 100.0, 86.603, 40.0],
        ),
        (
            400.0,
            300.0,
            "transform: rotateX(60deg); transform-origin: 0 0 30px",
            [400.0, 325.981, 100.0, 20.0],
        ),
        // The z meets a perspective(): the plane sits 10px further back.
        (
            200.0,
            0.0,
            "transform: perspective(100px) translateZ(10px); transform-origin: 0 0 20px",
            [200.0, 0.0, 90.909, 36.364],
        ),
        // A transform that stays in the page is not moved by it.
        (
            600.0,
            200.0,
            "transform: translateX(10px); transform-origin: 10px 20px -40px",
            [610.0, 200.0, 100.0, 40.0],
        ),
        (
            200.0,
            200.0,
            "transform: rotate(30deg); transform-origin: 0 0 50px",
            [180.0, 200.0, 106.603, 84.641],
        ),
    ] {
        let s = scene(left, top, style, None);
        assert_rect(s.rect(s.parent), want, style);
    }
}

/// A transition's frames compose with the origin's z too, and a frame that
/// turns the back of a `backface-visibility: hidden` box to the viewer is
/// neither drawn nor hit — Chrome 153, an `Element.animate()` of
/// `rotateY(0deg)` → `rotateY(180deg)` paused at 40% and at 60%: the first is
/// at x = 104.894 and hit, the second is not hit.
#[test]
fn a_transition_frame_takes_the_origin_z_and_the_backface() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(
        ".t { position: absolute; left: 200px; top: 0; width: 100px; height: 40px; \
         background: red; backface-visibility: hidden; transform-origin: 0 0 100px; \
         transition: transform 1000ms linear; transform: rotateY(0deg); } \
         .t.on { transform: rotateY(180deg); }",
    );
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "t");
    doc.append_child(body, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(800.0, 600.0);
    let t0 = doc
        .tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::Transform))
        .expect("a transform transition")
        .start_time_ms;
    let mut s = Scene {
        doc,
        parent: div,
        child: None,
    };

    rinch_dom::transition::tick_transitions(&mut s.doc.tree, t0 + 400.0);
    assert_rect(s.rect(div), [104.894, 0.0, 30.902, 40.0], "at 40%");
    assert!(s.painted().0 > 0, "at 40% the front faces the viewer");
    assert!(s.hits(div, 120.0, 20.0), "at 40% the box is hit");

    rinch_dom::transition::tick_transitions(&mut s.doc.tree, t0 + 600.0);
    assert_eq!(s.painted().0, 0, "at 60% the back faces the viewer");
    assert!(!s.hits(div, 89.0, 20.0), "at 60% nothing is hit");
}

/// `backface-visibility: hidden` hides a box whose own transform turns its
/// back to the viewer, and only then (#997). Chrome 153's `elementFromPoint`
/// at each box's centre: `rotateY(180deg)` and `rotateY(100deg)` are not hit,
/// while `rotateY(80deg)`, `scaleX(-1)` — a mirror, not a turn — and
/// `rotateX(180deg) rotateY(180deg)`, which is `rotate(180deg)`, are.
#[test]
fn a_hidden_backface_is_neither_drawn_nor_hit() {
    let mut bad = vec![];
    for (tf, shown) in [
        ("rotateY(180deg)", false),
        ("rotateY(100deg)", false),
        ("rotateX(180deg)", false),
        ("rotateY(180deg); transform-origin: 50% 50% 60px", false),
        ("rotateY(80deg)", true),
        ("scaleX(-1)", true),
        ("scale(-1, -1)", true),
        ("rotateX(180deg) rotateY(180deg)", true),
        ("rotateY(360deg)", true),
    ] {
        let mut s = scene(
            200.0,
            100.0,
            &format!("transform: {tf}; backface-visibility: hidden"),
            None,
        );
        let (red, _) = s.painted();
        let hit = s.hits(s.parent, 250.0, 120.0);
        if (red > 0, hit) != (shown, shown) {
            bad.push(format!("{tf}: {red} red px, hit={hit}, want shown={shown}"));
        }
        // The same box with its backface visible is always drawn.
        let mut v = scene(200.0, 100.0, &format!("transform: {tf}"), None);
        let (red, _) = v.painted();
        if red == 0 || !v.hits(v.parent, 250.0, 120.0) {
            bad.push(format!("{tf} (visible backface): {red} red px"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The box is hidden with everything in it — a child turned back to face the
/// viewer included — and it is the box's **own** transform that decides: a
/// child with a hidden backface under a turned parent, with no transform of its
/// own, is drawn. Chrome 153: the first child is not hit, the second is.
/// (A `position: fixed` descendant is the exception, still drawn for want of
/// containment — #1372; `repaint_old_rect_tests` pins it.)
#[test]
fn a_hidden_backface_hides_the_subtree_and_reads_only_the_own_transform() {
    let mut s = scene(
        400.0,
        200.0,
        "transform: rotateY(180deg); backface-visibility: hidden",
        Some("transform: rotateY(180deg)"),
    );
    let child = s.child.unwrap();
    assert_eq!(s.painted(), (0, 0), "a hidden parent hides its child");
    assert!(!s.hits(child, 475.0, 210.0));

    let mut s = scene(
        400.0,
        100.0,
        "transform: rotateY(180deg)",
        Some("backface-visibility: hidden"),
    );
    let child = s.child.unwrap();
    assert_rect(s.rect(child), [450.0, 100.0, 50.0, 20.0], "the child");
    let (red, blue) = s.painted();
    assert!(red > 0 && blue > 0, "{red} red px, {blue} blue px");
    assert!(s.hits(child, 475.0, 110.0));
}

/// Chrome 153 draws and hits `scale(0.01) rotateY(180deg)` and
/// `scale(0.015) …` with a hidden backface — its threshold is float epsilon on
/// `cofactor33 · det`, which is `−s⁴` here — and hides `scale(0.02) …`.
#[test]
fn a_tiny_turned_box_is_judged_with_chromes_epsilon() {
    let mut bad = vec![];
    for (s, w, h, shown) in [
        (0.01, 10000.0, 4000.0, true),
        (0.015, 6667.0, 2667.0, true),
        (0.02, 5000.0, 2000.0, false),
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let el = doc.create_element("div");
        doc.set_attribute(
            el,
            "style",
            &format!(
                "position: absolute; left: 200px; top: 100px; width: {w}px; height: {h}px; \
                 background: red; transform-origin: 0 0; backface-visibility: hidden; \
                 transform: scale({s}) rotateY(180deg) translateX(-{w}px)"
            ),
        );
        doc.append_child(body, el);
        doc.resolve_layout(800.0, 600.0);
        let back = doc.tree.nodes[el.0].computed_style.transform.back_facing;
        if back == shown {
            bad.push(format!(
                "scale({s}): back_facing={back}, Chrome shown={shown}"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

// ------------------------------------------------------ a singular 4×4 (#1051)

/// A box whose 4×4 transform is singular is neither drawn nor hit, even where
/// its flattening onto `z = 0` is invertible — `scaleZ(0)` flattens to the
/// identity (#1051). Measured in Chrome 153 on a red 100×40 box, screenshot
/// pixels and `elementFromPoint` at its centre: every row marked `false` paints
/// no pixel and is not hit, subtree included, whatever its
/// `backface-visibility`. (Its `getBoundingClientRect()` is still the
/// flattened rect; rinch's painted rect is not, as for a plane behind the
/// viewer.)
///
/// Chrome's cut is the determinant against the smallest normal `f32`:
/// `scaleZ(2.5e-38)` is drawn and `scaleZ(1e-38)` — or `scaleZ(1e-20)` twice —
/// is not; a determinant that is merely small (`scaleZ(0.001)`,
/// `scaleZ(1e-20)`, a near-singular rotation sandwich) is drawn. Chrome's
/// `elementFromPoint` is unreliable on the near-singular drawn rows
/// (`rotateY(20deg) scaleZ(1e-12)` is drawn and not hit), so only the rows it
/// does hit are asserted hit.
#[test]
fn a_singular_transform_is_neither_drawn_nor_hit() {
    let mut bad = vec![];
    // (transform, drawn, hit asserted)
    for (tf, drawn, hit_too) in [
        ("scaleZ(0)", false, false),
        ("scaleZ(-0)", false, false),
        ("scale3d(1, 1, 0)", false, false),
        ("scale3d(0.5, 0.5, 0)", false, false),
        ("rotateY(180deg) scaleZ(0)", false, false),
        (
            "rotateY(180deg) scaleZ(0); backface-visibility: hidden",
            false,
            false,
        ),
        ("rotateX(45deg) scaleZ(0)", false, false),
        ("scaleZ(0) rotateX(45deg)", false, false),
        ("rotateY(20deg) scaleZ(0)", false, false),
        ("rotateX(30deg) scaleZ(0) rotateY(20deg)", false, false),
        // Row 0 reaches every column, so the Laplace signs matter (review of #1055).
        (
            "rotate3d(1,2,3,37deg) scaleZ(0) rotate3d(3,1,2,53deg)",
            false,
            false,
        ),
        ("matrix3d(1,0,0,0, 0,1,0,0, 0,0,0,0, 0,0,0,1)", false, false),
        ("matrix3d(1,0,0,0, 0,1,0,0, 1,0,0,0, 0,0,0,1)", false, false),
        ("perspective(100px) scaleZ(0)", false, false),
        ("translateZ(10px) scaleZ(0)", false, false),
        ("scaleZ(0); transform-origin: 50% 50% 30px", false, false),
        ("scaleZ(1e-20) scaleZ(1e-20)", false, false),
        ("scaleZ(1e-38)", false, false),
        // Drawn: invertible, however nearly singular.
        ("scaleZ(0.001)", true, true),
        ("scaleZ(1e-20)", true, true),
        ("scaleZ(1e-37)", true, true),
        ("scaleZ(2.5e-38)", true, false),
        ("scaleZ(-1)", true, true),
        ("rotateX(30deg) scaleZ(1e-20) rotateY(20deg)", true, false),
        ("rotateY(20deg) scaleZ(1e-12)", true, false),
    ] {
        let mut s = scene(200.0, 100.0, &format!("transform: {tf}"), None);
        let (red, _) = s.painted();
        let hit = s.hits(s.parent, 250.0, 120.0);
        if (red > 0) != drawn || (hit_too && !hit) || (!drawn && hit) {
            bad.push(format!(
                "{tf}: {red} red px, hit={hit}, Chrome drawn={drawn}"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A singular box hides its whole subtree, a child with a transform of its own
/// that would undo it included — Chrome 153 paints neither the red parent nor
/// the blue child, and hits neither.
#[test]
fn a_singular_transform_hides_the_subtree() {
    for child in ["", "transform: scaleZ(1000)"] {
        let mut s = scene(400.0, 200.0, "transform: scaleZ(0)", Some(child));
        let c = s.child.unwrap();
        assert_eq!(s.painted(), (0, 0), "child `{child}`");
        assert!(!s.hits(c, 425.0, 210.0), "child `{child}` is not hit");
        assert!(!s.hits(s.parent, 450.0, 220.0));
    }
}
