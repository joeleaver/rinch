//! Unit tests for the CSS transition engine.

use rinch_dom::computed_style::*;
use rinch_dom::transition::*;
use std::collections::HashMap;

// ── TimingFunction tests ──────────────────────────────────────────────

#[test]
fn test_linear_timing() {
    let tf = TimingFunction::Linear;
    assert_eq!(tf.apply(0.0), 0.0);
    assert_eq!(tf.apply(0.5), 0.5);
    assert_eq!(tf.apply(1.0), 1.0);
}

#[test]
fn test_ease_timing_endpoints() {
    let tf = TimingFunction::Ease;
    assert!((tf.apply(0.0) - 0.0).abs() < 0.001);
    assert!((tf.apply(1.0) - 1.0).abs() < 0.001);
}

#[test]
fn test_ease_timing_midpoint() {
    // ease = cubic-bezier(0.25, 0.1, 0.25, 1.0)
    // At t=0.5, ease should be past 0.5 (it accelerates then decelerates)
    let tf = TimingFunction::Ease;
    let mid = tf.apply(0.5);
    assert!(mid > 0.5, "ease at t=0.5 should be > 0.5, got {}", mid);
}

#[test]
fn test_ease_in_starts_slow() {
    let tf = TimingFunction::EaseIn;
    // EaseIn starts slow — at t=0.25, value should be < 0.25
    let val = tf.apply(0.25);
    assert!(
        val < 0.25,
        "ease-in at t=0.25 should be < 0.25, got {}",
        val
    );
}

#[test]
fn test_ease_out_starts_fast() {
    let tf = TimingFunction::EaseOut;
    // EaseOut starts fast — at t=0.25, value should be > 0.25
    let val = tf.apply(0.25);
    assert!(
        val > 0.25,
        "ease-out at t=0.25 should be > 0.25, got {}",
        val
    );
}

#[test]
fn test_custom_cubic_bezier() {
    let tf = TimingFunction::CubicBezier(0.0, 0.0, 1.0, 1.0);
    // This is approximately linear
    let mid = tf.apply(0.5);
    assert!(
        (mid - 0.5).abs() < 0.1,
        "nearly-linear bezier at 0.5 should be ~0.5, got {}",
        mid
    );
}

// ── AnimatableValue interpolation tests ──────────────────────────────

#[test]
fn test_float_interpolation() {
    let from = AnimatableValue::Float(0.0);
    let to = AnimatableValue::Float(100.0);
    let mid = from.interpolate(&to, 0.5).unwrap();
    match mid {
        AnimatableValue::Float(v) => assert!((v - 50.0).abs() < 0.01),
        _ => panic!("Expected Float"),
    }
}

#[test]
fn test_float_interpolation_at_boundaries() {
    let from = AnimatableValue::Float(10.0);
    let to = AnimatableValue::Float(20.0);

    match from.interpolate(&to, 0.0).unwrap() {
        AnimatableValue::Float(v) => assert!((v - 10.0).abs() < 0.01),
        _ => panic!("Expected Float"),
    }
    match from.interpolate(&to, 1.0).unwrap() {
        AnimatableValue::Float(v) => assert!((v - 20.0).abs() < 0.01),
        _ => panic!("Expected Float"),
    }
}

#[test]
fn test_color_interpolation() {
    let black = peniko::Color::from_rgba8(0, 0, 0, 255);
    let white = peniko::Color::from_rgba8(255, 255, 255, 255);
    let from = AnimatableValue::Color(black);
    let to = AnimatableValue::Color(white);
    let mid = from.interpolate(&to, 0.5).unwrap();
    match mid {
        AnimatableValue::Color(c) => {
            let rgba = c.to_rgba8();
            assert!(
                (rgba.r as i32 - 128).abs() <= 1,
                "R should be ~128, got {}",
                rgba.r
            );
            assert!(
                (rgba.g as i32 - 128).abs() <= 1,
                "G should be ~128, got {}",
                rgba.g
            );
            assert!(
                (rgba.b as i32 - 128).abs() <= 1,
                "B should be ~128, got {}",
                rgba.b
            );
        }
        _ => panic!("Expected Color"),
    }
}

#[test]
fn test_dimension_interpolation() {
    let from = AnimatableValue::Dimension(DimensionValue::Length(100.0));
    let to = AnimatableValue::Dimension(DimensionValue::Length(200.0));
    let mid = from.interpolate(&to, 0.5).unwrap();
    match mid {
        AnimatableValue::Dimension(DimensionValue::Length(v)) => {
            assert!((v - 150.0).abs() < 0.01);
        }
        _ => panic!("Expected Dimension::Length"),
    }
}

#[test]
fn test_incompatible_interpolation_returns_none() {
    let from = AnimatableValue::Float(1.0);
    let to = AnimatableValue::Color(peniko::Color::from_rgba8(255, 0, 0, 255));
    assert!(from.interpolate(&to, 0.5).is_none());
}

// ── ActiveTransition tests ──────────────────────────────────────────

#[test]
fn test_active_transition_value_at() {
    let t = ActiveTransition {
        property: TransitionProperty::Opacity,
        from: AnimatableValue::Float(0.0),
        to: AnimatableValue::Float(1.0),
        timing: TimingFunction::Linear,
        start_time_ms: 1000.0,
        duration_ms: 500.0,
        delay_ms: 0.0,
        reversing_adjusted_start_value: AnimatableValue::Float(0.0),
        reversing_shortening_factor: 1.0,
    };

    // Before start (during delay or before)
    match t.value_at(1000.0).unwrap() {
        AnimatableValue::Float(v) => assert!((v - 0.0).abs() < 0.01),
        _ => panic!("Expected Float"),
    }

    // Midway
    match t.value_at(1250.0).unwrap() {
        AnimatableValue::Float(v) => assert!((v - 0.5).abs() < 0.01),
        _ => panic!("Expected Float"),
    }

    // At end
    match t.value_at(1500.0).unwrap() {
        AnimatableValue::Float(v) => assert!((v - 1.0).abs() < 0.01),
        _ => panic!("Expected Float"),
    }
}

#[test]
fn test_active_transition_with_delay() {
    let t = ActiveTransition {
        property: TransitionProperty::Opacity,
        from: AnimatableValue::Float(0.0),
        to: AnimatableValue::Float(1.0),
        timing: TimingFunction::Linear,
        start_time_ms: 1000.0,
        duration_ms: 500.0,
        delay_ms: 200.0,
        reversing_adjusted_start_value: AnimatableValue::Float(0.0),
        reversing_shortening_factor: 1.0,
    };

    // During delay — should be at from value
    match t.value_at(1100.0).unwrap() {
        AnimatableValue::Float(v) => assert!(
            (v - 0.0).abs() < 0.01,
            "During delay should be 0.0, got {}",
            v
        ),
        _ => panic!("Expected Float"),
    }

    // After delay, midway through animation
    match t.value_at(1450.0).unwrap() {
        AnimatableValue::Float(v) => {
            assert!((v - 0.5).abs() < 0.01, "Midway should be 0.5, got {}", v)
        }
        _ => panic!("Expected Float"),
    }

    assert!(!t.is_complete(1100.0)); // During delay
    assert!(!t.is_complete(1450.0)); // During animation
    assert!(t.is_complete(1700.0)); // After completion
}

#[test]
fn test_active_transition_is_complete() {
    let t = ActiveTransition {
        property: TransitionProperty::Opacity,
        from: AnimatableValue::Float(0.0),
        to: AnimatableValue::Float(1.0),
        timing: TimingFunction::Linear,
        start_time_ms: 1000.0,
        duration_ms: 300.0,
        delay_ms: 0.0,
        reversing_adjusted_start_value: AnimatableValue::Float(0.0),
        reversing_shortening_factor: 1.0,
    };

    assert!(!t.is_complete(1000.0));
    assert!(!t.is_complete(1150.0));
    assert!(t.is_complete(1300.0));
    assert!(t.is_complete(2000.0));
}

// ── TransitionProperty tests ──────────────────────────────────────

