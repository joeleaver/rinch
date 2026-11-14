//! The shell's decisions for a window sized by its content.

use super::*;
use rinch_core::element::{LayerAnchor, LayerAxes, LayerMargins};

fn output(id: u64, name: &str, size: (f64, f64)) -> Output {
    Output {
        id: rinch_core::element::OutputId::from_raw(id),
        name: Some(name.to_string()),
        position: (0.0, 0.0),
        size,
        scale: 1.0,
    }
}

fn surface(anchor: LayerAnchor, compositor_size: LayerAxes) -> LayerSurface {
    LayerSurface {
        anchor,
        compositor_size,
        ..Default::default()
    }
}

const CONTENT: ContentAxes = ContentAxes {
    width: AxisSizing::Content,
    height: AxisSizing::Content,
};

#[test]
fn a_window_that_is_no_layer_surface_sizes_both_axes() {
    assert_eq!(content_axes(None), Some(CONTENT));
}

#[test]
fn a_layer_surface_sizes_every_axis_the_compositor_does_not_assign() {
    let all = LayerAnchor::TOP | LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT;
    // No compositor size: both, whatever the anchors (anchored to both edges
    // of an axis, it is centred there at its own size).
    for anchor in [LayerAnchor::NONE, LayerAnchor::TOP, all] {
        assert_eq!(
            content_axes(Some(&surface(anchor, LayerAxes::NONE))),
            Some(CONTENT)
        );
    }
    // A bar: the width is the compositor's, the height the content's.
    let bar = surface(
        LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
        LayerAxes::WIDTH,
    );
    assert_eq!(
        content_axes(Some(&bar)),
        Some(ContentAxes {
            width: AxisSizing::Fill,
            height: AxisSizing::Content,
        })
    );
    // A column: the height is the compositor's.
    let column = surface(LayerAnchor::TOP | LayerAnchor::BOTTOM, LayerAxes::HEIGHT);
    assert_eq!(
        content_axes(Some(&column)),
        Some(ContentAxes {
            width: AxisSizing::Content,
            height: AxisSizing::Fill,
        })
    );
}

#[test]
fn a_compositor_size_the_protocol_cannot_assign_leaves_the_axis_to_the_content() {
    // Asked on an axis not anchored to both edges: not sent, so the content
    // sizes it.
    let half = surface(LayerAnchor::TOP | LayerAnchor::LEFT, LayerAxes::BOTH);
    assert_eq!(content_axes(Some(&half)), Some(CONTENT));
}

#[test]
fn both_axes_assigned_leave_nothing_to_size() {
    let all = LayerAnchor::TOP | LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT;
    assert_eq!(content_axes(Some(&surface(all, LayerAxes::BOTH))), None);
}

#[test]
fn the_maximum_is_the_apps_else_the_output_less_the_anchored_margins() {
    let out = output(1, "DP-1", (1920.0, 1080.0));
    let mut s = surface(LayerAnchor::TOP | LayerAnchor::RIGHT, LayerAxes::NONE);
    s.margins = LayerMargins {
        top: 10,
        right: 20,
        bottom: 30,
        left: 40,
    };
    let none = ContentSizing::new();
    // Only the margins on anchored edges count: right (20) and top (10).
    assert_eq!(
        content_max(&none, Some(&s), Some(&out), (800, 600)),
        (1900.0, 1070.0)
    );
    let given = ContentSizing::new().with_max_width(360);
    assert_eq!(
        content_max(&given, Some(&s), Some(&out), (800, 600)),
        (360.0, 1070.0)
    );
    // Not a layer surface: the whole output.
    assert_eq!(
        content_max(&none, None, Some(&out), (800, 600)),
        (1920.0, 1080.0)
    );
    // No output listed: the props' size.
    assert_eq!(
        content_max(&none, Some(&s), None, (800, 600)),
        (800.0, 600.0)
    );
}

#[test]
fn the_output_is_the_named_one_else_the_first() {
    let outputs = [
        output(1, "DP-1", (1920.0, 1080.0)),
        output(2, "DP-2", (1280.0, 720.0)),
    ];
    let props = WindowProps::default();
    let mut s = surface(LayerAnchor::NONE, LayerAxes::NONE);
    assert_eq!(
        sizing_output(Some(&s), &props, &outputs).and_then(|o| o.name.clone()),
        Some("DP-1".into())
    );
    s.output = Some("DP-2".into());
    assert_eq!(
        sizing_output(Some(&s), &props, &outputs).and_then(|o| o.name.clone()),
        Some("DP-2".into())
    );
    s.output_id = Some(rinch_core::element::OutputId::from_raw(1));
    assert_eq!(
        sizing_output(Some(&s), &props, &outputs).and_then(|o| o.name.clone()),
        Some("DP-1".into()),
        "the id wins over the name"
    );
    s.output_id = None;
    s.output = Some("HDMI-A-9".into());
    assert_eq!(
        sizing_output(Some(&s), &props, &outputs).and_then(|o| o.name.clone()),
        Some("DP-1".into()),
        "an unknown name: the first"
    );
    let toplevel = WindowProps {
        monitor: Some("DP-2".into()),
        ..Default::default()
    };
    assert_eq!(
        sizing_output(None, &toplevel, &outputs).and_then(|o| o.name.clone()),
        Some("DP-2".into())
    );
    assert!(sizing_output(None, &props, &[]).is_none());
}

#[test]
fn width_and_height_beside_size_to_content_are_reported_unused() {
    let sized = WindowProps {
        size_to_content: Some(ContentSizing::new()),
        ..Default::default()
    };
    assert!(!sets_ignored_size(&sized), "defaults left alone");
    let both = WindowProps {
        width: 300,
        ..sized.clone()
    };
    assert!(sets_ignored_size(&both));
    let plain = WindowProps {
        width: 300,
        ..Default::default()
    };
    assert!(!sets_ignored_size(&plain), "not content-sized");
}
