//! A `@keyframes` rule that leaves out its `from` (or `to`) stop animates from
//! (or to) the element's own computed value — for the layout properties too
//! (issue #767).
//!
//! css-animations-1 §4: "If a 0% or from keyframe is not specified, then the
//! user agent constructs a 0% keyframe using the computed values of the
//! properties being animated." Stylo hands that implicit stop over as
//! `KeyframesStepValue::ComputedValues`, and `extract_base_style_values` fills
//! it from the element's `ComputedStyle`. It used to carry only opacity, the
//! colours, the background position and the transform, so a to-only
//! `width: 200px` on a 100px box had nothing to interpolate from and wrote the
//! end value on the first frame.
//!
//! # The oracle
//!
//! Chrome 156.0.8078.73, headless: each case is a `div` in a 400px-wide
//! container (`font-size: 10px`), `animation: <k> 1000ms linear`, the
//! animation paused and seeked with `currentTime`, then `getComputedStyle` and
//! `offsetWidth`/`offsetHeight` read.
//!
//! | base | keyframes | 250ms | 500ms | 750ms |
//! |---|---|---|---|---|
//! | `width: 100px` | `to { width: 200px }` | 125 | 150 | 175 |
//! | `width: 100px` | `from { width: 200px }` | 175 | 150 | 125 |
//! | `height: 100px` | `to { height: 200px }` | 125 | 150 | 175 |
//! | `width: auto` (400) | `to { width: 200px }` | 400 | 200 | 200 |
//! | `width: 25%` (100) | `to { width: 200px }` | 125 | 150 | 175 |
//! | `width: 10em` (100) | `to { width: 200px }` | 125 | 150 | 175 |
//! | `width: 100px` | `to { width: 50% }` (200) | 125 | 150 | 175 |
//! | `width: 100px` | `50% { width: 200px }` | 150 | 200 | 150 |
//! | `padding-left: 10px` | `to { padding-left: 40px }` | 17.5 | 25 | 32.5 |
//! | `margin-left: 10px` | `to { margin-left: 40px }` | 17.5 | 25 | 32.5 |
//! | `margin-left: auto` (300) | `to { margin-left: 40px }` | 300 | 40 | 40 |
//! | `border-left-width: 10px` | `to { border-left-width: 40px }` | 17 | 25 | 32 |
//! | `font-size: 10px` | `to { font-size: 30px }` | 15 | 20 | 25 |
//! | `border-left-color: rgb(255,0,0)` | `to { … rgb(0,0,255) }` | (191,0,64) | (128,0,128) | (64,0,191) |
//!
//! (Chrome snaps a border width to whole device pixels — 17 and 32 — a rule
//! rinch does not implement; the fixtures sample at 500ms, where both agree.)
//!
//! Every width/height fixture reads the **laid-out** box, not only the
//! computed style: the frame has to reach Taffy.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::animation::{AnimationResult, tick_animations};
use rinch_dom::transition::{AnimatableValue, TransitionProperty};

/// A `div` with inline style `base` and `animation: k 1000ms linear`, under
/// `@keyframes k { keyframes }`, inside a 400px-wide container whose
/// `font-size` is 10px. Laid out once, so the animation is minted.
fn animated(base: &str, keyframes: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; }}
         .c {{ width: 400px; font-size: 10px; line-height: 10px; }}
         .b {{ height: 20px; }}
         @keyframes k {{ {keyframes} }}"
    ));
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(c, "class", "c");
    doc.append_child(body, c);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "b");
    doc.set_attribute(div, "style", &format!("{base}; animation: k 1000ms linear"));
    doc.append_child(c, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.active_animations.contains_key(&div.0),
        "positive control: the animation must be running after the first layout"
    );
    (doc, div)
}

/// Tick the div's animation to `ms` after its start and lay out again; the
/// box's laid-out `(width, height)`.
fn layout_at(doc: &mut RinchDocument, div: NodeId, ms: f64) -> (f32, f32) {
    let t0 = doc.tree.active_animations[&div.0][0].start_time_ms;
    tick_animations(&mut doc.tree, t0 + ms);
    doc.tree.layout_dirty = true;
    doc.resolve_layout(800.0, 600.0);
    let n = doc.tree.get(div.0).expect("the div is live");
    (n.layout.width, n.layout.height)
}