#[test]
fn test_transition_property_affects_layout() {
    // Paint-only properties should NOT affect layout
    assert!(!TransitionProperty::Opacity.affects_layout());
    assert!(!TransitionProperty::BackgroundColor.affects_layout());
    assert!(!TransitionProperty::Color.affects_layout());
    assert!(!TransitionProperty::Transform.affects_layout());

    // Layout properties SHOULD affect layout
    assert!(TransitionProperty::Width.affects_layout());
    assert!(TransitionProperty::Height.affects_layout());
    assert!(TransitionProperty::PaddingTop.affects_layout());
    assert!(TransitionProperty::MarginLeft.affects_layout());
    assert!(TransitionProperty::FontSize.affects_layout());
}

// ── diff_animatable tests ──────────────────────────────────────

#[test]
fn test_diff_detects_opacity_change() {
    let mut old = ComputedStyle::default();
    let mut new = ComputedStyle::default();
    old.opacity = 1.0;
    new.opacity = 0.5;
    let changes = diff_animatable(&old, &new);
    assert_eq!(changes.len(), 1);
    assert!(matches!(changes[0].property, TransitionProperty::Opacity));
}

#[test]
fn test_diff_detects_no_change() {
    let old = ComputedStyle::default();
    let new = ComputedStyle::default();
    let changes = diff_animatable(&old, &new);
    assert!(changes.is_empty());
}

#[test]
fn test_diff_detects_background_color_change() {
    let mut old = ComputedStyle::default();
    let mut new = ComputedStyle::default();
    old.background = BackgroundValue::Color(peniko::Color::from_rgba8(255, 0, 0, 255));
    new.background = BackgroundValue::Color(peniko::Color::from_rgba8(0, 0, 255, 255));
    let changes = diff_animatable(&old, &new);
    assert!(
        changes
            .iter()
            .any(|c| matches!(c.property, TransitionProperty::BackgroundColor))
    );
}

// ── find_matching_spec tests ──────────────────────────────────────

#[test]
fn test_find_matching_spec_exact() {
    let specs = vec![TransitionSpec {
        property: TransitionProperty::Opacity,
        duration_ms: 300.0,
        delay_ms: 0.0,
        timing: TimingFunction::Ease,
    }];
    let found = find_matching_spec(&specs, TransitionProperty::Opacity);
    assert!(found.is_some());
    assert_eq!(found.unwrap().duration_ms, 300.0);
}

#[test]
fn test_find_matching_spec_all_fallback() {
    let specs = vec![TransitionSpec {
        property: TransitionProperty::All,
        duration_ms: 500.0,
        delay_ms: 0.0,
        timing: TimingFunction::EaseInOut,
    }];
    // "all" should match any property
    let found = find_matching_spec(&specs, TransitionProperty::BackgroundColor);
    assert!(found.is_some());
    assert_eq!(found.unwrap().duration_ms, 500.0);
}

#[test]
fn test_find_matching_spec_exact_takes_priority() {
    let specs = vec![
        TransitionSpec {
            property: TransitionProperty::All,
            duration_ms: 500.0,
            delay_ms: 0.0,
            timing: TimingFunction::Linear,
        },
        TransitionSpec {
            property: TransitionProperty::Opacity,
            duration_ms: 200.0,
            delay_ms: 0.0,
            timing: TimingFunction::Ease,
        },
    ];
    let found = find_matching_spec(&specs, TransitionProperty::Opacity);
    assert!(found.is_some());
    assert_eq!(found.unwrap().duration_ms, 200.0); // Exact match takes priority
}

// ── start_transitions tests ──────────────────────────────────────

#[test]
fn test_start_transitions_creates_new() {
    let mut active = HashMap::new();
    let specs = vec![TransitionSpec {
        property: TransitionProperty::All,
        duration_ms: 300.0,
        delay_ms: 0.0,
        timing: TimingFunction::Ease,
    }];
    let changes = vec![PropertyChange {
        property: TransitionProperty::Opacity,
        old_value: AnimatableValue::Float(1.0),
        new_value: AnimatableValue::Float(0.0),
    }];

    let transitioning = start_transitions(&mut active, &specs, &changes, 1000.0);
    assert_eq!(transitioning.len(), 1);
    assert!(active.contains_key(&TransitionProperty::Opacity));

    let t = &active[&TransitionProperty::Opacity];
    assert_eq!(t.duration_ms, 300.0);
    assert_eq!(t.start_time_ms, 1000.0);
}

// ── apply_value_to_style tests ──────────────────────────────────

#[test]
fn test_apply_value_opacity() {
    let mut style = ComputedStyle {
        opacity: 1.0,
        ..Default::default()
    };
    apply_value_to_style(
        &mut style,
        TransitionProperty::Opacity,
        &AnimatableValue::Float(0.5),
    );
    assert!((style.opacity - 0.5).abs() < 0.01);
}

#[test]
fn test_apply_value_background_color() {
    let mut style = ComputedStyle::default();
    let blue = peniko::Color::from_rgba8(0, 0, 255, 255);
    apply_value_to_style(
        &mut style,
        TransitionProperty::BackgroundColor,
        &AnimatableValue::Color(blue),
    );
    match &style.background {
        BackgroundValue::Color(c) => {
            let rgba = c.to_rgba8();
            assert_eq!(rgba.b, 255);
        }
        _ => panic!("Expected Color background"),
    }
}

// ── #250: `@keyframes` colour stops use the same colour parser as everything else ──

/// Build a document whose one div runs `animation: tint 1000ms linear` from the
/// given `@keyframes` body, with animations enabled from the first layout on.
/// The body's `color` is `rgb(7, 8, 9)`, the div's own `rgb(10, 20, 30)`.
fn animated_div(keyframes: &str) -> (rinch_dom::RinchDocument, rinch_core::dom::NodeId) {
    use rinch_core::dom::DomDocument;

    let mut doc = rinch_dom::RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "color: rgb(7, 8, 9)");

    let style_el = doc.create_element("style");
    let css = doc.create_text(&format!(
        "@keyframes tint {{ {keyframes} }} \
         .tint {{ animation: tint 1000ms linear; width: 10px; height: 10px; }}"
    ));
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);

    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "tint");
    doc.set_attribute(div, "style", "color: rgb(10, 20, 30)");
    doc.append_child(body, div);

    // Animations are held off until the first layout has completed (the
    // page-load guard); this test wants the very first resolve to start them.
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// The animated colour of `property` on the div's first active animation at
/// `elapsed_ms` into it.
fn animated_colour(
    doc: &rinch_dom::RinchDocument,
    div: rinch_core::dom::NodeId,
    elapsed_ms: f64,
    property: TransitionProperty,
) -> Option<peniko::Color> {
    let anim = doc
        .tree
        .active_animations
        .get(&div.0)
        .and_then(|anims| anims.first())
        .expect("the div's animation should be active after layout");
    let rinch_dom::animation::AnimationResult::Values(values) =
        anim.values_at(anim.start_time_ms + elapsed_ms)
    else {
        panic!("a running animation should yield values");
    };
    values.iter().find_map(|(prop, value)| match value {
        AnimatableValue::Color(c) if *prop == property => Some(*c),
        _ => None,
    })
}

/// #250 (B): stylo serialises an authored colour keyword verbatim, and the
/// keyframe extractor used to re-parse that text with a private 11-name table,
/// so `rebeccapurple` / `aqua` stops silently dropped out of the animation.
#[test]
fn keyframes_named_colour_stops_animate() {
    let (doc, div) =
        animated_div("from { background-color: rebeccapurple; } to { background-color: aqua; }");

    let mid = animated_colour(&doc, div, 500.0, TransitionProperty::BackgroundColor)
        .expect("both colour stops should parse, so background-color animates")
        .to_rgba8();
    // Halfway from rebeccapurple (102, 51, 153) to aqua (0, 255, 255).
    assert_eq!((mid.r, mid.g, mid.b), (51, 153, 204));
}

/// A `currentcolor` stop resolves against the element's own `color`, as before.
#[test]
fn keyframes_currentcolor_stop_uses_element_colour() {
    let (doc, div) = animated_div(
        "from { background-color: currentcolor; } to { background-color: currentcolor; }",
    );

    let mid = animated_colour(&doc, div, 500.0, TransitionProperty::BackgroundColor)
        .expect("currentcolor stops should resolve against the element's color")
        .to_rgba8();
    assert_eq!((mid.r, mid.g, mid.b), (10, 20, 30));
}

