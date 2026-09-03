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

/// `border-top-left-radius: calc(10% + 5px)` resolves to 25 against a 200px
/// box at paint time. Broken: Zero.
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
