//! Mixed `calc()` — a `calc()` combining a percentage and a length — across
//! every converter family (#278, #404).
//!
//! stylo's `to_length()` and `to_percentage()` both answer `None` for a mixed
//! calc, so before the fix every converter's else-arm silently degraded: an
//! inset became `0` (#278), a translation vanished (#404), padding became
//! `Zero`, a transform-origin became `50%`. Every expected value below was
//! taken from Chrome (`getBoundingClientRect`/`getComputedStyle` on the same
//! declarations, 300x200 container) before the fix was designed, so the
//! fixtures cannot inherit a misconception from the implementation.
//!
//! Fixture discipline: each mixed calc here has a percentage part that is
//! non-zero at the test's container size, and its correct answer differs from
//! the pure-length reading, the pure-percentage reading, *and* the broken
//! (zero / 50%) reading.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::LengthPercentageValue;

const EPS: f32 = 0.51; // one device px of layout rounding

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < EPS,
        "{what}: expected {expected}, got {actual}"
    );
}

/// A 300x200 `position: relative` container appended to `<body>`.
fn container(doc: &mut RinchDocument) -> rinch_core::dom::NodeId {
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        "position: relative; width: 300px; height: 200px; padding: 0; margin: 0; border: 0;",
    );
    doc.append_child(body, c);
    c
}

// ---------------------------------------------------------------------------
// Insets (#278) — box_model::inset_from_stylo_generic
// ---------------------------------------------------------------------------

/// The exact #278 fixture: `left: calc(50% - 10px)` in a 300px containing
/// block sits at x = 140 (Chrome: 140). Broken: 0. Pure-pct: 150. Pure-len: -10.
#[test]
fn calc_inset_left() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "position: absolute; left: calc(50% - 10px); top: 0; width: 50px; height: 50px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.x, 140.0, "left");
}

/// `top: calc(50% + 10px)` resolves against the containing block *height*
/// (200): y = 110 (Chrome: 110). Broken: 0. Pure-pct: 100. Width-basis: 160.
#[test]
fn calc_inset_top_uses_height_basis() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "position: absolute; top: calc(50% + 10px); left: 0; width: 50px; height: 50px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.y, 110.0, "top");
}

/// `right: calc(25% + 5px)`: 80px from the right edge of a 300px containing
/// block, so x = 300 - 80 - 50 = 170 (Chrome: 80 from the right).
#[test]
fn calc_inset_right() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "position: absolute; right: calc(25% + 5px); top: 0; width: 50px; height: 50px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(
        doc.tree.get(a.0).unwrap().layout.x,
        170.0,
        "right-derived x",
    );
}

// ---------------------------------------------------------------------------
// Sizes — layout::size_from_stylo / max_size_from_stylo / flex_basis
// ---------------------------------------------------------------------------

/// `width: calc(50% + 25px)` in 300 → 175 (Chrome: 175). Broken: auto.
#[test]
fn calc_width() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: calc(50% + 25px); height: 10px;");
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 175.0, "width");
}

/// `height: calc(50% - 20px)` against a definite 200px parent → 80 (Chrome: 80).
#[test]
fn calc_height() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "height: calc(50% - 20px);");
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.height, 80.0, "height");
}

/// `min-width: calc(50% + 20px)` floors a 10px box at 170 (Chrome: 170).
#[test]
fn calc_min_width() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "width: 10px; min-width: calc(50% + 20px); height: 10px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 170.0, "min-width");
}

/// `max-width: calc(50% - 50px)` caps a 250px box at 100 (Chrome: 100).
#[test]
fn calc_max_width() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "width: 250px; max-width: calc(50% - 50px); height: 10px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 100.0, "max-width");
}

/// `flex-basis: calc(50% - 10px)` in a 300px row flex container → 140
/// (Chrome: 140). Broken: auto (content size, 0 here).
#[test]
fn calc_flex_basis() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    doc.set_attribute(
        c,
        "style",
        "position: relative; display: flex; flex-direction: row; width: 300px; height: 200px;",
    );
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "flex: 0 0 calc(50% - 10px); height: 10px;");
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 140.0, "flex-basis");
}

// ---------------------------------------------------------------------------
// Padding / margin / gap — box_model
// ---------------------------------------------------------------------------