/// On `color` itself, a `currentcolor` stop is `inherit`: the parent's colour
/// (CSS Color 4 §7.1), not the element's own.
#[test]
fn keyframes_color_currentcolor_stop_uses_parent_colour() {
    let (doc, div) = animated_div("from { color: currentcolor; } to { color: currentcolor; }");

    let mid = animated_colour(&doc, div, 500.0, TransitionProperty::Color)
        .expect("currentcolor stops on `color` should resolve against the parent's colour")
        .to_rgba8();
    assert_eq!((mid.r, mid.g, mid.b), (7, 8, 9));
}

// ── A transform transition keeps the percentage part (#403) ─────────────────
//
// `TransformValue` keeps the percentage part of a `translate` outside `matrix`
// — it cannot be resolved until the element's border box is known. The
// transition machinery carried only the matrix, so for the whole duration of
// any `transition: transform` the element rendered as if every percentage
// translate in it were `0`.
//
// The two tests below are the two shipped components that broke, driven
// end-to-end: a real stylesheet, a real class change, a real tick, and the
// painted box read back.

/// The left edge of a node's painted box, in layout pixels.
fn painted_x(doc: &rinch_dom::RinchDocument, id: rinch_core::dom::NodeId) -> f64 {
    rinch_dom::paint::painted_border_box(&doc.tree, id.0, 1.0).x0
}

/// A `<div class="slider">` under `css`, laid out once with transitions armed.
fn transitioning_div(css: &str) -> (rinch_dom::RinchDocument, rinch_core::dom::NodeId) {
    use rinch_core::dom::DomDocument;

    let mut doc = rinch_dom::RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let text = doc.create_text(css);
    doc.append_child(style_el, text);
    doc.append_child(body, style_el);

    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "slider");
    doc.append_child(body, div);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// Change `div`'s class, re-resolve, and return the resulting transform
/// transition's start time — panicking if none started.
fn start_transform_transition(
    doc: &mut rinch_dom::RinchDocument,
    div: rinch_core::dom::NodeId,
    class: &str,
) -> f64 {
    use rinch_core::dom::DomDocument;

    doc.set_attribute(div, "class", class);
    doc.resolve_layout(800.0, 600.0);
    doc.tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::Transform))
        .expect("the class change should have started a transform transition")
        .start_time_ms
}

/// **The Drawer.** `styles/drawer.rs` transitions `transform` between
/// `translateX(-100%)` (closed) and `translateX(0)` (open). Both endpoints have
/// the *identity* matrix — the entire difference is the percentage coefficient
/// — so a transition carrying only the matrix interpolated identity to
/// identity: for 300ms the drawer sat at `translate(0)`, i.e. fully open, in
/// both directions. Opening, it appeared instantly with no slide; closing, it
/// stayed put and then vanished.
#[test]
fn a_percentage_only_transform_transition_actually_slides() {
    let (mut doc, div) = transitioning_div(
        ".slider { position: absolute; left: 0; top: 0; width: 200px; height: 100px; \
                   transition: transform 300ms linear; transform: translateX(-100%); } \
         .slider.open { transform: translateX(0); }",
    );

    // Closed: the 200px-wide box sits one full width to the left.
    let closed = painted_x(&doc, div);

    let start = start_transform_transition(&mut doc, div, "slider open");

    // Linear timing, so at 150ms of 300ms it is exactly half a width along.
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 150.0);
    let mid = painted_x(&doc, div);
    assert!(
        (mid - closed - 100.0).abs() < 0.5,
        "half-way through a 200px slide the box should have moved 100px, \
         went from {closed} to {mid}"
    );

    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 300.0);
    let open = painted_x(&doc, div);
    assert!(
        (open - closed - 200.0).abs() < 0.5,
        "at the end of the slide the box should have moved a full 200px, \
         went from {closed} to {open}"
    );
}

/// **The Popover.** `styles/popover.rs` transitions `transform` between
/// `translateX(-50%) translateY(-4px)` and `translateX(-50%) translateY(0)`.
/// The `-50%` is the same at both ends and is pure centring; dropping it drew
/// the popover half its own width to the right for the whole 150ms and then
/// snapped it into place.
#[test]
fn a_centring_offset_survives_a_transform_transition() {
    let (mut doc, div) = transitioning_div(
        ".slider { position: absolute; left: 300px; top: 0; width: 200px; height: 100px; \
                   transition: transform 150ms linear; \
                   transform: translateX(-50%) translateY(-4px); } \
         .slider.open { transform: translateX(-50%) translateY(0); }",
    );

    // Centred on `left: 300px`: 300 − 100.
    let closed = painted_x(&doc, div);
    assert!(
        (closed - 200.0).abs() < 0.5,
        "the centring offset should place the box at 200, got {closed}"
    );

    let start = start_transform_transition(&mut doc, div, "slider open");

    // Only the vertical offset is animating, so x must not budge. Before the
    // fix it jumped to 300 for the duration.
    for at in [0.0, 75.0, 150.0] {
        rinch_dom::transition::tick_transitions(&mut doc.tree, start + at);
        let x = painted_x(&doc, div);
        assert!(
            (x - closed).abs() < 0.5,
            "at {at}ms the centring offset should still hold x at {closed}, got {x}"
        );
    }
}

/// The interpolation itself, and the write-back that used to zero it.
#[test]
fn transform_interpolation_carries_the_percentage_coefficients() {
    let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let from = AnimatableValue::Transform(AnimatableTransform {
        matrix: identity,
        pct_translate_w: [-1.0, 0.0],
        pct_translate_h: [0.0, -0.5],
    });
    let to = AnimatableValue::Transform(AnimatableTransform {
        matrix: identity,
        pct_translate_w: [0.0, 0.0],
        pct_translate_h: [0.0, -0.5],
    });

    let mid = from
        .interpolate(&to, 0.25)
        .expect("two transforms are compatible");
    let AnimatableValue::Transform(tf) = &mid else {
        panic!("interpolating two transforms should give a transform");
    };
    assert!(
        (tf.pct_translate_w[0] + 0.75).abs() < 1e-9,
        "a quarter of the way from -100% to 0 is -75%, got {:?}",
        tf.pct_translate_w
    );
    assert!(
        (tf.pct_translate_h[1] + 0.5).abs() < 1e-9,
        "an unchanging coefficient must survive, got {:?}",
        tf.pct_translate_h
    );

    // And it reaches the computed style rather than being zeroed on write-back.
    let mut style = ComputedStyle::default();
    apply_value_to_style(&mut style, TransitionProperty::Transform, &mid);
    assert!(
        (style.transform.pct_translate_w[0] + 0.75).abs() < 1e-9,
        "the write-back dropped the percentage: {:?}",
        style.transform.pct_translate_w
    );
}

/// The third drop site: an element's *base* transform, projected into the
/// implicit keyframe stop stylo synthesises for an animation that does not
/// declare `transform` itself.
///
/// `extract_base_style_values` builds that stop from the computed style, and a
/// stop that carries only `matrix` zeroes the percentage translate for the
/// whole animation. So a centred popup with `animation: fade …` on it jumped
/// half its own width the moment the animation started — with nothing in the
/// animation touching `transform` at all. `values_at_progress` keeps a property
/// present in only one stop, so the base transform really is written back on
/// every frame (#403).
#[test]
fn an_animation_that_ignores_transform_keeps_the_base_percentage_translate() {
    use rinch_core::dom::DomDocument;

    let mut doc = rinch_dom::RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let css = doc.create_text(
        "@keyframes fade { to { opacity: 0.5; } } \
         .centred { position: absolute; left: 300px; top: 0; width: 200px; height: 100px; \
                    transform: translateX(-50%); animation: fade 1000ms linear; }",
    );
    doc.append_child(style_el, css);
    doc.append_child(body, style_el);

    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "centred");
    doc.append_child(body, div);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);

    // Centred on `left: 300px`: 300 − 100.
    let centred = painted_x(&doc, div);
    assert!(
        (centred - 200.0).abs() < 0.5,
        "the centring offset should place the box at 200 before the animation ticks, \
         got {centred}"
    );

    for at in [0.0, 250.0, 500.0, 1000.0] {
        rinch_dom::animation::tick_animations(&mut doc.tree, at);
        let x = painted_x(&doc, div);
        assert!(
            (x - 200.0).abs() < 0.5,
            "at {at}ms an opacity-only animation must not move the box, got {x}"
        );
    }
}

