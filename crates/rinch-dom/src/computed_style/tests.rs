//! Unit tests for ComputedStyle.

use super::*;
use std::collections::HashMap;
use crate::layout::Viewport;

#[test]
fn test_display_value_parse() {
    assert_eq!(DisplayValue::parse("flex"), DisplayValue::Flex);
    assert_eq!(DisplayValue::parse("block"), DisplayValue::Block);
    assert_eq!(DisplayValue::parse("none"), DisplayValue::None);
    assert_eq!(DisplayValue::parse("inline-flex"), DisplayValue::InlineFlex);
}

#[test]
fn test_dimension_value_parse() {
    let vp = Viewport {
        width: 1000.0,
        height: 800.0,
    };
    assert!(matches!(
        DimensionValue::parse("auto", &vp),
        DimensionValue::Auto
    ));
    assert!(
        matches!(DimensionValue::parse("100px", &vp), DimensionValue::Length(v) if (v - 100.0).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("50%", &vp), DimensionValue::Percent(v) if (v - 0.5).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("10vh", &vp), DimensionValue::Length(v) if (v - 80.0).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("10vw", &vp), DimensionValue::Length(v) if (v - 100.0).abs() < 0.01)
    );
    assert!(
        matches!(DimensionValue::parse("2rem", &vp), DimensionValue::Length(v) if (v - 32.0).abs() < 0.01)
    );
}

#[test]
fn test_computed_style_from_props() {
    let vp = Viewport {
        width: 1000.0,
        height: 800.0,
    };
    let mut props = HashMap::new();
    props.insert("display".to_string(), "flex".to_string());
    props.insert("width".to_string(), "100px".to_string());
    props.insert("flex-grow".to_string(), "1".to_string());
    props.insert("padding-top".to_string(), "10px".to_string());

    let style = ComputedStyle::from_props(&props, &vp);
    assert_eq!(style.display, DisplayValue::Flex);
    assert!(matches!(style.width, DimensionValue::Length(v) if (v - 100.0).abs() < 0.01));
    assert!((style.flex_grow - 1.0).abs() < 0.01);
    assert!(
        matches!(style.padding_top, LengthPercentageValue::Length(v) if (v - 10.0).abs() < 0.01)
    );
}

#[test]
fn test_flex_shorthand() {
    let vp = Viewport::default();
    let mut props = HashMap::new();
    props.insert("flex".to_string(), "1".to_string());

    let style = ComputedStyle::from_props(&props, &vp);
    assert!((style.flex_grow - 1.0).abs() < 0.01);
    assert!((style.flex_shrink - 1.0).abs() < 0.01);
    assert!(matches!(style.flex_basis, DimensionValue::Length(v) if v.abs() < 0.01));
}

#[test]
fn test_to_taffy_style() {
    let vp = Viewport::default();
    let mut props = HashMap::new();
    props.insert("display".to_string(), "flex".to_string());
    props.insert("flex-direction".to_string(), "column".to_string());
    props.insert("width".to_string(), "200px".to_string());

    let style = ComputedStyle::from_props(&props, &vp);
    let taffy_style = style.to_taffy_style(crate::layout::DefaultDisplay::Block);

    assert_eq!(taffy_style.display, taffy::Display::Flex);
    assert_eq!(taffy_style.flex_direction, taffy::FlexDirection::Column);
}
