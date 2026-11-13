//! A hidden window prepared for its next show (#2633): the show's frame does
//! no style, layout or glyph rasterization, and decodes no image, because
//! `prepare_hidden` did it while the window was hidden.
//!
//! Driven as the shell drives a hidden start: the root is mounted at the
//! sizing window's size and the window hidden (`create_window`), then the
//! show's frame is what `show_window` asks for — the whole scene repainted
//! (`mark_scene_dirty`) — painted at the size the window comes back at. The
//! software renderer, so the frames are exact on any host; the GPU half is
//! measured live (the PR's numbers) and counts `gpu_renderers_built` in the
//! prepare's frame. Text is set in the bundled Inter (`perf_expect`).

use super::*;
use crate as rinch;
use crate::app::perf_expect::expect_frame;
use crate::shell::renderer::Renderer;
use rinch_core::Signal;
use rinch_core::element::WindowProps;
use rinch_dom::perf::{Counter::*, FrameStats};
use rinch_macros::rsx;

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// A 2x2 PNG with an alpha channel (premultiplied when painted).
const ICON: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DQwMDw/wQDjAEAOAQGjfQ5xpIAAAAASUVORK5CYII=";

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 14px; line-height: 20px; }
    .field { width: 600px; height: 24px; }
    .row { height: 32px; display: flex; align-items: center; }
    .row.on { background: linear-gradient(rgb(40, 80, 200), rgb(20, 40, 120)); color: white; }
    .row img { width: 24px; height: 24px; margin-right: 8px; }
";

/// One row of the launcher: an icon and a name, highlighted while
/// `highlighted` names it.
fn row(__scope: &mut RenderScope, i: usize, name: &str, highlighted: Signal<usize>) -> NodeHandle {
    rsx! {
        div { class: {move || if highlighted.get() == i { "row on" } else { "row" }},
            img { src: ICON }
            {name}
        }
    }
}

/// The issue's launcher: a field and five rows of an icon and a name, the
/// one `highlighted` names highlighted.
fn launcher(highlighted: Signal<usize>) -> impl FnOnce(&mut RenderScope) -> NodeHandle {
    move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style { {CSS} }
                div { class: "field", "Search applications" }
                {row(__scope, 0, "Files", highlighted)}
                {row(__scope, 1, "Terminal", highlighted)}
                {row(__scope, 2, "Browser", highlighted)}
                {row(__scope, 3, "Mail", highlighted)}
                {row(__scope, 4, "Settings", highlighted)}
            }
        }
    }
}

/// The props' size, which the root is mounted at.
const MOUNTED: (u32, u32) = (800, 600);
/// The size the app gives the window while it is hidden
/// (`set_current_window_size`), which it is shown at: off the mounted size,
/// so a preparation at the wrong size leaves the show a layout to do.
const SHOWN: (u32, u32) = (640, 420);

/// A hidden start (`create_window` with `visible: false`); then, while it is
/// hidden, the window resized and its content changed (the highlight moved
/// to the second row); the counters zeroed.
fn hidden_launcher() -> RinchRuntime {
    let highlighted = Signal::new(0);
    let props = WindowProps {
        width: MOUNTED.0,
        height: MOUNTED.1,
        visible: false,
        ..Default::default()
    };
    let mut rt = build_runtime(
        launcher(highlighted),
        &props,
        None,
        &[crate::font::AppFont::sans_serif(INTER)],
    );
    rt.main.app.set_device_pixel_ratio(1.0);
    rt.main
        .app
        .mount_component(MOUNTED.0 as f32, MOUNTED.1 as f32);
    rt.main.hide_window(None);
    rt.main.saved_geometry.size = Some(Size::Logical(LogicalSize::new(
        SHOWN.0 as f64,
        SHOWN.1 as f64,
    )));
    highlighted.set(1);
    rt.main.app.reset_perf();
    rt
}

/// The frame `show_window` asks for at scale 1.
fn show_frame(rt: &mut RinchRuntime) -> FrameStats {
    show_frame_at(rt, 1.0)
}