// ── #255 / #412: a keyframe stop that is not a plain `px` length ──
//
// `convert_declaration` used to serialise every non-colour declaration back to
// CSS text and re-parse it with a `strip_suffix("px")` mini-parser, so a stop
// written as `10em`, `1.25rem`, `5%` or `translate(50%, 0)` produced no value
// at all — the property simply vanished from the animation. It reads stylo's
// typed specified value now.

/// The animated value of `property` at `elapsed_ms` into the div's animation.
fn animated_value(
    doc: &rinch_dom::RinchDocument,
    div: rinch_core::dom::NodeId,
    elapsed_ms: f64,
    property: TransitionProperty,
) -> Option<AnimatableValue> {
    let anim = doc
        .tree
        .active_animations
        .get(&div.0)
        .and_then(|anims| anims.first())
        .expect("the div's animation should be active after layout");
    let rinch_dom::animation::AnimationResult::Values(values) =
        anim.values_at(anim.start_time_ms + elapsed_ms)
    else {
        panic!("a running animation should yield values");
    };
    values
        .iter()
        .find_map(|(prop, value)| (*prop == property).then(|| value.clone()))
}

/// The div's `font-size` is 16px and the root's is 16px, so `em` and `rem`
/// land on the same numbers here; the tests that need to tell them apart set
/// the div's own font size.
fn animated_px(
    doc: &rinch_dom::RinchDocument,
    div: rinch_core::dom::NodeId,
    ms: f64,
    property: TransitionProperty,
) -> Option<f32> {
    match animated_value(doc, div, ms, property)? {
        AnimatableValue::Dimension(DimensionValue::Length(px)) => Some(px),
        AnimatableValue::LengthPercentage(LengthPercentageValue::Length(px)) => Some(px),
        AnimatableValue::LengthPercentage(LengthPercentageValue::Zero) => Some(0.0),
        AnimatableValue::LengthPercentageAuto(LengthPercentageAutoValue::Length(px)) => Some(px),
        AnimatableValue::Float(v) => Some(v),
        other => panic!("expected a length, got {other:?}"),
    }
}

fn animated_percent(
    doc: &rinch_dom::RinchDocument,
    div: rinch_core::dom::NodeId,
    ms: f64,
    property: TransitionProperty,
) -> Option<f32> {
    match animated_value(doc, div, ms, property)? {
        AnimatableValue::Dimension(DimensionValue::Percent(p)) => Some(p),
        AnimatableValue::LengthPercentage(LengthPercentageValue::Percent(p)) => Some(p),
        AnimatableValue::LengthPercentageAuto(LengthPercentageAutoValue::Percent(p)) => Some(p),
        other => panic!("expected a percentage, got {other:?}"),
    }
}

/// `em` resolves against the element's own font size.
#[test]
fn keyframes_em_width_stops_animate() {
    let (doc, div) = animated_div("from { width: 10em; } to { width: 20em; }");
    assert_eq!(
        animated_px(&doc, div, 500.0, TransitionProperty::Width),
        Some(240.0),
        "halfway from 160px to 320px at the default 16px font size"
    );
}

/// ...and against *this element's* font size, not the root's.
#[test]
fn keyframes_em_resolves_against_the_elements_own_font_size() {
    let (doc, div) = animated_div(
        "from { width: 10em; } to { width: 10em; }          } .tint { font-size: 32px; ",
    );
    assert_eq!(
        animated_px(&doc, div, 500.0, TransitionProperty::Width),
        Some(320.0),
        "10em at a 32px font size"
    );
}

/// `rem` resolves against the root's font size, so it is unmoved by the
/// element's own.
#[test]
fn keyframes_rem_padding_stops_animate() {
    let (doc, div) = animated_div(
        "from { padding-top: 1.25rem; } to { padding-top: 2.5rem; }          } .tint { font-size: 32px; ",
    );
    assert_eq!(
        animated_px(&doc, div, 500.0, TransitionProperty::PaddingTop),
        Some(30.0),
        "halfway from 20px to 40px against the root's 16px, not the div's 32px"
    );
}

/// A percentage stop keeps its percentage — resolving it here would need the
/// containing block, and `LengthPercentageValue` can carry it as authored.
#[test]
fn keyframes_percentage_stops_animate_as_percentages() {
    let (doc, div) = animated_div("from { height: 5%; } to { height: 25%; }");
    assert_eq!(
        animated_percent(&doc, div, 500.0, TransitionProperty::Height),
        Some(0.15),
        "halfway from 5% to 25%"
    );
}

/// The half of #255 the issue does not mention: emitting `Percent` without
/// interpolation arms for it would make the stop *step* at 50% rather than
/// animate. Three samples, one per third, prove it is a ramp.
#[test]
fn a_percentage_keyframe_ramps_rather_than_stepping() {
    let (doc, div) = animated_div("from { margin-left: 0%; } to { margin-left: 30%; }");
    let at = |ms| animated_percent(&doc, div, ms, TransitionProperty::MarginLeft).unwrap();
    assert!((at(250.0) - 0.075).abs() < 1e-6, "quarter: {}", at(250.0));
    assert!((at(500.0) - 0.15).abs() < 1e-6, "half: {}", at(500.0));
    assert!(
        (at(750.0) - 0.225).abs() < 1e-6,
        "three quarters: {}",
        at(750.0)
    );
}

/// Percentage to percentage on a `<length-percentage>` property. The other
/// percentage tests use `width`/`height` (a `DimensionValue`) and `margin`
/// (a `LengthPercentageAutoValue`); this is the third value type, and it has
/// its own interpolation arm.
#[test]
fn keyframes_percentage_padding_animates() {
    let (doc, div) = animated_div("from { padding-top: 10%; } to { padding-top: 30%; }");
    let mid = animated_percent(&doc, div, 500.0, TransitionProperty::PaddingTop).unwrap();
    assert!((mid - 0.2).abs() < 1e-6, "halfway from 10% to 30%: {mid}");
}

/// A zero stop pairs with a percentage stop: `0` is unitless.
#[test]
fn keyframes_zero_to_percentage_animates() {
    let (doc, div) = animated_div("from { padding-left: 0; } to { padding-left: 20%; }");
    assert_eq!(
        animated_percent(&doc, div, 500.0, TransitionProperty::PaddingLeft),
        Some(0.1)
    );
}

/// `border-width` is the one property still read from its CSS serialisation
/// (stylo's `BorderSideWidth` hides its `LineWidth` behind a private field), so
/// it gets its own coverage: `em` works, and so do the keyword widths.
#[test]
fn keyframes_border_width_em_and_keyword_stops_animate() {
    let (doc, div) = animated_div("from { border-top-width: 1em; } to { border-top-width: 2em; }");
    assert_eq!(
        animated_px(&doc, div, 500.0, TransitionProperty::BorderTopWidth),
        Some(24.0),
        "halfway from 16px to 32px"
    );

    let (doc, div) =
        animated_div("from { border-top-width: thin; } to { border-top-width: thick; }");
    // A quarter of the way, not half: thin (1px), medium (3px) and thick (5px)
    // are evenly spaced, so a midpoint sample reads 3px whether the three
    // keywords are distinguished or all collapsed onto medium.
    assert_eq!(
        animated_px(&doc, div, 250.0, TransitionProperty::BorderTopWidth),
        Some(2.0),
        "a quarter of the way from thin (1px) to thick (5px)"
    );
}

/// `font-size` in `rem` is exact — its base is the root. `em` and `%` are
/// declined rather than resolved against the element's own (already-resolved)
/// size, which would be the wrong base.
#[test]
fn keyframes_font_size_rem_animates_and_em_declines() {
    let (doc, div) = animated_div("from { font-size: 1rem; } to { font-size: 2rem; }");
    assert_eq!(
        animated_px(&doc, div, 500.0, TransitionProperty::FontSize),
        Some(24.0)
    );

    let (doc, div) = animated_div("from { font-size: 1em; } to { font-size: 2em; }");
    assert!(
        animated_value(&doc, div, 500.0, TransitionProperty::FontSize).is_none(),
        "em on font-size needs the parent's size, which the extractor lacks"
    );
}

// ── transform ──

