//! #2719 review: the rasters paint keeps for a `filter` go stale in a real
//! app's frame order. Every shell path drains the change lists
//! (`consume_paint_dirty`) **before** it paints — `build_scene_with` for the
//! embed overlay and the GPU window, `frame_damage` for a desktop software
//! frame — so the kept rasters are invalidated there, and these tests go
//! through `RinchApp` to check it.

use super::*;

const SIZE: (u32, u32) = (300, 200);

/// A change to the mounted app, given the panel's child and the readout's
/// text.
type Step<'a> = &'a dyn Fn(&NodeHandle, &NodeHandle);

/// `(app, the panel's child, the readout's text node)`: a blurred panel with
/// a child box, and a readout under a drop shadow.
fn mounted() -> (RinchApp, NodeHandle, NodeHandle) {
    let (app, child, text, _) = mounted_with_other();
    (app, child, text)
}

/// [`mounted`], and a box outside both filters.
fn mounted_with_other() -> (RinchApp, NodeHandle, NodeHandle, NodeHandle) {
    let slot: Rc<RefCell<Option<(NodeHandle, NodeHandle, NodeHandle)>>> =
        Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: relative; width: 300px; height: 200px; background: #fff",
        );
        let panel = scope.create_element("div");
        panel.set_attribute(
            "style",
            "position: absolute; left: 61px; top: 47px; width: 113px; height: 71px; \
             background: #fe8; filter: blur(2.5px)",
        );
        let child = scope.create_element("div");
        child.set_attribute(
            "style",
            "position: absolute; left: 13px; top: 17px; width: 21px; height: 19px; \
             background: #a0c",
        );
        panel.append_child(&child);
        root.append_child(&panel);
        let readout = scope.create_element("div");
        readout.set_attribute(
            "style",
            "position: absolute; left: 190px; top: 120px; font: bold 24px/30px sans-serif; \
             filter: drop-shadow(2px 3px 2px #000)",
        );
        let text = scope.create_text("218");
        readout.append_child(&text);
        root.append_child(&readout);
        let other = scope.create_element("div");
        other.set_attribute(
            "style",
            "position: absolute; left: 10px; top: 160px; width: 20px; height: 20px; \
             background: #999",
        );
        root.append_child(&other);
        *slot_in.borrow_mut() = Some((child, text, other));
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let (child, text, other) = slot.borrow().clone().unwrap();
    (app, child, text, other)
}

fn counters(app: &mut RinchApp) -> (u64, u64) {
    use rinch_dom::perf::Counter::{FilterCacheHits, FilterRasters};
    let s = app.end_perf_frame().expect("mounted");
    (s.get(FilterRasters), s.get(FilterCacheHits))
}

/// The embed overlay's frame (`build_scene`): a frame that changes neither
/// filter draws both from their kept rasters, and a change inside one rasterises that
/// one again — the readout ticking does not freeze at its first value.
#[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
#[test]
fn build_scene_rasterises_again_what_changed_inside_a_filter() {
    let (mut app, child, text, other) = mounted_with_other();
    let _ = app.build_scene(1.0, SIZE);
    let _ = counters(&mut app);
    // A change outside both filters: the scene is built again, and both are
    // drawn from their kept rasters.
    other.set_style("background", "#0a0");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let _ = app.build_scene(1.0, SIZE);
    assert_eq!(
        counters(&mut app),
        (0, 2),
        "a frame with both filters untouched rasterised"
    );
    text.set_text("219");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let _ = app.build_scene(1.0, SIZE);
    assert_eq!(
        counters(&mut app),
        (1, 1),
        "the ticking readout was not rasterised again"
    );
    child.set_style("background", "#0cf");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let _ = app.build_scene(1.0, SIZE);
    assert_eq!(
        counters(&mut app),
        (1, 1),
        "the recoloured panel was not rasterised again"
    );
}

/// A desktop software frame (`build_pixels`, partial repaints through the
/// damage): after each change the pixels are a fresh app's of the same state.
#[cfg(all(software_shell, feature = "desktop"))]
#[test]
fn desktop_frames_show_what_changed_inside_a_filter() {
    let fresh = |f: Step| {
        let (mut app, child, text) = mounted();
        f(&child, &text);
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        app.build_pixels(1.0, SIZE, false).0.to_vec()
    };
    let (mut app, child, text) = mounted();
    let _ = app.build_pixels(1.0, SIZE, false);
    let _ = app.build_pixels(1.0, SIZE, false);
    let steps: [Step; 2] = [&|_c, t| t.set_text("219"), &|c, t| {
        t.set_text("219");
        c.set_style("background", "#0cf");
    }];
    for (i, step) in steps.iter().enumerate() {
        step(&child, &text);
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        let got = app.build_pixels(1.0, SIZE, false).0.to_vec();
        let want = fresh(*step);
        let bad = got
            .chunks(4)
            .zip(want.chunks(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
            .count();
        assert_eq!(bad, 0, "step {i}: {bad} stale pixels");
    }
}
