//! The scenes the calibration renders: a deliberately curve-heavy one, and
//! the sections of ui-zoo, rinch's component showcase.

use rinch::prelude::*;

/// A scene to render, and how many steps it has (ui-zoo: one per section).
pub struct SceneKind {
    pub name: &'static str,
    pub steps: usize,
}

pub const CURVES: SceneKind = SceneKind {
    name: "curves",
    steps: 1,
};

/// ui-zoo's sections, by index into `ui_zoo::section_content`, leaving out
/// those that open media or the network (video, audio and camera, WebRTC):
/// the calibration runs headless and touches no device but the GPU.
pub const UI_ZOO_SECTIONS: &[usize] =
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 19, 20];

pub const UI_ZOO: SceneKind = SceneKind {
    name: "ui-zoo",
    steps: UI_ZOO_SECTIONS.len(),
};

/// Large filled and stroked circles, wide round strokes along cubics, and
/// target-high diagonals, drawn in a 1280x720 view box scaled to
/// `width` x `height`: the scene whose lines and segments per path tag set
/// those rules (#2466).
pub fn curves(scope: &mut RenderScope, width: u32, height: u32) -> NodeHandle {
    let el = |scope: &mut RenderScope, tag: &str, attrs: &[(&str, String)]| {
        let node = scope.create_element(tag);
        for (k, v) in attrs {
            node.set_attribute(k, v);
        }
        node
    };
    let root = el(
        scope,
        "div",
        &[("style", "margin: 0; padding: 0; overflow: hidden".into())],
    );
    let svg = el(
        scope,
        "svg",
        &[
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("viewBox", "0 0 1280 720".into()),
            (
                "style",
                format!("width: {width}px; height: {height}px; display: block"),
            ),
        ],
    );
    for (i, r) in [340, 260, 200, 150, 110, 80].into_iter().enumerate() {
        let attrs = [
            ("cx", (200 + i * 170).to_string()),
            ("cy", "360".into()),
            ("r", r.to_string()),
            ("fill", format!("rgba({}, 90, 200, 0.25)", 40 * i)),
        ];
        svg.append_child(&el(scope, "circle", &attrs));
    }
    for (i, r) in [300, 220, 140].into_iter().enumerate() {
        let attrs = [
            ("cx", (640 + i * 40).to_string()),
            ("cy", "360".into()),
            ("r", r.to_string()),
            ("fill", "none".into()),
            ("stroke", "#c04020".into()),
            ("stroke-width", "30".into()),
        ];
        svg.append_child(&el(scope, "circle", &attrs));
    }
    for i in 0..20_i64 {
        let y = 20 + i * 34;
        let attrs = [
            (
                "d",
                format!("M 10 {y} C 300 {} 900 {} 1270 {y}", y - 300, y + 300),
            ),
            ("fill", "none".into()),
            ("stroke", "#2080c0".into()),
            ("stroke-width", "24".into()),
            ("stroke-linecap", "round".into()),
            ("stroke-linejoin", "round".into()),
        ];
        svg.append_child(&el(scope, "path", &attrs));
    }
    for i in 0..40 {
        let x = i * 32;
        let attrs = [
            ("x1", x.to_string()),
            ("y1", "0".into()),
            ("x2", (1280 - x).to_string()),
            ("y2", "720".into()),
            ("stroke", "#208040".into()),
            ("stroke-width", if i % 2 == 0 { "1" } else { "12" }.into()),
        ];
        svg.append_child(&el(scope, "line", &attrs));
    }
    root.append_child(&svg);
    root
}