fn animated_transform(
    doc: &rinch_dom::RinchDocument,
    div: rinch_core::dom::NodeId,
    ms: f64,
) -> ([f64; 6], [f64; 2], [f64; 2]) {
    match animated_value(doc, div, ms, TransitionProperty::Transform) {
        Some(AnimatableValue::TransformComponents {
            ops,
            pct_translate_w,
            pct_translate_h,
        }) => (
            rinch_dom::transition::compose_matrices(&ops),
            pct_translate_w,
            pct_translate_h,
        ),
        other => panic!("expected transform components, got {other:?}"),
    }
}

/// #412: a percentage translate in an authored `@keyframes` stop. The old
/// mini-parser routed `translate(50%, 0)` through `strip_suffix("px")`, got
/// `None`, produced an empty op list and dropped the whole transform.
#[test]
fn keyframes_percentage_translate_populates_the_pct_channel() {
    let (doc, div) =
        animated_div("from { transform: translate(0%, 0); } to { transform: translate(50%, 0); }");
    // Halfway through 0% -> 50%.
    let (m, pct_w, pct_h) = animated_transform(&doc, div, 500.0);
    assert!(
        (pct_w[0] - 0.25).abs() < 1e-6 && pct_w[1].abs() < 1e-9,
        "the x translate is a fraction of the border-box width: {pct_w:?}"
    );
    assert_eq!(pct_h, [0.0, 0.0]);
    assert_eq!(
        (m[4], m[5]),
        (0.0, 0.0),
        "no pixel part — the whole translate lives in the percentage channel"
    );
}

/// The channel interpolates too, rather than snapping at the end.
#[test]
fn a_percentage_translate_ramps() {
    let (doc, div) = animated_div(
        "from { transform: translate(0%, 0); } to { transform: translate(40%, 20%); }",
    );
    let (_, pct_w, pct_h) = animated_transform(&doc, div, 500.0);
    assert!((pct_w[0] - 0.2).abs() < 1e-6, "half of 40%: {pct_w:?}");
    assert!((pct_h[1] - 0.1).abs() < 1e-6, "half of 20%: {pct_h:?}");
}

/// A `calc()` angle. Stylo folds it into `AngleDimension::Deg`, so the typed
/// read handles it; `strip_suffix("deg")` saw `calc(45deg)` and gave up.
#[test]
fn keyframes_calc_angle_rotates() {
    let (doc, div) = animated_div(
        "from { transform: rotate(0deg); } to { transform: rotate(calc(30deg + 60deg)); }",
    );
    // Halfway through 0deg -> 90deg is 45deg.
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    let root_half = std::f64::consts::FRAC_1_SQRT_2;
    assert!(
        (m[0] - root_half).abs() < 1e-6 && (m[1] - root_half).abs() < 1e-6,
        "{m:?}"
    );
}

/// `turn` was never broken — the issue's example is wrong about that — and must
/// stay unbroken by the rewrite.
#[test]
fn keyframes_turn_angle_still_rotates() {
    let (doc, div) =
        animated_div("from { transform: rotate(0turn); } to { transform: rotate(0.25turn); }");
    // Halfway through 0 -> a quarter turn is 45deg.
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    let root_half = std::f64::consts::FRAC_1_SQRT_2;
    assert!(
        (m[0] - root_half).abs() < 1e-6 && (m[1] - root_half).abs() < 1e-6,
        "{m:?}"
    );
}

/// An `em` translate, which the px-only parser also dropped.
#[test]
fn keyframes_em_translate_animates() {
    let (doc, div) =
        animated_div("from { transform: translateX(0); } to { transform: translateX(2em); }");
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    assert!((m[4] - 16.0).abs() < 1e-6, "half of 2em at 16px: {m:?}");
}

// ── the shapes every shipped component animates, which must not regress ──
//
// Seven of the nine `@keyframes` blocks in the workspace are `rotate(Ndeg)`;
// the other two are `scale()`/`scaleY()` plus `opacity`. They are the whole
// regression surface of the transform rewrite, so each is pinned here.

#[test]
fn keyframes_spin_still_rotates() {
    let (doc, div) =
        animated_div("from { transform: rotate(0deg); } to { transform: rotate(360deg); }");
    let (m, _, _) = animated_transform(&doc, div, 250.0);
    // A quarter of the way is 90deg.
    assert!(m[0].abs() < 1e-6 && (m[1] - 1.0).abs() < 1e-6, "{m:?}");
}

#[test]
fn keyframes_scale_and_opacity_still_animate() {
    let (doc, div) = animated_div(
        "0% { transform: scale(0); opacity: 0.5; } 100% { transform: scale(1); opacity: 1; }",
    );
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    assert!(
        (m[0] - 0.5).abs() < 1e-6 && (m[3] - 0.5).abs() < 1e-6,
        "{m:?}"
    );
    assert_eq!(
        animated_px(&doc, div, 500.0, TransitionProperty::Opacity),
        Some(0.75)
    );
}

#[test]
fn keyframes_scale_y_still_animates() {
    let (doc, div) = animated_div("0% { transform: scaleY(0.4); } 100% { transform: scaleY(1); }");
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    assert!((m[0] - 1.0).abs() < 1e-6, "x untouched: {m:?}");
    assert!((m[3] - 0.7).abs() < 1e-6, "halfway from 0.4 to 1: {m:?}");
}

/// ...and `scaleX` the other axis. No shipped component uses it, which is
/// exactly why it needs a pin of its own: a review mutant that made `scaleX`
/// write the y axis survived the whole suite.
#[test]
fn keyframes_scale_x_still_animates() {
    let (doc, div) = animated_div("0% { transform: scaleX(0.4); } 100% { transform: scaleX(1); }");
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    assert!((m[3] - 1.0).abs() < 1e-6, "y untouched: {m:?}");
    assert!((m[0] - 0.7).abs() < 1e-6, "halfway from 0.4 to 1: {m:?}");
}

/// `translate3d` is spelled out in the extractor *because* dropping it to the
/// identity arm would silently lose a whole translation — but nothing pinned
/// that arm, and a review mutant that deleted it survived the whole suite.
/// Both channels: the px part rides the op, the percentage part rides the
/// #212 linear form.
///
/// This pins only the 2D projection. The z component is dropped by design,
/// and every other 3D operation still flattens to identity — that is #405,
/// which this test makes no claim about.
#[test]
fn keyframes_translate3d_keeps_its_2d_translation() {
    let (doc, div) = animated_div(
        "from { transform: translate3d(0, 0, 0); } to { transform: translate3d(20px, 50%, 7px); }",
    );
    let (m, _, pct_h) = animated_transform(&doc, div, 500.0);
    assert!((m[4] - 10.0).abs() < 1e-6, "half of 20px: {m:?}");
    assert!(
        (pct_h[1] - 0.25).abs() < 1e-6,
        "half of 50% of the height: {pct_h:?}"
    );
}

/// `transform: none` is the identity, and interpolates component-wise against
/// a `scale()` stop rather than falling back to matrix interpolation.
#[test]
fn keyframes_transform_none_is_the_identity() {
    let (doc, div) = animated_div("from { transform: none; } to { transform: scale(3); }");
    let (m, _, _) = animated_transform(&doc, div, 500.0);
    assert!(
        (m[0] - 2.0).abs() < 1e-6 && (m[3] - 2.0).abs() < 1e-6,
        "{m:?}"
    );
}

/// The gaps that stay gaps, named so a future reader knows they are declined
/// rather than forgotten: a mixed `calc()`, a viewport unit, an `ex`.
#[test]
fn keyframes_values_needing_more_than_a_font_size_are_declined() {
    for stop in ["calc(1rem + 2px)", "10vw", "3ex", "5cqw"] {
        // Both stops, so a surviving value cannot come from the other one.
        let (doc, div) = animated_div(&format!(
            "from {{ width: {stop}; }} to {{ width: {stop}; }}"
        ));
        // The animation may not exist at all — an entirely empty stop list is
        // not registered — which is also "no value for width".
        let width = doc
            .tree
            .active_animations
            .get(&div.0)
            .and_then(|anims| anims.first())
            .and_then(|anim| match anim.values_at(anim.start_time_ms + 500.0) {
                rinch_dom::animation::AnimationResult::Values(values) => values
                    .iter()
                    .find(|(prop, _)| *prop == TransitionProperty::Width)
                    .map(|(_, v)| v.clone()),
                _ => None,
            });
        assert!(
            width.is_none(),
            "`width: {stop}` should be declined, not guessed at — got {width:?}"
        );
    }
}