/// The animation's own sample of `property` at `ms`.
fn sample(
    doc: &RinchDocument,
    div: NodeId,
    ms: f64,
    property: TransitionProperty,
) -> Option<AnimatableValue> {
    let anim = &doc.tree.active_animations[&div.0][0];
    let AnimationResult::Values(values) = anim.values_at(anim.start_time_ms + ms) else {
        panic!("a running animation yields values");
    };
    values
        .into_iter()
        .find_map(|(p, v)| (p == property).then_some(v))
}

fn close(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() < 0.5,
        "{what}: Chrome gives {want}, got {got}"
    );
}

#[test]
fn a_to_only_width_animates_from_the_computed_width() {
    let (mut doc, div) = animated("width: 100px", "to { width: 200px; }");
    for (ms, want) in [(250.0, 125.0), (500.0, 150.0), (750.0, 175.0)] {
        close(
            layout_at(&mut doc, div, ms).0,
            want,
            &format!("width at {ms}ms"),
        );
    }
}

#[test]
fn a_from_only_width_animates_to_the_computed_width() {
    let (mut doc, div) = animated("width: 100px", "from { width: 200px; }");
    for (ms, want) in [(250.0, 175.0), (500.0, 150.0), (750.0, 125.0)] {
        close(
            layout_at(&mut doc, div, ms).0,
            want,
            &format!("width at {ms}ms"),
        );
    }
}

#[test]
fn a_to_only_height_animates_from_the_computed_height() {
    let (mut doc, div) = animated("height: 100px", "to { height: 200px; }");
    for (ms, want) in [(250.0, 125.0), (500.0, 150.0), (750.0, 175.0)] {
        close(
            layout_at(&mut doc, div, ms).1,
            want,
            &format!("height at {ms}ms"),
        );
    }
}

/// A stop only at 50%: both ends are implicit.
#[test]
fn a_middle_only_stop_takes_both_ends_from_the_computed_width() {
    let (mut doc, div) = animated("width: 100px", "50% { width: 200px; }");
    for (ms, want) in [(250.0, 150.0), (500.0, 200.0), (750.0, 150.0)] {
        close(
            layout_at(&mut doc, div, ms).0,
            want,
            &format!("width at {ms}ms"),
        );
    }
}

/// `auto` does not interpolate with a length (`interpolate-size` is
/// `numeric-only` by default): Chrome flips at the midpoint.
#[test]
fn an_auto_base_width_flips_at_the_midpoint() {
    let (mut doc, div) = animated("", "to { width: 200px; }");
    close(
        layout_at(&mut doc, div, 250.0).0,
        400.0,
        "auto before the midpoint",
    );
    close(
        layout_at(&mut doc, div, 750.0).0,
        200.0,
        "200px after the midpoint",
    );
}

/// `width: 10em` computes to 100px at a 10px font size.
#[test]
fn an_em_base_width_animates_from_its_computed_length() {
    let (mut doc, div) = animated("width: 10em", "to { width: 200px; }");
    close(layout_at(&mut doc, div, 500.0).0, 150.0, "width at 500ms");
}

/// A percentage base against a length stop interpolates as a `calc()`:
/// `25%` of 400 halfway to 200px is 150.
#[test]
fn a_percentage_base_width_interpolates_against_a_length_stop() {
    let (mut doc, div) = animated("width: 25%", "to { width: 200px; }");
    for (ms, want) in [(250.0, 125.0), (500.0, 150.0), (750.0, 175.0)] {
        close(
            layout_at(&mut doc, div, ms).0,
            want,
            &format!("width at {ms}ms"),
        );
    }
}

/// And the mirror: a length base against a percentage stop.
#[test]
fn a_length_base_width_interpolates_against_a_percentage_stop() {
    let (mut doc, div) = animated("width: 100px", "to { width: 50%; }");
    for (ms, want) in [(250.0, 125.0), (500.0, 150.0), (750.0, 175.0)] {
        close(
            layout_at(&mut doc, div, ms).0,
            want,
            &format!("width at {ms}ms"),
        );
    }
}

#[test]
fn a_to_only_padding_animates_from_the_computed_padding() {
    let (doc, div) = animated("padding-left: 10px", "to { padding-left: 40px; }");
    let got = sample(&doc, div, 500.0, TransitionProperty::PaddingLeft);
    assert!(
        matches!(got, Some(AnimatableValue::LengthPercentage(v)) if (v.to_px() - 25.0).abs() < 0.01),
        "padding-left at 500ms: Chrome gives 25px, got {got:?}"
    );
}

