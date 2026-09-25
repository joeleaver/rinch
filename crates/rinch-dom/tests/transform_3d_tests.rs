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
//! Two things are **not** Chrome's, and each has a fixture below saying so:
//! a projective transform (a `perspective()` acting on a rotation out of the
//! page, which draws a trapezoid) is drawn with its perspective's `x`/`y`
//! terms dropped, and a mismatched pair of lists is decomposed as flattened 2D
//! matrices where Chrome decomposes in 3D (#989).

// The expected matrices are Chrome's printed numbers, `0.707107` among them.
#![allow(clippy::approx_constant)]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
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

/// **Not Chrome's (#989).** `rotateX(60deg)` and `translate(10px)` do not
/// pair, so each list is composed and the two decomposed. Chrome decomposes the
/// 4×4 matrices, which recovers the rotation and draws `rotateX(30deg)` half
/// way (`d` = 0.866025); rinch decomposes the flattened 2D matrices, which sees
/// only a y scale of 0.5 and lerps it to 0.75.
#[test]
fn a_mismatched_3d_pair_decomposes_flat() {
    check_transition(
        "rotateX(60deg)",
        "translate(10px)",
        &[
            (0.2, [1.0, 0.0, 0.0, 0.6, 2.0, 0.0]),
            (0.5, [1.0, 0.0, 0.0, 0.75, 5.0, 0.0]),
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