/// **A finished layout-affecting transition must reach the layout, not just
/// the computed style** (#489).
///
/// `RinchDocument::tick_transitions` rebuilds the Taffy style from the
/// interpolated values, but it used to leave `tree.layout_dirty` alone — and
/// `resolve_layout` early-returns on `!layout_dirty`. So the *end* value of a
/// `width` transition landed in `computed_style` and in the Taffy style and
/// then sat there: the node kept whatever box the last relayout that happened
/// for some *other* reason had given it. In UI Zoo's Inputs section that froze
/// the checkbox/switch/select size cards at an arbitrary point on their curves,
/// at a different point on every run, and cascaded ±1px through 112 of the
/// section's 1011 boxes.
///
/// Both resolves below use the **same** viewport on purpose: a viewport change
/// sets `layout_dirty` by itself and would make this pass either way.
#[test]
fn a_finished_width_transition_reaches_the_layout() {
    use rinch_core::dom::DomDocument;

    let (mut doc, div) = transitioning_div(
        ".slider { width: 100px; height: 40px; transition: width 150ms linear; } \
         .slider.wide { width: 200px; }",
    );
    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.width,
        100.0,
        "the pre-change width should be the declared 100px"
    );

    doc.set_attribute(div, "class", "slider wide");
    doc.resolve_layout(800.0, 600.0);

    // Back-date the running transition past its own duration so the next tick
    // completes it. This is what keeps the test clock-free —
    // `RinchDocument::tick_transitions` reads `SystemTime::now()` itself.
    let running = doc
        .tree
        .active_transitions
        .get_mut(&div.0)
        .expect("the class change should have started a width transition");
    for t in running.values_mut() {
        t.start_time_ms -= 10_000.0;
    }

    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.width,
        200.0,
        "a completed width transition must leave the box at its end value"
    );
}

/// A `<div class="grow">` with a text child, under `css`, laid out once with
/// animations armed. Separate from [`animated_div`], which hard-codes a colour
/// animation on a 10x10 box.
fn animated_width_div(css: &str) -> (rinch_dom::RinchDocument, rinch_core::dom::NodeId) {
    use rinch_core::dom::DomDocument;

    let mut doc = rinch_dom::RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let text = doc.create_text(css);
    doc.append_child(style_el, text);
    doc.append_child(body, style_el);

    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "grow");
    doc.append_child(body, div);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// The **animation** half of [`a_finished_width_transition_reaches_the_layout`]
/// (#489). `tick_animations` is the second `taffy.set_style` site that used to
/// leave `tree.layout_dirty` alone, and it needs its own pin: a fix applied to
/// only one of the two sites passes the transition test and fails this one.
///
/// `forwards` is what makes the end state observable — without a fill mode the
/// completed animation is dropped having applied nothing.
#[test]
fn a_finished_width_animation_reaches_the_layout() {
    let (mut doc, div) = animated_width_div(
        "@keyframes grow { from { width: 100px; } to { width: 200px; } } \
         .grow { animation: grow 150ms linear forwards; width: 100px; height: 40px; }",
    );
    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.width,
        100.0,
        "at the start of the animation the box should be at the `from` width"
    );

    // Back-date past the duration so the next tick lands in the `forwards`
    // fill, clock-free — `RinchDocument::tick_animations` reads
    // `SystemTime::now()` itself.
    for anim in doc
        .tree
        .active_animations
        .get_mut(&div.0)
        .expect("the first layout should have started the animation")
    {
        anim.start_time_ms -= 10_000.0;
    }

    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.width,
        200.0,
        "a completed `forwards` animation must leave the box at its end value"
    );
}

/// A `<div class="{class}">` holding a paragraph of text, under `css`, laid out
/// once with transitions armed.
fn texted_div(css: &str, class: &str) -> (rinch_dom::RinchDocument, rinch_core::dom::NodeId) {
    use rinch_core::dom::DomDocument;

    let mut doc = rinch_dom::RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let sheet = doc.create_text(css);
    doc.append_child(style_el, sheet);
    doc.append_child(body, style_el);

    let div = doc.create_element("div");
    doc.set_attribute(div, "class", class);
    let text = doc.create_text("Hello world, wrap me please, several words here");
    doc.append_child(div, text);
    doc.append_child(body, div);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// The **text** case of #489: a finished `font-size` transition must reach the
/// inline layout, not just `computed_style`.
///
/// This is the same missing `layout_dirty` — measured, it left a 10px box at
/// its 10px height forever while `computed_style` said 40px — but it is worth
/// its own pin because it travels a different route: the size change lands in
/// the IFC's Parley measurement rather than in a Taffy `size` field, and the
/// question of whether `tick_transitions` additionally owes the tree an
/// `ifc_dirty` was open until this measured that it does not.
///
/// The oracle is a **second document** declaring the end size directly, never a
/// glyph-derived literal: a height measured from text is a pin on whichever
/// fonts the machine happens to have, and CI's differ from a developer's.
#[test]
fn a_finished_font_size_transition_reaches_the_inline_layout() {
    use rinch_core::dom::DomDocument;

    const CSS: &str = ".t { width: 200px; font-size: 10px; line-height: 1.5; \
                       transition: font-size 150ms linear; } \
                       .t.big { font-size: 40px; }";

    let (reference, ref_div) = texted_div(CSS, "t big");
    let want = reference.tree.get(ref_div.0).unwrap().layout.height;

    let (mut doc, div) = texted_div(CSS, "t");
    let small = doc.tree.get(div.0).unwrap().layout.height;
    assert!(
        want > small,
        "the 40px reference ({want}) must be taller than the 10px box ({small}) \
         or this fixture is parked on a fixed point"
    );

    doc.set_attribute(div, "class", "t big");
    doc.resolve_layout(800.0, 600.0);
    for t in doc
        .tree
        .active_transitions
        .get_mut(&div.0)
        .expect("the class change should have started a font-size transition")
        .values_mut()
    {
        t.start_time_ms -= 10_000.0;
    }

    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);

    let got = doc.tree.get(div.0).unwrap().layout.height;
    assert!(
        (got - want).abs() < 0.5,
        "a completed font-size transition should leave the box the height a \
         directly-declared 40px box has: want {want}, got {got} \
         (the 10px box was {small})"
    );
}

// ── #652: css-transitions-1 §3, "Starting of transitions" ────────────────────
//
// `start_transitions` used to insert a fresh `ActiveTransition` for every
// property in the diff, unconditionally. Its caller diffs the node's
// `computed_style` — which holds the *interpolated* value while a transition
// runs — against the freshly resolved target, so **every** restyle of a node
// mid-transition saw a change and restarted the transition with a new clock.
// A declared 150ms animation then ran for as long as restyles kept arriving.
//
// §3 says the opposite: a running transition whose end value still equals the
// after-change value is left alone. The same section says a *reversal* is
// shortened in proportion to how far the transition it cancels had got.

/// The `width` in a node's computed style, in px.
fn computed_width_px(doc: &rinch_dom::RinchDocument, id: rinch_core::dom::NodeId) -> f32 {
    match doc.tree.get(id.0).unwrap().computed_style.width {
        DimensionValue::Length(px) => px,
        other => panic!("expected a length width, got {other:?}"),
    }
}

/// The node's running `width` transition, or `None` if it has none.
fn width_transition(
    doc: &rinch_dom::RinchDocument,
    id: rinch_core::dom::NodeId,
) -> Option<&ActiveTransition> {
    doc.tree
        .active_transitions
        .get(&id.0)
        .and_then(|m| m.get(&TransitionProperty::Width))
}

/// Move the node's running `width` transition `by_ms` into the past, so that
/// "now" — which `resolve_layout` reads off the wall clock itself — sits that
/// far along the curve. Returns the back-dated start time.
fn backdate_width_transition(
    doc: &mut rinch_dom::RinchDocument,
    id: rinch_core::dom::NodeId,
    by_ms: f64,
) -> f64 {
    let t = doc
        .tree
        .active_transitions
        .get_mut(&id.0)
        .and_then(|m| m.get_mut(&TransitionProperty::Width))
        .expect("the class change should have started a width transition");
    t.start_time_ms -= by_ms;
    t.start_time_ms
}