/// The frame `show_window` asks for, on an output at `scale`, painted at
/// the size the window comes back at: the new window's scale (winit's
/// `ScaleFactorChanged`, a no-op at the scale the document has) and its
/// `SurfaceResized`, then what `show_window` itself asks of the document
/// (`repaint_whole_after_show`).
fn show_frame_at(rt: &mut RinchRuntime, scale: f64) -> FrameStats {
    let physical = (
        (SHOWN.0 as f64 * scale).round() as u32,
        (SHOWN.1 as f64 * scale).round() as u32,
    );
    let _ = rt
        .main
        .app
        .handle_event(PlatformEvent::ScaleFactorChanged(scale), physical, scale);
    let _ = rt.main.app.handle_event(
        PlatformEvent::Resized {
            width: SHOWN.0,
            height: SHOWN.1,
        },
        physical,
        scale,
    );
    rt.main.repaint_whole_after_show();
    rt.main
        .paint_software_at(scale, SHOWN)
        .expect("a software paint");
    rt.main.app.end_perf_frame().expect("mounted")
}

/// The outputs the shell lists: one, at `scale`, named `eDP-1`, and a
/// second at scale 1 after it.
fn publish_outputs(scale: f64) {
    let output = |name: &str, scale| crate::outputs::Output {
        name: Some(name.into()),
        position: (0.0, 0.0),
        size: (1280.0, 720.0),
        scale,
    };
    crate::outputs::publish(
        vec![output("eDP-1", scale), output("HDMI-A-1", 1.0)],
        crate::outputs::OutputTracking::Live,
    );
}

/// The document's presented and prepared frame counts.
fn frame_counts(rt: &RinchRuntime) -> (u64, u64) {
    let doc = rt.main.app.doc.as_ref().expect("mounted").borrow();
    (doc.tree.perf.frames(), doc.tree.perf.prepared())
}

#[test]
fn a_prepared_hidden_window_shows_with_no_style_layout_or_raster_work() {
    let mut rt = hidden_launcher();
    rt.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    let prepared = rt.main.app.last_frame_perf();
    // Positive control: the preparation did the work the show is spared.
    for c in [StyleResolves, LayoutResolves, GlyphAtlasMisses] {
        assert!(
            prepared.get(c) > 0,
            "the preparation did no {}: {prepared:?}",
            c.name()
        );
    }
    let shown = show_frame(&mut rt);
    // The scene repainted in full, every glyph from the atlas. The one
    // layout resolve is the show's `Resized` at the size it was prepared at,
    // found to change nothing (`layout_skipped_paint_only`); a show after a
    // hide does the same.
    expect_frame(
        "the first show of a prepared hidden window",
        &shown,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullUnattributed, 1),
            (RepaintedPx, 268800),
            (SurfacePx, 268800),
            (PaintNodesVisited, 18),
            (StackingOrderBuilds, 1),
            (GlyphAtlasHits, 51),
        ],
    );
}

/// What the preparation spares: without it, the first show restyles what
/// changed while hidden, lays the document out at its new size and
/// rasterizes every glyph.
#[test]
fn an_unprepared_hidden_window_does_it_all_at_its_first_show() {
    let mut rt = hidden_launcher();
    let shown = show_frame(&mut rt);
    expect_frame(
        "the first show of a hidden window not prepared",
        &shown,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 2),
            (ElementsRecascaded, 2),
            (StyleNodesVisited, 4),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 4),
            (ShapeMeasureText, 14),
            (IfcMeasureCacheHits, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 20),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullFirstFrame, 1),
            (RepaintedPx, 268800),
            (SurfacePx, 268800),
            (PaintNodesVisited, 18),
            (StackingOrderBuilds, 1),
            (GlyphAtlasHits, 12),
            (GlyphAtlasMisses, 39),
            (SignalNotifies, 1),
        ],
    );
}