/// `padding-left: calc(25% + 5px)` on a box whose parent is 300px wide —
/// padding percentages resolve against the *parent's* content width — indents
/// the grandchild to x = 80 (Chrome: 80px computed padding). Broken: 0.
#[test]
fn calc_padding() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "padding-left: calc(25% + 5px);");
    doc.append_child(c, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 10px; height: 10px;");
    doc.append_child(a, b);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(b.0).unwrap().layout.x, 80.0, "padding-left");
}

/// `padding-left: calc(10px - 50%)` computes negative and the property's
/// non-negative clamp floors it at 0 (Chrome: 0px).
///
/// NOTE: this fixture does NOT distinguish fixed from broken code — the broken
/// else-arm also answered 0. It is here to pin the clamp: the fix recovers the
/// underlying affine (10px - 50%) and must still clamp its resolution to 0,
/// as stylo's own `resolve()` would. `calc_padding` above is the
/// distinguishing fixture for this converter.
#[test]
fn calc_padding_negative_clamps_to_zero() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "padding-left: calc(10px - 50%);");
    doc.append_child(c, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 10px; height: 10px;");
    doc.append_child(a, b);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(b.0).unwrap().layout.x, 0.0, "clamped padding");
}

/// `margin-left: calc(25% + 5px)` on 300 → x = 80 (Chrome: 80). Broken: 0.
#[test]
fn calc_margin() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "margin-left: calc(25% + 5px); width: 10px; height: 10px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.x, 80.0, "margin-left");
}

/// Margins are NOT clamped: `margin-left: calc(10px - 10%)` on 300 → x = -20
/// (Chrome: -20). Broken: 0. This is the fixture that catches a fix that
/// clamps everything non-negative.
#[test]
fn calc_margin_negative_is_legal() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "margin-left: calc(10px - 10%); width: 10px; height: 10px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(
        doc.tree.get(a.0).unwrap().layout.x,
        -20.0,
        "negative margin",
    );
}

/// `column-gap: calc(10% + 5px)` in a 300px flex row → gap 35, so the second
/// 40px item sits at x = 75 (Chrome: 75). Broken: 40 (gap 0).
#[test]
fn calc_column_gap() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    doc.set_attribute(
        c,
        "style",
        "position: relative; display: flex; flex-direction: row; width: 300px; height: 200px; column-gap: calc(10% + 5px);",
    );
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 40px; height: 10px;");
    doc.append_child(c, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 40px; height: 10px;");
    doc.append_child(c, b);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(b.0).unwrap().layout.x, 75.0, "column-gap");
}

// ---------------------------------------------------------------------------
// Stability and responsiveness of the resolution
// ---------------------------------------------------------------------------

/// The same document laid out twice gives the same answer — the calc fixpoint
/// is stable, not oscillating.
#[test]
fn calc_stable_across_layouts() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: calc(50% + 25px); height: 10px;");
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let first = doc.tree.get(a.0).unwrap().layout.width;
    doc.resolve_layout(800.0, 600.0);
    let second = doc.tree.get(a.0).unwrap().layout.width;
    assert_near(first, 175.0, "first layout");
    assert_near(second, 175.0, "second layout");
}

/// Restyling the container re-resolves the calc against the new basis:
/// 300 → 175, then 400 → 225.
#[test]
fn calc_follows_container_resize() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: calc(50% + 25px); height: 10px;");
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 175.0, "at 300");
    doc.set_attribute(
        c,
        "style",
        "position: relative; width: 400px; height: 200px;",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 225.0, "at 400");
}

/// A chain of calc-sized containers converges: parent `calc(50% + 50px)` of
/// 400 → 250, child `calc(50% - 25px)` of 250 → 100.
#[test]
fn calc_nested_chain_converges() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let g = doc.create_element("div");
    doc.set_attribute(g, "style", "width: 400px; height: 200px;");
    doc.append_child(body, g);
    let p = doc.create_element("div");
    doc.set_attribute(p, "style", "width: calc(50% + 50px); height: 100px;");
    doc.append_child(g, p);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: calc(50% - 25px); height: 10px;");
    doc.append_child(p, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(p.0).unwrap().layout.width, 250.0, "parent");
    assert_near(doc.tree.get(a.0).unwrap().layout.width, 100.0, "child");
}

