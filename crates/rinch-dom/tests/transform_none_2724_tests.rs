//! #2724: a `transform` transition to or from `none` starts whenever the two
//! computed values differ — an identity list included.
//!
//! css-transitions-1 §3 starts a transition when the before-change and
//! after-change values differ and interpolate; `none` interpolates with any
//! list as the identity functions of the other side's types (css-transforms-1
//! §9). rinch compared the lists *after* that padding, so `translateX(0)` and
//! `none` were "the same value" and nothing started, in either direction. The
//! element then stopped (or started) being a stacking context and a fixed
//! box's containing block on the frame the class changed, where Chrome holds
//! the transform for the whole run.
//!
//! **Every expectation below was measured in Chrome 157** (headless, a 200×80
//! box with `transition: transform 1000ms linear`, the inline `transform`
//! switched after a style flush, the transition's `Animation` paused and
//! `currentTime` set; `getComputedStyle(el).transform` and
//! `getAnimations().length`):
//!
//! | from → to | Chrome starts | at 250 / 500 / 750 ms |
//! |---|---|---|
//! | `translateX(0)` ↔ `none`, `rotate(0deg)` ↔ `none`, `scale(1)` ↔ `none`, `translateX(0) rotate(0deg) scale(1)` ↔ `none` | yes, both ways | `matrix(1, 0, 0, 1, 0, 0)` |
//! | `translateX(0)` → `translateX(0) rotate(0deg)` | yes | `matrix(1, 0, 0, 1, 0, 0)` |
//! | `translateX(0)` → `translateX(0px)`, `none` → `none` | no | — |
//! | `rotate(90deg)`, `scale(2)` / `scale(2, 3)`, `translateX(40px) rotate(90deg) scale(2)` ↔ `none` | yes | the matrices in the tests |

#![allow(clippy::approx_constant)]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::transition::TransitionProperty;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// One `.t` div, `from` as its transform, a 1000ms linear transition, laid out
/// once with transitions armed. `transform-origin: 0 0` so the composed matrix
/// is Chrome's computed one.
fn transition_doc(from: &str, to: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(1_000_000.0)); // pinned: exact samples (#2267)
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(&format!(
        ".t {{ width: 200px; height: 80px; transform-origin: 0 0; \
         transition: transform 1000ms linear; transform: {from}; }} \
         .t.on {{ transform: {to}; }}"
    ));
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "t");
    doc.append_child(body, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, div)
}

/// Flip the class; the transform transition's start time, if one started.
fn flip(doc: &mut RinchDocument, div: NodeId) -> Option<f64> {
    doc.set_attribute(div, "class", "t on");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    doc.tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::Transform))
        .map(|t| t.start_time_ms)
}

fn assert_matrix(got: [f64; 6], want: [f64; 6], what: &str) {
    let close = got
        .iter()
        .zip(want.iter())
        .all(|(g, w)| (g - w).abs() <= 2e-4 * w.abs().max(1.0));
    assert!(close, "{what}: got {got:?}, Chrome 157 gives {want:?}");
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Run `from` → `to` and check the transform at 250, 500 and 750 ms: the
/// computed matrix is Chrome's, and the value is a transform, not `none` — so
/// the element is a stacking context and its fixed descendants' containing
/// block for the run, as in Chrome.
fn check(from: &str, to: &str, samples: [[f64; 6]; 3]) {
    let (mut doc, div) = transition_doc(from, to);
    let t0 = flip(&mut doc, div)
        .unwrap_or_else(|| panic!("{from} -> {to}: Chrome 157 starts a transition"));
    for (ms, want) in [250.0, 500.0, 750.0].into_iter().zip(samples) {
        rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + ms);
        let n = &doc.tree.nodes[div.0];
        let what = format!("{from} -> {to} at {ms}ms");
        assert_matrix(n.computed_style.transform.matrix, want, &what);
        assert!(!n.computed_style.transform.is_none, "{what}: not `none`");
        assert!(n.creates_stacking_context(), "{what}: a stacking context");
        assert!(
            n.establishes_fixed_containing_block(),
            "{what}: contains its fixed boxes"
        );
    }
    // The finishing tick lands on the after-change value: `none` is `none`
    // again, an identity list stays a transform.
    rinch_dom::transition::tick_transitions(&mut doc.tree, t0 + 1500.0);
    let tf = &doc.tree.nodes[div.0].computed_style.transform;
    assert_eq!(tf.is_none, to == "none", "{from} -> {to}: finished");
}

/// The issue's pair, both ways.
#[test]
fn translate_x_zero_against_none_transitions_both_ways() {
    check("translateX(0)", "none", [IDENTITY; 3]);
    check("none", "translateX(0)", [IDENTITY; 3]);
}

/// Other identity functions, and a list of several, against `none`.
#[test]
fn identity_functions_and_lists_against_none_transition_both_ways() {
    for tf in [
        "rotate(0deg)",
        "scale(1)",
        "translateX(0) rotate(0deg) scale(1)",
    ] {
        check(tf, "none", [IDENTITY; 3]);
        check("none", tf, [IDENTITY; 3]);
    }
}

/// A list that only gains an identity function is a change too: Chrome starts
/// a transition from `translateX(0)` to `translateX(0) rotate(0deg)`.
#[test]
fn a_longer_identity_list_is_a_change() {
    check("translateX(0)", "translateX(0) rotate(0deg)", [IDENTITY; 3]);
}

/// Equal computed values still start nothing.
#[test]
fn equal_values_start_nothing() {
    for (from, to) in [("translateX(0)", "translateX(0px)"), ("none", "none")] {
        let (mut doc, div) = transition_doc(from, to);
        assert_eq!(flip(&mut doc, div), None, "{from} -> {to}");
    }
}

/// Non-identity functions against `none` were already right; pinned here
/// with Chrome 157's numbers off the midpoint, both ways.
#[test]
fn rotate_scale_and_a_list_against_none_match_chrome() {
    let r1 = [0.382683, 0.92388, -0.92388, 0.382683, 0.0, 0.0];
    let r2 = [0.707107, 0.707107, -0.707107, 0.707107, 0.0, 0.0];
    let r3 = [0.92388, 0.382683, -0.382683, 0.92388, 0.0, 0.0];
    check("rotate(90deg)", "none", [r1, r2, r3]);
    check("none", "rotate(90deg)", [r3, r2, r1]);

    check(
        "scale(2)",
        "none",
        [
            [1.75, 0.0, 0.0, 1.75, 0.0, 0.0],
            [1.5, 0.0, 0.0, 1.5, 0.0, 0.0],
            [1.25, 0.0, 0.0, 1.25, 0.0, 0.0],
        ],
    );
    check(
        "none",
        "scale(2, 3)",
        [
            [1.25, 0.0, 0.0, 1.5, 0.0, 0.0],
            [1.5, 0.0, 0.0, 2.0, 0.0, 0.0],
            [1.75, 0.0, 0.0, 2.5, 0.0, 0.0],
        ],
    );

    let l1 = [0.669696, 1.61679, -1.61679, 0.669696, 30.0, 0.0];
    let l2 = [1.06066, 1.06066, -1.06066, 1.06066, 20.0, 0.0];
    let l3 = [1.15485, 0.478354, -0.478354, 1.15485, 10.0, 0.0];
    let list = "translateX(40px) rotate(90deg) scale(2)";
    check(list, "none", [l1, l2, l3]);
    check("none", list, [l3, l2, l1]);
}