#[test]
fn a_to_only_margin_animates_from_the_computed_margin() {
    let (mut doc, div) = animated(
        "width: 100px; margin-left: 10px",
        "to { margin-left: 40px; }",
    );
    let got = sample(&doc, div, 500.0, TransitionProperty::MarginLeft);
    assert!(
        matches!(got, Some(AnimatableValue::LengthPercentageAuto(
            rinch_dom::computed_style::LengthPercentageAutoValue::Length(v)
        )) if (v - 25.0).abs() < 0.01),
        "margin-left at 500ms: Chrome gives 25px, got {got:?}"
    );
    // And it moves the box.
    layout_at(&mut doc, div, 500.0);
    let x = doc.tree.get(div.0).unwrap().layout.x;
    close(x, 25.0, "the box's x at 500ms");
}

/// `margin-left: auto` is not interpolable with a length either.
#[test]
fn an_auto_base_margin_flips_at_the_midpoint() {
    let (doc, div) = animated(
        "width: 100px; margin-left: auto",
        "to { margin-left: 40px; }",
    );
    let before = sample(&doc, div, 250.0, TransitionProperty::MarginLeft);
    assert!(
        matches!(
            before,
            Some(AnimatableValue::LengthPercentageAuto(
                rinch_dom::computed_style::LengthPercentageAutoValue::Auto
            ))
        ),
        "margin-left at 250ms is still `auto` (Chrome: 300px used), got {before:?}"
    );
}

#[test]
fn a_to_only_border_width_animates_from_the_computed_border_width() {
    let (doc, div) = animated(
        "border-left: 10px solid rgb(0, 0, 0)",
        "to { border-left-width: 40px; }",
    );
    let got = sample(&doc, div, 500.0, TransitionProperty::BorderLeftWidth);
    assert!(
        matches!(got, Some(AnimatableValue::LengthPercentage(v)) if (v.to_px() - 25.0).abs() < 0.01),
        "border-left-width at 500ms: Chrome gives 25px, got {got:?}"
    );
}

#[test]
fn a_to_only_font_size_animates_from_the_computed_font_size() {
    let (doc, div) = animated("font-size: 10px", "to { font-size: 30px; }");
    let got = sample(&doc, div, 500.0, TransitionProperty::FontSize);
    assert!(
        matches!(got, Some(AnimatableValue::Float(v)) if (v - 20.0).abs() < 0.01),
        "font-size at 500ms: Chrome gives 20px, got {got:?}"
    );
}

#[test]
fn a_to_only_border_color_animates_from_the_computed_border_color() {
    let (doc, div) = animated(
        "border-left: 10px solid rgb(255, 0, 0)",
        "to { border-left-color: rgb(0, 0, 255); }",
    );
    let got = sample(&doc, div, 500.0, TransitionProperty::BorderLeftColor);
    let Some(AnimatableValue::Color(c)) = got else {
        panic!("border-left-color at 500ms: Chrome gives rgb(128, 0, 128), got {got:?}");
    };
    let c = c.to_rgba8();
    assert!(
        (c.r as i32 - 128).abs() <= 1 && c.g == 0 && (c.b as i32 - 128).abs() <= 1,
        "border-left-color at 500ms: Chrome gives rgb(128, 0, 128), got {c:?}"
    );
}

/// The implicit stop carries **only the properties the rule animates**
/// (css-animations-1: "the properties being animated"). A transform-only spin
/// on a sized box must not write its width, height, margins or padding on
/// every frame — that would pin them at the values the animation started with
/// and mark the box for layout on every tick.
#[test]
fn a_transform_only_animation_carries_no_layout_property() {
    let (doc, div) = animated(
        "width: 100px; padding-left: 10px; margin-left: 10px",
        "to { transform: rotate(360deg); }",
    );
    let anim = &doc.tree.active_animations[&div.0][0];
    let AnimationResult::Values(values) = anim.values_at(anim.start_time_ms + 500.0) else {
        panic!("a running animation yields values");
    };
    let props: Vec<_> = values.iter().map(|(p, _)| *p).collect();
    assert_eq!(
        props,
        vec![TransitionProperty::Transform],
        "a transform-only rule animates transform alone"
    );
}