/// A non-affine calc — `calc(min(50%, 100px) + 10px)` — resolves as its
/// large-basis linearization (min collapses to its 100px arm), which at this
/// container size agrees with Chrome: x = 110 on 300. The linearization is a
/// named approximation: below a 200px container Chrome would track the 50%
/// arm while rinch stays at 110. See `from_stylo/calc.rs`.
#[test]
fn calc_non_affine_min_linearizes() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "position: absolute; left: calc(min(50%, 100px) + 10px); top: 0; width: 10px; height: 10px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(a.0).unwrap().layout.x, 110.0, "min() calc");
}

// ---------------------------------------------------------------------------
// Paint-side consumers — visual.rs (#404) and border-radius
// ---------------------------------------------------------------------------

/// #404's exact fixture: `translateX(calc(50% - 10px))` used to resolve to no
/// translation at all. The split must recover px = -10 into the matrix and
/// pct = 0.5 into the percentage-translate coefficients, so paint composes
/// 0.5·200 - 10 = 90 (Chrome: matrix(1, 0, 0, 1, 90, 0) on a 200px box).
#[test]
fn calc_transform_translate_split() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "width: 200px; height: 20px; transform: translateX(calc(50% - 10px));",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let tf = &doc.tree.get(a.0).unwrap().computed_style.transform;
    assert!(
        (tf.matrix[4] - (-10.0)).abs() < 1e-3,
        "px part in matrix e: expected -10, got {}",
        tf.matrix[4]
    );
    assert!(
        (tf.pct_translate_w[0] - 0.5).abs() < 1e-6,
        "pct part: expected 0.5, got {}",
        tf.pct_translate_w[0]
    );
    assert!(!tf.is_identity, "a mixed-calc translate is not identity");
}

/// `transform-origin: calc(25% + 30px)` on a 200px box resolves to 80
/// (Chrome: "80px"). Broken: the else-arm answered 50%, i.e. 100 — which is
/// why the box is 200px wide and the calc is 25%-based: correct and broken
/// answers must not coincide.
#[test]
fn calc_transform_origin() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "width: 200px; height: 100px; transform-origin: calc(25% + 30px) center;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let ox = doc
        .tree
        .get(a.0)
        .unwrap()
        .computed_style
        .transform_origin_x
        .resolve(200.0);
    assert_near(ox, 80.0, "transform-origin-x");
}

/// `border-top-left-radius: calc(10% + 5px)` carries the split into the
/// computed value; this pins the converter + `resolve()` contract at an
/// explicit 200px basis (10%·200 + 5 = 25). Broken: Zero. Paint itself
/// resolves radii against `min(width, height)` — 100 here, painting 15 — a
/// pre-existing per-axis approximation independent of calc
/// (`paint/mod.rs`, `paint/borders.rs`).
#[test]
fn calc_border_radius() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "width: 200px; height: 100px; border-top-left-radius: calc(10% + 5px);",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let r = doc
        .tree
        .get(a.0)
        .unwrap()
        .computed_style
        .border_radius_top_left
        .resolve(200.0);
    assert_near(r, 25.0, "border-radius");
}

/// The computed value itself carries the split, not a degraded single variant.
#[test]
fn calc_computed_value_is_calc_variant() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "padding-left: calc(50% - 10px);");
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let pl = doc.tree.get(a.0).unwrap().computed_style.padding_left;
    match pl {
        LengthPercentageValue::Calc { px, pct } => {
            assert!((px - (-10.0)).abs() < 1e-3, "px: {px}");
            assert!((pct - 0.5).abs() < 1e-6, "pct: {pct}");
        }
        other => panic!("expected Calc, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Indefinite percentage bases (#496 review finding 1)
// ---------------------------------------------------------------------------

/// A `Calc` whose percentage basis is an auto-sized axis must not feed the
/// fixpoint its own output. `row-gap: calc(10% + 5px)` in an auto-height
/// column flex resolves its own-axis percentage against an *indefinite*
/// height, and an indefinite basis resolves the percentage part as zero —
/// like taffy's `maybe_resolve(None)` and like Chrome: gap 5px, container
/// height 12·10 + 11·5 = 175, second child at y = 15. Before this gate the
/// fixpoint resolved against the post-layout height the gap itself inflated
/// and exploded to height 2376 / gap 205.
#[test]
fn calc_row_gap_indefinite_basis_keeps_px_part() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        "display: flex; flex-direction: column; width: 300px; row-gap: calc(10% + 5px);",
    );
    doc.append_child(body, c);
    let mut kids = Vec::new();
    for _ in 0..12 {
        let k = doc.create_element("div");
        doc.set_attribute(k, "style", "width: 12px; height: 10px; flex: none;");
        doc.append_child(c, k);
        kids.push(k);
    }
    doc.resolve_layout(800.0, 600.0);
    assert_near(
        doc.tree.get(c.0).unwrap().layout.height,
        175.0,
        "container height",
    );
    assert_near(
        doc.tree.get(kids[1].0).unwrap().layout.y,
        15.0,
        "second child y",
    );
}