/// A `width` transition from `css`, back-dated so the wall clock now reads
/// `elapsed_ms` into it. Returns the document, the node, and the (back-dated)
/// start time.
fn running_width_transition(
    css: &str,
    elapsed_ms: f64,
) -> (rinch_dom::RinchDocument, rinch_core::dom::NodeId, f64) {
    use rinch_core::dom::DomDocument;

    let (mut doc, div) = transitioning_div(css);
    doc.set_attribute(div, "class", "slider wide");
    doc.resolve_layout(800.0, 600.0);
    let start = backdate_width_transition(&mut doc, div, elapsed_ms);
    (doc, div, start)
}

const LINEAR_20_TO_30: &str = ".slider { width: 20px; height: 40px; \
     transition: width 150ms linear; } \
     .slider.wide { width: 30px; }";

// `150ms` is `0.15s`, which does not survive an f32 round trip: the spec comes
// back as 150.00000596ms. So "at the declared duration" is read one millisecond
// past it rather than on the nose — an assertion sitting exactly on 150.0 fails
// against correct code.
const JUST_PAST_150: f64 = 151.0;

fn px(v: f32) -> AnimatableValue {
    AnimatableValue::Dimension(DimensionValue::Length(v))
}

fn px_of(v: &AnimatableValue) -> f32 {
    match v {
        AnimatableValue::Dimension(DimensionValue::Length(px)) => *px,
        other => panic!("expected a length, got {other:?}"),
    }
}

fn width_spec(duration_ms: f64, delay_ms: f64) -> TransitionSpec {
    TransitionSpec {
        property: TransitionProperty::Width,
        duration_ms,
        delay_ms,
        timing: TimingFunction::Linear,
    }
}

fn width_change(old: f32, new: f32) -> PropertyChange {
    PropertyChange {
        property: TransitionProperty::Width,
        old_value: px(old),
        new_value: px(new),
    }
}

/// A map holding one running `width` transition, `from` → `to`, started at
/// `start_time_ms` over `spec`.
fn running(
    from: f32,
    to: f32,
    spec: &TransitionSpec,
    start_time_ms: f64,
) -> HashMap<TransitionProperty, ActiveTransition> {
    let mut active = HashMap::new();
    active.insert(
        TransitionProperty::Width,
        ActiveTransition::starting(
            TransitionProperty::Width,
            px(from),
            px(to),
            spec,
            start_time_ms,
        ),
    );
    active
}

/// **(a) The bug.** An unrelated restyle — here a `data-` attribute nobody's
/// selector reads — re-resolves the node, and the resolved `width` is still the
/// same 30px the running transition is already heading for. The transition must
/// be left exactly as it is.
///
/// The clock assertion is exact and jitter-free: the restyle happens at some
/// wall-clock time strictly after the back-dated start, so a restart is
/// *always* observable as a different `start_time_ms`.
///
/// Kills two mutants: the unconditional `insert` this replaced, and a guard
/// that compares the running transition's `from` (20) rather than its `to` (30)
/// against the after-change value.
#[test]
fn an_unrelated_restyle_leaves_a_running_transition_alone() {
    use rinch_core::dom::DomDocument;

    let (mut doc, div, start) = running_width_transition(LINEAR_20_TO_30, 50.0);

    doc.set_attribute(div, "data-probe", "1");
    doc.resolve_layout(800.0, 600.0);

    let t = width_transition(&doc, div).expect("the transition must still be running");
    assert_eq!(
        t.start_time_ms, start,
        "an unrelated restyle must not reset the transition's clock"
    );
    assert!(
        (px_of(&t.from) - 20.0).abs() < 0.001,
        "the transition must still start from its original 20px, got {:?}",
        t.from
    );

    // Linear, so 100ms into 150ms of 20 → 30 is exactly 26.667px. Before the
    // fix the restart at ~50ms left it at ~25.6 and it arrived 50ms late.
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 100.0);
    let w = computed_width_px(&doc, div);
    assert!(
        (w - 26.667).abs() < 0.05,
        "two thirds of the way through a 20 → 30 linear transition the width \
         should be 26.667, got {w}"
    );
}

/// **(a2) The leave-alone path must still report the property as
/// transitioning.** The caller assigns the whole after-change style into
/// `computed_style` and then writes the interpolated value back for exactly the
/// properties `start_transitions` returns. A guard that leaves the transition
/// alone but drops the property from that list snaps the box to its end value
/// for a frame — and one frame is all #489 needed.
///
/// Kills the mutant that `continue`s without pushing onto `transitioning`,
/// which is what the issue's own fix sketch proposed.
#[test]
fn the_leave_alone_path_keeps_the_interpolated_value_in_the_computed_style() {
    use rinch_core::dom::DomDocument;

    let (mut doc, div, _start) = running_width_transition(LINEAR_20_TO_30, 50.0);

    doc.set_attribute(div, "data-probe", "1");
    doc.resolve_layout(800.0, 600.0);

    // ~50ms into 150ms of 20 → 30 is ~23.3px; the restyle itself costs a few ms
    // of wall clock, so allow a little more progress but nothing near 30.
    let w = computed_width_px(&doc, div);
    assert!(
        (23.0..24.5).contains(&w),
        "the restyle must leave the interpolated width in place, not the 30px \
         target it resolved, got {w}"
    );
}

/// **The user-visible contract** (the fixture the issue asks for): a declared
/// 150ms transition arrives in 150ms however many restyles happen while it
/// runs. Ten of them here; before the fix each reset the clock and the box was
/// still ~27.8px at t=150.
#[test]
fn repeated_restyles_do_not_extend_a_transitions_declared_duration() {
    use rinch_core::dom::DomDocument;

    let (mut doc, div, start) = running_width_transition(LINEAR_20_TO_30, 50.0);

    for i in 0..10 {
        doc.set_attribute(div, "data-probe", &i.to_string());
        doc.resolve_layout(800.0, 600.0);
    }

    assert!(
        width_transition(&doc, div)
            .expect("still running")
            .is_complete(start + JUST_PAST_150),
        "a 150ms transition must be complete 150ms after it started, however \
         many restyles happened in between"
    );

    rinch_dom::transition::tick_transitions(&mut doc.tree, start + JUST_PAST_150);
    let w = computed_width_px(&doc, div);
    assert!(
        (w - 30.0).abs() < 0.001,
        "at its declared duration the transition must be at its end value, got {w}"
    );
}

/// The same, with `ease` — the timing function the issue measured. `ease` is
/// slow near t=0, so every restart advanced the value by a sliver and the box
/// crawled toward its target asymptotically. Nothing here depends on the shape
/// of the curve: both assertions are about *when* it finishes.
#[test]
fn repeated_restyles_do_not_extend_an_ease_transition_either() {
    use rinch_core::dom::DomDocument;

    let (mut doc, div, start) = running_width_transition(
        ".slider { width: 20px; height: 40px; transition: all 150ms ease; } \
         .slider.wide { width: 30px; }",
        10.0,
    );

    for i in 0..10 {
        doc.set_attribute(div, "data-probe", &i.to_string());
        doc.resolve_layout(800.0, 600.0);
    }

    assert!(
        width_transition(&doc, div)
            .expect("still running")
            .is_complete(start + JUST_PAST_150),
        "an `ease` transition must be complete at its declared duration too"
    );

    rinch_dom::transition::tick_transitions(&mut doc.tree, start + JUST_PAST_150);
    let w = computed_width_px(&doc, div);
    assert!(
        (w - 30.0).abs() < 0.001,
        "an `ease` transition must also arrive at its declared duration, got {w}"
    );
    assert!(
        width_transition(&doc, div).is_none(),
        "a completed transition must have been removed by the tick"
    );
}