/// A second preparation with nothing changed paints nothing.
#[test]
fn preparing_again_with_nothing_changed_is_free() {
    let mut rt = hidden_launcher();
    rt.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    rt.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    expect_frame(
        "a second preparation with nothing changed",
        &rt.main.app.last_frame_perf(),
        &[(PaintCachedFrames, 1), (PreparedFrames, 1)],
    );
}

/// A prepared frame is painted and not presented, and the performance
/// counters say so (#2633): it counts `prepared_frames`, its `RINCH_PERF`
/// line is `[PERF] prepared N`, and it is not one of `frames` — so "a hidden
/// window draws nothing" holds while it is hidden. The show's frame is.
#[test]
fn a_prepared_frame_is_marked_and_is_not_a_presented_frame() {
    let mut rt = hidden_launcher();
    assert_eq!(frame_counts(&rt), (0, 0));
    rt.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    let prepared = rt.main.app.last_frame_perf();
    assert_eq!(prepared.get(PreparedFrames), 1);
    assert_eq!(prepared.get(PaintFrames), 1, "it painted");
    assert_eq!(frame_counts(&rt), (0, 1), "(frames, prepared)");
    let shown = show_frame(&mut rt);
    assert_eq!(shown.get(PreparedFrames), 0);
    assert_eq!(frame_counts(&rt), (1, 1), "(frames, prepared)");
}

/// Before its first show a hidden window has no scale of its own (a
/// Wayland surface has none until it is mapped), so a preparation paints at
/// the scale of the output it is likely shown on: the first listed here, at
/// 1.5. Shown there, nothing is rasterized again. The scale the hidden start
/// was created with (1) would have been wrong.
#[test]
fn a_preparation_paints_at_the_likely_outputs_scale() {
    publish_outputs(1.5);
    let mut rt = hidden_launcher();
    rt.main.prepare_hidden_as(Renderer::Software, None);
    assert_eq!(rt.main.app.device_pixel_ratio, 1.5);
    let shown = show_frame_at(&mut rt, 1.5);
    for c in [StyleResolves, TaffyRootComputes, GlyphAtlasMisses] {
        assert_eq!(shown.get(c), 0, "{} at the show: {shown:?}", c.name());
    }
    assert!(shown.get(GlyphAtlasHits) > 0, "{shown:?}");
}

/// The control: prepared at scale 1 and shown at 2, the show restyles (the
/// device pixel ratio) and rasterizes every glyph again.
#[test]
fn a_preparation_at_the_wrong_scale_leaves_the_show_its_glyphs() {
    let mut rt = hidden_launcher();
    rt.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    let shown = show_frame_at(&mut rt, 2.0);
    assert!(shown.get(StyleResolves) > 0, "{shown:?}");
    assert!(shown.get(GlyphAtlasMisses) > 0, "{shown:?}");
}

/// The app's scale wins over the guess.
#[test]
fn the_apps_scale_wins() {
    publish_outputs(1.5);
    let mut rt = hidden_launcher();
    rt.main.prepare_hidden_as(Renderer::Software, Some(2.0));
    assert_eq!(rt.main.app.device_pixel_ratio, 2.0);
    let shown = show_frame_at(&mut rt, 2.0);
    assert_eq!(shown.get(GlyphAtlasMisses), 0, "{shown:?}");
}

#[test]
fn the_likely_output_is_the_named_one_else_the_first() {
    let output = |name: &str, scale| crate::outputs::Output {
        name: Some(name.into()),
        position: (0.0, 0.0),
        size: (1.0, 1.0),
        scale,
    };
    let outputs = [output("DP-1", 1.0), output("eDP-1", 1.75)];
    assert_eq!(likely_output_scale(&outputs, Some("eDP-1")), Some(1.75));
    assert_eq!(likely_output_scale(&outputs, Some("gone")), Some(1.0));
    assert_eq!(likely_output_scale(&outputs, None), Some(1.0));
    assert_eq!(likely_output_scale(&[], None), None);
}