/// `height: calc(50% + 10px)` against an auto-height parent behaves as auto,
/// exactly like plain `height: 50%` does (Chrome: both 0). The assertion is
/// parity with the plain-percentage twin in an identical sibling container —
/// whatever rinch answers for `50%` it must answer for the calc — plus a
/// bound that rules out the pre-gate feedback explosion (parent measured at
/// 2261 before the fix).
#[test]
fn calc_size_pct_against_indefinite_behaves_as_auto() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let build = |doc: &mut RinchDocument, child_style: &str| {
        let c = doc.create_element("div");
        doc.set_attribute(c, "style", "width: 300px;");
        doc.append_child(doc.body(), c);
        for _ in 0..3 {
            let k = doc.create_element("div");
            doc.set_attribute(k, "style", child_style);
            doc.append_child(c, k);
        }
        c
    };
    let _ = body;
    let pct = build(&mut doc, "height: 50%;");
    let calc = build(&mut doc, "height: calc(50% + 10px);");
    doc.resolve_layout(800.0, 600.0);
    let pct_h = doc.tree.get(pct.0).unwrap().layout.height;
    let calc_h = doc.tree.get(calc.0).unwrap().layout.height;
    assert!(
        (calc_h - pct_h).abs() < EPS,
        "calc must behave like its plain-%% twin against an indefinite basis: %% gives {pct_h}, calc gives {calc_h}"
    );
    assert!(
        calc_h < 100.0,
        "indefinite-basis calc height must not feed back and explode; got {calc_h}"
    );
}

// ---------------------------------------------------------------------------
// Absolute children resolve against the padding box (#496 review finding 2)
// ---------------------------------------------------------------------------

/// An absolutely positioned child's percentages resolve against its
/// containing block's *padding box*, not its content box — Taffy subtracts
/// only the border (flexbox.rs:2166, block.rs:580) and so does CSS. On a
/// 300×200 container with padding 20 (border-box 300 under Taffy's
/// box-sizing), Chrome gives left 140 and width 160; resolving against the
/// content box would give 120/140, off by exactly the parent padding and
/// inconsistent with the plain-% twin (150/150) on the same box.
#[test]
fn calc_abs_child_resolves_against_padding_box() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    doc.set_attribute(
        c,
        "style",
        "position: relative; width: 300px; height: 200px; padding: 20px;",
    );
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "position: absolute; left: calc(50% - 10px); top: 0; width: calc(50% + 10px); height: 50px;",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let la = doc.tree.get(a.0).unwrap().layout;
    assert_near(la.x, 140.0, "abs left vs padding box");
    assert_near(la.width, 160.0, "abs width vs padding box");
}

// ---------------------------------------------------------------------------
// ICB-absolute sizes resolve against the viewport (#496 review finding 3)
// ---------------------------------------------------------------------------

