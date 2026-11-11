//! #2466: Vello's bump buffers start at sizes that follow the target and the
//! scene (`BumpSizes::initial`, joeleaver/vello-fork#8), not at fixed sizes
//! that cost every renderer about 165 MiB. These pin both sides of that: a
//! small window holds a few MiB of them, and the first frame of a new renderer
//! at 1080p and 4K still draws first time for the scenes the sizes were
//! calibrated on. They need an adapter:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_bump --test-threads=1`.
use super::gpu_bump_overflow_2071_tests::{Gpu, layers};
use super::gpu_partial_frame_1718_tests::readback;
use super::*;
use rinch_dom::perf::FullRepaintReason;
use vello::FrameCheck;

/// A bar: a row of boxes and labels, the size of a panel.
fn bar(width: u32, height: u32) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            &format!(
                "display: flex; gap: 8px; width: {width}px; height: {height}px; \
                 background-color: rgb(30, 30, 40); color: white; font-family: sans-serif"
            ),
        );
        for i in 0..8 {
            let item = scope.create_element("div");
            item.set_attribute(
                "style",
                "padding: 4px 10px; border-radius: 6px; background-color: rgb(60, 70, 90)",
            );
            item.append_child(&scope.create_text(&format!("Item {i}")));
            root.append_child(&item);
        }
        root
    });
    app.mount_component(width as f32, height as f32);
    app.resolve_and_repaint(width as f32, height as f32);
    app
}

/// A page of text and boxes filling the target, with rounded and bordered
/// cards: the kind of window the calibration's examples are.
fn page(width: u32, height: u32) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            &format!(
                "display: flex; flex-wrap: wrap; gap: 12px; padding: 12px; \
                 width: {width}px; height: {height}px; font-family: sans-serif"
            ),
        );
        for i in 0..120 {
            let card = scope.create_element("div");
            card.set_attribute(
                "style",
                "width: 220px; padding: 8px; border: 1px solid rgb(120, 120, 140); \
                 border-radius: 10px; background-color: rgb(240, 240, 250)",
            );
            card.append_child(&scope.create_text(&format!(
                "Card {i}: some text that wraps over more than one line"
            )));
            root.append_child(&card);
        }
        root
    });
    app.mount_component(width as f32, height as f32);
    app.resolve_and_repaint(width as f32, height as f32);
    app
}

/// The calibration's curve-heavy scene, scaled to the target: large filled
/// and stroked circles, wide round strokes along cubics, and window-high
/// diagonals.
fn curves(width: u32, height: u32) -> vello::Scene {
    use vello::kurbo::{Affine, BezPath, Cap, Circle, Join, Line, Stroke};
    use vello::peniko::{Color, Fill};
    let scale = Affine::scale_non_uniform(width as f64 / 1280.0, height as f64 / 720.0);
    let mut scene = vello::Scene::new();
    for (i, r) in [340.0, 260.0, 200.0, 150.0, 110.0, 80.0]
        .into_iter()
        .enumerate()
    {
        let c = Circle::new((200.0 + i as f64 * 170.0, 360.0), r);
        scene.fill(
            Fill::NonZero,
            scale,
            Color::from_rgba8(40, 90, 200, 64),
            None,
            &c,
        );
    }
    let wide = Stroke::new(30.0);
    for (i, r) in [300.0, 220.0, 140.0].into_iter().enumerate() {
        let c = Circle::new((640.0 + i as f64 * 40.0, 360.0), r);
        scene.stroke(&wide, scale, Color::from_rgb8(192, 64, 32), None, &c);
    }
    let round = Stroke::new(24.0)
        .with_caps(Cap::Round)
        .with_join(Join::Round);
    for i in 0..20 {
        let y = 20.0 + i as f64 * 34.0;
        let mut path = BezPath::new();
        path.move_to((10.0, y));
        path.curve_to((300.0, y - 300.0), (900.0, y + 300.0), (1270.0, y));
        scene.stroke(&round, scale, Color::from_rgb8(32, 128, 192), None, &path);
    }
    for i in 0..40 {
        let x = i as f64 * 32.0;
        let width = if i % 2 == 0 { 1.0 } else { 12.0 };
        let line = Line::new((x, 0.0), (1280.0 - x, 720.0));
        scene.stroke(
            &Stroke::new(width),
            scale,
            Color::from_rgb8(32, 128, 64),
            None,
            &line,
        );
    }
    scene
}

/// The full first frame of `app`, rendered by a new renderer into a target of
/// its size, and what became of it.
fn first_frame(gpu: &mut Gpu, app: &mut RinchApp, width: u32, height: u32) -> FrameCheck {
    let target = gpu.target(width, height);
    let GpuFrame::Full(scene) =
        app.build_gpu_frame(1.0, (width, height), Some(FullRepaintReason::FirstFrame))
    else {
        panic!("a first frame is full");
    };
    gpu.render_scene(scene, &target);
    gpu.settled_check()
}

/// The bytes of Vello's own buffers wgpu's allocator holds for `gpu`.
fn vello_bytes(gpu: &Gpu) -> u64 {
    let report = gpu
        .device
        .generate_allocator_report()
        .expect("an allocator report");
    report
        .allocations
        .iter()
        .filter(|a| a.name.starts_with("vello."))
        .map(|a| a.size)
        .sum()
}

#[test]
#[ignore]
fn gpu_bump_sizes_a_small_window_holds_a_few_mib_of_vello_buffers() {
    let mut gpu = Gpu::new();
    let mut app = bar(1280, 40);
    assert_eq!(
        first_frame(&mut gpu, &mut app, 1280, 40),
        FrameCheck::Settled
    );
    let bytes = vello_bytes(&gpu);
    // About 1.8 MiB of bump buffers at the initial sizes, the scene's own
    // buffers on top; Vello's fixed sizes were 165 MiB.
    assert!(bytes < 4 << 20, "Vello's buffers hold {bytes} bytes");
}

#[test]
#[ignore]
fn gpu_bump_sizes_first_frames_draw_first_time_at_1080p_and_4k() {
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        // The page that set the ptcl floor (#2071), at both sizes.
        let mut gpu = Gpu::new();
        let target = gpu.target(width, height);
        gpu.render_scene(&layers(8, width, height, false), &target);
        assert_eq!(
            gpu.settled_check(),
            FrameCheck::Settled,
            "8 layers at {width}x{height}"
        );
        let px = readback(&gpu.device, &gpu.queue, &target, width, height);
        let at = (((height / 2) * width + width / 2) * 4) as usize;
        assert!(
            px[at + 3] > 200,
            "the layers are there: {:?}",
            &px[at..at + 4]
        );

        // The calibration's curve-heavy scene.
        let mut gpu = Gpu::new();
        let target = gpu.target(width, height);
        gpu.render_scene(&curves(width, height), &target);
        assert_eq!(
            gpu.settled_check(),
            FrameCheck::Settled,
            "curves at {width}x{height}"
        );

        // A page of cards and text.
        let mut gpu = Gpu::new();
        let mut app = page(width, height);
        assert_eq!(
            first_frame(&mut gpu, &mut app, width, height),
            FrameCheck::Settled,
            "a page at {width}x{height}"
        );
    }
}
