//! Unit tests for the CSS transition engine.

use rinch_dom::transition::*;
use rinch_dom::computed_style::*;
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
    assert!(val < 0.25, "ease-in at t=0.25 should be < 0.25, got {}", val);
}

#[test]
fn test_ease_out_starts_fast() {
    let tf = TimingFunction::EaseOut;
    // EaseOut starts fast — at t=0.25, value should be > 0.25
    let val = tf.apply(0.25);
    assert!(val > 0.25, "ease-out at t=0.25 should be > 0.25, got {}", val);
}

#[test]
fn test_custom_cubic_bezier() {
    let tf = TimingFunction::CubicBezier(0.0, 0.0, 1.0, 1.0);
    // This is approximately linear
    let mid = tf.apply(0.5);
    assert!((mid - 0.5).abs() < 0.1, "nearly-linear bezier at 0.5 should be ~0.5, got {}", mid);
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
            assert!((rgba.r as i32 - 128).abs() <= 1, "R should be ~128, got {}", rgba.r);
            assert!((rgba.g as i32 - 128).abs() <= 1, "G should be ~128, got {}", rgba.g);
            assert!((rgba.b as i32 - 128).abs() <= 1, "B should be ~128, got {}", rgba.b);
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
        AnimatableValue::Float(v) => assert!((v - 0.0).abs() < 0.01, "During delay should be 0.0, got {}", v),
        _ => panic!("Expected Float"),
    }

    // After delay, midway through animation
    match t.value_at(1450.0).unwrap() {
        AnimatableValue::Float(v) => assert!((v - 0.5).abs() < 0.01, "Midway should be 0.5, got {}", v),
        _ => panic!("Expected Float"),
    }

    assert!(!t.is_complete(1100.0)); // During delay
    assert!(!t.is_complete(1450.0)); // During animation
    assert!(t.is_complete(1700.0));  // After completion
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
    assert!(changes.iter().any(|c| matches!(c.property, TransitionProperty::BackgroundColor)));
}

// ── find_matching_spec tests ──────────────────────────────────────

#[test]
fn test_find_matching_spec_exact() {
    let specs = vec![
        TransitionSpec {
            property: TransitionProperty::Opacity,
            duration_ms: 300.0,
            delay_ms: 0.0,
            timing: TimingFunction::Ease,
        },
    ];
    let found = find_matching_spec(&specs, TransitionProperty::Opacity);
    assert!(found.is_some());
    assert_eq!(found.unwrap().duration_ms, 300.0);
}

#[test]
fn test_find_matching_spec_all_fallback() {
    let specs = vec![
        TransitionSpec {
            property: TransitionProperty::All,
            duration_ms: 500.0,
            delay_ms: 0.0,
            timing: TimingFunction::EaseInOut,
        },
    ];
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
    let mut style = ComputedStyle::default();
    style.opacity = 1.0;
    apply_value_to_style(&mut style, TransitionProperty::Opacity, &AnimatableValue::Float(0.5));
    assert!((style.opacity - 0.5).abs() < 0.01);
}

#[test]
fn test_apply_value_background_color() {
    let mut style = ComputedStyle::default();
    let blue = peniko::Color::from_rgba8(0, 0, 255, 255);
    apply_value_to_style(&mut style, TransitionProperty::BackgroundColor, &AnimatableValue::Color(blue));
    match &style.background {
        BackgroundValue::Color(c) => {
            let rgba = c.to_rgba8();
            assert_eq!(rgba.b, 255);
        }
        _ => panic!("Expected Color background"),
    }
}