/// An absolute box with no positioned ancestor sizes against the initial
/// containing block (#204). `out_of_flow` already bakes a plain `Percent`
/// width/height from the viewport; `Calc` must take the same arm: with an
/// unpositioned 300px parent and an 800px viewport, `width: calc(50% + 10px)`
/// is 410 (Chrome: innerWidth·0.5 + 10). Resolving against the Taffy parent
/// would give 160.
#[test]
fn calc_icb_absolute_resolves_against_viewport() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let g = doc.create_element("div");
    doc.set_attribute(g, "style", "width: 300px; height: 100px;");
    doc.append_child(body, g);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "position: absolute; width: calc(50% + 10px); height: calc(25% - 20px);",
    );
    doc.append_child(g, a);
    doc.resolve_layout(800.0, 600.0);
    let la = doc.tree.get(a.0).unwrap().layout;
    assert_near(la.width, 410.0, "ICB abs width");
    assert_near(la.height, 130.0, "ICB abs height (0.25*600 - 20)");
}

// ---------------------------------------------------------------------------
// Composite survivor sweep (#496 review finding 4): one fixture per family
// member the first batch left untested. Every expected value is Chrome's, and
// every height-basis value differs from its width-basis reading (and vice
// versa), so a transposed basis or a dropped patch arm fails here.
// ---------------------------------------------------------------------------

#[test]
fn calc_composite_families() {
    let mut doc = RinchDocument::new();

    // t1: padding-top, basis parent WIDTH 300 → 10%·300 + 5 = 35 (Chrome 35;
    // height basis would give 25).
    let c1 = container(&mut doc);
    let a1 = doc.create_element("div");
    doc.set_attribute(a1, "style", "padding-top: calc(10% + 5px);");
    doc.append_child(c1, a1);
    let b1 = doc.create_element("div");
    doc.set_attribute(b1, "style", "width: 10px; height: 10px;");
    doc.append_child(a1, b1);

    // t2: padding-right, basis parent WIDTH 300 → content width
    // 300 − (5%·300 + 5) = 280 (Chrome 280; height basis: 285).
    let c2 = container(&mut doc);
    let a2 = doc.create_element("div");
    doc.set_attribute(a2, "style", "padding-right: calc(5% + 5px);");
    doc.append_child(c2, a2);
    let b2 = doc.create_element("div");
    doc.set_attribute(b2, "style", "height: 10px;");
    doc.append_child(a2, b2);

    // t3/t4: margin-top and a negative margin-bottom, both basis parent WIDTH
    // 300 (explicit flex column, so no block margin collapsing in either
    // engine): first at y = 5%·300 + 10 = 25, second at
    // 25 + 10 + (10%·300 − 45) = 20 (Chrome 25/20; height bases: 20/10).
    let c3 = container(&mut doc);
    doc.set_attribute(
        c3,
        "style",
        "position: relative; display: flex; flex-direction: column; align-items: flex-start; width: 300px; height: 200px;",
    );
    let a3 = doc.create_element("div");
    doc.set_attribute(
        a3,
        "style",
        "margin-top: calc(5% + 10px); margin-bottom: calc(10% - 45px); width: 10px; height: 10px; flex: none;",
    );
    doc.append_child(c3, a3);
    let b3 = doc.create_element("div");
    doc.set_attribute(b3, "style", "width: 10px; height: 10px; flex: none;");
    doc.append_child(c3, b3);

    // t5: bottom inset, basis parent HEIGHT 200 → y = 200 − (10%·200 + 10)
    // − 50 = 120 (Chrome 120; width basis: 110).
    let c5 = container(&mut doc);
    let a5 = doc.create_element("div");
    doc.set_attribute(
        a5,
        "style",
        "position: absolute; bottom: calc(10% + 10px); left: 0; width: 50px; height: 50px;",
    );
    doc.append_child(c5, a5);

    // t6: row-gap with a DEFINITE own height, basis own content HEIGHT 200 →
    // gap 25, second child at y = 35 (Chrome 35; width basis: 45; a deleted
    // row-gap patch: 15).
    let c6 = container(&mut doc);
    doc.set_attribute(
        c6,
        "style",
        "position: relative; display: flex; flex-direction: column; width: 300px; height: 200px; row-gap: calc(10% + 5px);",
    );
    let a6 = doc.create_element("div");
    doc.set_attribute(a6, "style", "width: 10px; height: 10px; flex: none;");
    doc.append_child(c6, a6);
    let b6 = doc.create_element("div");
    doc.set_attribute(b6, "style", "width: 10px; height: 10px; flex: none;");
    doc.append_child(c6, b6);

    // t7: min-height, basis parent HEIGHT 200 → 25%·200 + 30 = 80
    // (Chrome 80; width basis: 105; dropped from has_layout_calc: seed 30).
    let c7 = container(&mut doc);
    let a7 = doc.create_element("div");
    doc.set_attribute(a7, "style", "min-height: calc(25% + 30px);");
    doc.append_child(c7, a7);

    // t8: max-height, basis parent HEIGHT 200 → 500px capped at
    // 50%·200 + 40 = 140 (Chrome 140; width basis: 190; dropped: seed 40).
    let c8 = container(&mut doc);
    let a8 = doc.create_element("div");
    doc.set_attribute(a8, "style", "height: 500px; max-height: calc(50% + 40px);");
    doc.append_child(c8, a8);

    doc.resolve_layout(800.0, 600.0);
    assert_near(doc.tree.get(b1.0).unwrap().layout.y, 35.0, "t1 padding-top");
    assert_near(
        doc.tree.get(b2.0).unwrap().layout.width,
        280.0,
        "t2 padding-right",
    );
    assert_near(doc.tree.get(a3.0).unwrap().layout.y, 25.0, "t3 margin-top");
    assert_near(
        doc.tree.get(b3.0).unwrap().layout.y,
        20.0,
        "t4 margin-bottom",
    );
    assert_near(
        doc.tree.get(a5.0).unwrap().layout.y,
        120.0,
        "t5 bottom inset",
    );
    assert_near(doc.tree.get(b6.0).unwrap().layout.y, 35.0, "t6 row-gap");
    assert_near(
        doc.tree.get(a7.0).unwrap().layout.height,
        80.0,
        "t7 min-height",
    );
    assert_near(
        doc.tree.get(a8.0).unwrap().layout.height,
        140.0,
        "t8 max-height",
    );
}

