//! #2071: `render_to_texture` does not wait for the GPU, so a frame that
//! overflows Vello's buffers goes out undrawn, and is found out from its
//! counters afterwards. What it left must not stay: the frame after it, full
//! and at the grown sizes, repairs everything. These tests look for that the
//! way the desktop loop does, with `check_frames` and never a blocking poll.
//! The caller's half, `WgpuRenderer::check_frames` and the loop's timed wake,
//! needs a window. They need an adapter:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_bump --test-threads=1`.
use super::gpu_partial_frame_1718_tests::readback;
use super::*;
use rinch_dom::perf::FullRepaintReason;
use std::time::{Duration, Instant};
use vello::FrameCheck;

const W: u32 = 1024;
const H: u32 = 768;
/// Window-sized opacity layers, 64 by 48 tiles of 16 px, twice each (a layer's
/// clip and its fill): over Vello's 2^21 tiles, so these frames overflow.
const LAYERS: usize = 400;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    ui: wgpu::Texture,
}

impl Gpu {
    fn new() -> Self {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("a GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .unwrap();
        let ui = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            device,
            queue,
            renderer,
            ui,
        }
    }

    /// What `WgpuRenderer` renders a frame with, over a transparent window's
    /// base: a frame that goes undrawn there is a blank window.
    fn render(&mut self, app: &mut RinchApp, target: Option<FullRepaintReason>) -> &'static str {
        use crate::shell::desktop::{PartialFrame, render_partial};
        let base = vello::peniko::Color::TRANSPARENT;
        let mut partial_target = None;
        match app.build_gpu_frame(1.0, (W, H), target) {
            GpuFrame::Full(scene) => {
                self.renderer
                    .render_to_texture(
                        &self.device,
                        &self.queue,
                        scene,
                        &self.ui.create_view(&Default::default()),
                        &vello::RenderParams {
                            base_color: base,
                            width: W,
                            height: H,
                            antialiasing_method: vello::AaConfig::Msaa16,
                        },
                    )
                    .unwrap();
                "full"
            }
            GpuFrame::Partial { scene, size, rects } => {
                render_partial(
                    &self.device,
                    &self.queue,
                    &mut self.renderer,
                    &self.ui,
                    &mut partial_target,
                    PartialFrame {
                        scene,
                        size,
                        rects: &rects,
                        base,
                    },
                )
                .unwrap();
                "partial"
            }
            GpuFrame::Unchanged => "unchanged",
        }
    }

    /// The first answer that is not `Pending`, asked the way the desktop loop
    /// asks: without ever blocking on the device.
    fn settled_check(&mut self) -> FrameCheck {
        let start = Instant::now();
        loop {
            match self.renderer.check_frames(&self.device) {
                FrameCheck::Pending => {
                    assert!(
                        start.elapsed() < Duration::from_secs(120),
                        "the counters never came back"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                check => return check,
            }
        }
    }

    /// Full frames until one is not lost, as the loop asks for them, and how
    /// many it took. A frame can overflow a second buffer once the first is
    /// grown: stages after one that failed stop short of saying what they need.
    fn heal(&mut self, app: &mut RinchApp) -> u32 {
        for tries in 1..=4 {
            let kind = self.render(app, Some(FullRepaintReason::Invalidated));
            assert_eq!(kind, "full", "a lost frame is repainted in full");
            if self.settled_check() == FrameCheck::Settled {
                return tries;
            }
        }
        panic!("still overflowing after four grown frames");
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let px = readback(&self.device, &self.queue, &self.ui, W, H);
        let at = ((y * W + x) * 4) as usize;
        px[at..at + 4].try_into().unwrap()
    }
}

/// A blue page under `LAYERS` red ones, each at half opacity, which saturate
/// to red, and a green box over them all.
fn app() -> RinchApp {
    let mut app = super::perf_expect::new_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 1024px; height: 768px; \
             background-color: rgb(20, 60, 200)",
        );
        for _ in 0..LAYERS {
            let layer = scope.create_element("div");
            layer.set_attribute(
                "style",
                "position: absolute; left: 0; top: 0; width: 1024px; height: 768px; \
                 background-color: rgb(200, 30, 40); opacity: 0.5",
            );
            root.append_child(&layer);
        }
        let green = scope.create_element("div");
        green.set_attribute(
            "style",
            "position: absolute; left: 300px; top: 200px; width: 40px; height: 40px; \
             background-color: rgb(30, 200, 90)",
        );
        root.append_child(&green);
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

fn assert_drawn(gpu: &Gpu) {
    assert_eq!(gpu.pixel(310, 210), [30, 200, 90, 255], "the box on top");
    let [r, g, b, a] = gpu.pixel(700, 500);
    assert_eq!(a, 255, "the page");
    assert!(
        r.abs_diff(200) <= 2 && g.abs_diff(30) <= 2 && b.abs_diff(40) <= 2,
        "the layers: {r},{g},{b}"
    );
}

#[test]
#[ignore]
fn gpu_bump_overflow_a_lost_frame_is_found_and_the_next_full_one_repairs_it() {
    let mut gpu = Gpu::new();
    let mut app = app();
    assert_eq!(
        gpu.render(&mut app, Some(FullRepaintReason::FirstFrame)),
        "full"
    );
    assert_eq!(
        gpu.settled_check(),
        FrameCheck::Lost,
        "the fixture overflows"
    );
    assert_eq!(
        gpu.pixel(310, 210),
        [0, 0, 0, 0],
        "the frame went out undrawn, or this test proves nothing"
    );
    assert!(
        gpu.heal(&mut app) <= 3,
        "one bad frame per buffer at the most"
    );
    assert_drawn(&gpu);
    // At the grown sizes it now fits at once, and stays fitted.
    gpu.render(&mut app, Some(FullRepaintReason::Invalidated));
    assert_eq!(gpu.settled_check(), FrameCheck::Settled);
    assert_drawn(&gpu);
}

#[test]
#[ignore]
fn gpu_bump_overflow_frames_sent_before_the_counters_came_back_are_lost_too() {
    let mut gpu = Gpu::new();
    let mut app = app();
    // A window drawing on: nothing is known of the first frame when the next
    // two are submitted, and each overflows at the old sizes.
    gpu.render(&mut app, Some(FullRepaintReason::FirstFrame));
    gpu.render(&mut app, Some(FullRepaintReason::Invalidated));
    gpu.render(&mut app, Some(FullRepaintReason::Invalidated));
    assert_eq!(gpu.settled_check(), FrameCheck::Lost);
    // The last of them is the one showing, undrawn, so the frame after the
    // counters arrive has to repair it whichever of them reported.
    gpu.heal(&mut app);
    assert_drawn(&gpu);
}

#[test]
#[ignore]
fn gpu_bump_overflow_a_lost_partial_frame_leaves_nothing_behind() {
    let mut gpu = Gpu::new();
    let page = std::rc::Rc::new(std::cell::RefCell::new(None));
    let handle = page.clone();
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 1024px; height: 768px; \
             background-color: rgb(20, 60, 200)",
        );
        // A band the first frame does not draw, too heavy for Vello's buffers
        // yet small enough to come as a partial frame.
        let band = scope.create_element("div");
        band.set_attribute(
            "style",
            "position: absolute; left: 100px; top: 100px; width: 320px; height: 240px; \
             visibility: hidden",
        );
        for _ in 0..4000 {
            let layer = scope.create_element("div");
            layer.set_attribute(
                "style",
                "position: absolute; left: 0; top: 0; width: 320px; height: 240px; \
                 background-color: rgb(200, 30, 40); opacity: 0.5",
            );
            band.append_child(&layer);
        }
        root.append_child(&band);
        *handle.borrow_mut() = Some(band);
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);

    assert_eq!(
        gpu.render(&mut app, Some(FullRepaintReason::FirstFrame)),
        "full"
    );
    assert_eq!(gpu.settled_check(), FrameCheck::Settled, "the page fits");
    assert_eq!(gpu.pixel(700, 500), [20, 60, 200, 255]);

    page.borrow()
        .as_ref()
        .unwrap()
        .set_style("visibility", "visible");
    app.handle_event(PlatformEvent::AboutToWait, (W, H), 1.0);
    if app.has_pending_layout() {
        app.resolve_and_repaint(W as f32, H as f32);
    }
    assert_eq!(gpu.render(&mut app, None), "partial");
    assert_eq!(
        gpu.settled_check(),
        FrameCheck::Lost,
        "the fixture overflows"
    );
    // The copied rects hold the partial frame's undrawn target.
    assert_eq!(gpu.pixel(200, 200)[3], 0, "or this test proves nothing");

    gpu.heal(&mut app);
    let [r, g, b, a] = gpu.pixel(200, 200);
    assert_eq!(a, 255);
    assert!(r.abs_diff(200) <= 2 && g.abs_diff(30) <= 2 && b.abs_diff(40) <= 2);
    assert_eq!(gpu.pixel(700, 500), [20, 60, 200, 255]);
}
