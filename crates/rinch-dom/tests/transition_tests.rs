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