/// **(e) A restyle during a transition's `delay`.** The clock has started but
/// nothing has moved, so a restart is invisible at the moment it happens and
/// shows up only as a late arrival: before the fix, ticking at 175ms — 75ms
/// into a 150ms curve that began after a 100ms delay — still found the box at
/// its start value, because the restarted delay had not run out.
#[test]
fn an_unrelated_restyle_during_a_transitions_delay_does_not_restart_it() {
    use rinch_core::dom::DomDocument;

    let (mut doc, div, start) = running_width_transition(
        ".slider { width: 20px; height: 40px; \
         transition: width 150ms linear 100ms; } \
         .slider.wide { width: 30px; }",
        50.0,
    );

    doc.set_attribute(div, "data-probe", "1");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        width_transition(&doc, div)
            .expect("still running")
            .start_time_ms,
        start,
        "a restyle inside the delay must not reset the clock either"
    );

    // 100ms delay, then half of a 150ms linear ramp: 25px.
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 175.0);
    let w = computed_width_px(&doc, div);
    assert!(
        (w - 25.0).abs() < 0.05,
        "half way through the ramp that follows the delay the width should be \
         25, got {w}"
    );
}

/// **(d) A transition that has run past its duration but has not been ticked
/// away yet.** `resolve_layout` runs before `tick_transitions` in a frame, so
/// the map really does hold finished transitions at diff time, and the last
/// interpolated value written into `computed_style` is a little short of the
/// end value — a genuine diff.
///
/// Kills the `from`-instead-of-`to` mutant on its own: `from` (20) differs from
/// the after-change 30, so that comparison restarts a transition that has
/// already finished and gives it a fresh 150ms of doing nothing.
#[test]
fn a_finished_but_unticked_transition_is_not_restarted() {
    let spec = width_spec(150.0, 0.0);
    let mut active = running(20.0, 30.0, &spec, 1000.0);

    // 1200ms — 50ms past the end of a 150ms transition that began at 1000.
    let transitioning = start_transitions(
        &mut active,
        std::slice::from_ref(&spec),
        &[width_change(29.9, 30.0)],
        1200.0,
    );

    assert_eq!(
        active[&TransitionProperty::Width].start_time_ms,
        1000.0,
        "a finished transition must be left alone"
    );
    assert_eq!(
        transitioning,
        vec![TransitionProperty::Width],
        "it is still the transition's value that belongs in the computed style"
    );
}

/// **(b) A target that genuinely changed mid-flight restarts from the current
/// interpolated value, with a fresh clock and the full declared duration.**
/// This is the behaviour the leave-alone guard must not swallow.
///
/// Kills the "never restart" mutant — a guard that leaves *every* running
/// transition alone.
#[test]
fn a_changed_target_restarts_the_transition_from_where_it_is() {
    let spec = width_spec(150.0, 0.0);
    let mut active = running(20.0, 30.0, &spec, 1000.0);

    // Half way along, the target becomes 50px — neither the 30 it was heading
    // for nor the 20 it started from, so this is a retarget, not a reversal.
    start_transitions(
        &mut active,
        std::slice::from_ref(&spec),
        &[width_change(25.0, 50.0)],
        1075.0,
    );

    let t = &active[&TransitionProperty::Width];
    assert_eq!(
        t.start_time_ms, 1075.0,
        "a changed target restarts the clock"
    );
    assert_eq!(
        t.duration_ms, 150.0,
        "and gets the declared duration in full"
    );
    assert_eq!(
        t.reversing_shortening_factor, 1.0,
        "a retarget is not a reversal and is not shortened"
    );
    assert!(
        (px_of(&t.from) - 25.0).abs() < 0.001,
        "it must restart from the current interpolated 25px, got {:?}",
        t.from
    );
    assert!(
        (px_of(&t.to) - 50.0).abs() < 0.001,
        "heading for the new 50px, got {:?}",
        t.to
    );
}

/// **§3 step 5.1.** A running transition that has already arrived at the new
/// target is cancelled outright, and the property is *not* reported as
/// transitioning — the after-change value the caller already wrote is the value
/// the box is at.
///
/// Kills the mutant that omits step 5.1 and starts a 25px → 25px transition
/// that then occupies the node for a further 150ms of ticks.
#[test]
fn a_transition_that_has_already_reached_the_new_target_is_cancelled() {
    let spec = width_spec(150.0, 0.0);
    let mut active = running(20.0, 30.0, &spec, 1000.0);

    let transitioning = start_transitions(
        &mut active,
        std::slice::from_ref(&spec),
        &[width_change(25.0, 25.0)],
        1075.0,
    );

    assert!(
        active.is_empty(),
        "a transition that has reached its new target must be cancelled, got {active:?}"
    );
    assert!(
        transitioning.is_empty(),
        "and the after-change value must be allowed to stand"
    );
}

/// **(c) §3 step 5.3: a reversal is shortened.** Turning a 150ms slide around
/// half way through is 75ms of travel back, not another 150ms — so it lands on
/// its old start value at exactly the moment it would have reached its end
/// value.
///
/// Kills the mutant that reverses with a shortening factor of 1 (the plain
/// step-5.2 restart), which gives the reversal a full 150ms.
#[test]
fn reversing_a_transition_half_way_through_takes_half_as_long() {
    let spec = width_spec(150.0, 0.0);
    let mut active = running(20.0, 30.0, &spec, 1000.0);

    // Back to 20 at t=1075: linear, so progress is 0.5 and the box is at 25.
    start_transitions(
        &mut active,
        std::slice::from_ref(&spec),
        &[width_change(25.0, 20.0)],
        1075.0,
    );

    let t = &active[&TransitionProperty::Width];
    assert_eq!(t.start_time_ms, 1075.0);
    assert!(
        (t.reversing_shortening_factor - 0.5).abs() < 0.001,
        "half way through, the factor is the progress: got {}",
        t.reversing_shortening_factor
    );
    assert!(
        (t.duration_ms - 75.0).abs() < 0.001,
        "a reversal half way through a 150ms transition lasts 75ms, got {}",
        t.duration_ms
    );
    assert!(
        (px_of(&t.from) - 25.0).abs() < 0.001,
        "from the current 25px"
    );
    assert!(
        (px_of(&t.to) - 20.0).abs() < 0.001,
        "back to the original 20px"
    );
    assert!(
        (px_of(&t.reversing_adjusted_start_value) - 30.0).abs() < 0.001,
        "a reversal would itself reverse back to the value it cancelled heading for"
    );
    assert!(
        t.is_complete(1150.0),
        "it must land at 1150 — the moment the transition it cancelled would \
         have arrived"
    );
}

/// **Reversing a reversal.** The shortening factor folds the running
/// transition's own factor back in — `|f·progress + (1 − f)|` — so a second
/// turn is measured against the *declared* duration rather than compounding
/// down to nothing.
///
/// Kills the mutant that drops the `(1 − f)` term: it would give 0.25 and a
/// 37.5ms duration here, where the spec gives 0.75 and 112.5ms. That term is
/// invisible on the first reversal, where `f` is 1 and it is zero — the
/// fixed point this fixture exists to sample off.
#[test]
fn reversing_a_reversal_is_measured_against_the_declared_duration() {
    let spec = width_spec(150.0, 0.0);
    let mut active = running(20.0, 30.0, &spec, 1000.0);

    // First reversal at 1075: 25px → 20px over 75ms, factor 0.5.
    start_transitions(
        &mut active,
        std::slice::from_ref(&spec),
        &[width_change(25.0, 20.0)],
        1075.0,
    );
    // Second at 1112.5 — half way through those 75ms, so the box is at 22.5px
    // and heading back to 30, which is the reversal's reversing-adjusted start
    // value.
    start_transitions(
        &mut active,
        std::slice::from_ref(&spec),
        &[width_change(22.5, 30.0)],
        1112.5,
    );

    let t = &active[&TransitionProperty::Width];
    assert!(
        (t.reversing_shortening_factor - 0.75).abs() < 0.001,
        "|0.5·0.5 + (1 − 0.5)| = 0.75, got {}",
        t.reversing_shortening_factor
    );
    assert!(
        (t.duration_ms - 112.5).abs() < 0.001,
        "0.75 of the declared 150ms, got {}",
        t.duration_ms
    );
    assert!(
        (px_of(&t.from) - 22.5).abs() < 0.001,
        "from the current 22.5px"
    );
    assert!((px_of(&t.to) - 30.0).abs() < 0.001, "back toward 30px");
    assert!(
        (px_of(&t.reversing_adjusted_start_value) - 20.0).abs() < 0.001,
        "and a third turn would head back to 20 again"
    );
}