/// The vertical transform-origin component takes the height basis:
/// `transform-origin: center calc(10% + 70px)` on a 200×100 box resolves
/// y to 80 (Chrome "100px 80px"; the broken 50% default would give 50, the
/// width basis 90).
#[test]
fn calc_transform_origin_y() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "width: 200px; height: 100px; transform-origin: center calc(10% + 70px);",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let oy = doc
        .tree
        .get(a.0)
        .unwrap()
        .computed_style
        .transform_origin_y
        .resolve(100.0);
    assert_near(oy, 80.0, "transform-origin-y");
}

/// letter-/word-spacing keep the length part of a mixed calc — the named
/// limitation at `typography.rs`: the percentage part is font-relative at
/// used-value time (Chrome keeps `calc(50% + 5px)` unresolved in the
/// computed value) and px-only spacing cannot express it. Before the fix the
/// whole value was dropped to 0.
#[test]
fn calc_letter_and_word_spacing_keep_px_part() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    let a = doc.create_element("div");
    doc.set_attribute(
        a,
        "style",
        "letter-spacing: calc(5px + 50%); word-spacing: calc(3px + 25%);",
    );
    doc.append_child(c, a);
    doc.resolve_layout(800.0, 600.0);
    let cs = &doc.tree.get(a.0).unwrap().computed_style;
    assert_near(cs.letter_spacing, 5.0, "letter-spacing px part");
    assert_near(cs.word_spacing, 3.0, "word-spacing px part");
}

/// Grid tracks keep only the percentage component of a mixed calc — the named
/// limitation at `grid.rs::length_percentage_from_stylo_lp`. This PINS the
/// documented approximation: `calc(50% - 10px)` sizes the track at 50% of 300
/// = 150 where Chrome answers 140 exactly. If this fails at 140, the
/// limitation was lifted — update the comment there and this fixture together.
#[test]
fn calc_grid_track_keeps_percent_component() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc);
    doc.set_attribute(
        c,
        "style",
        "position: relative; display: grid; grid-template-columns: calc(50% - 10px) 1fr; width: 300px; height: 200px;",
    );
    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "height: 10px;");
    doc.append_child(c, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "height: 10px;");
    doc.append_child(c, b);
    doc.resolve_layout(800.0, 600.0);
    assert_near(
        doc.tree.get(a.0).unwrap().layout.width,
        150.0,
        "grid track percent-component approximation (Chrome exact: 140)",
    );
}
