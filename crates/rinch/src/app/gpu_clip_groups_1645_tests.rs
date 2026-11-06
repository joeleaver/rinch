//! #1645: both renderers are handed a stacking sequence gathered by clip.
//! `rinch-dom`'s `clip_groups_1645_tests` pin the picture on the software
//! painter. Away from clip edges Vello agrees with it. At clip edges it does not
//! have to: gathered entries share one clip layer, which on Vello is an isolated
//! layer on every tile the edge crosses (the accepted cost in `stacking.rs`,
//! **Clip groups**). The `gpu_clip_edge_*` tests pin what that gives. They need
//! an adapter:
//! `cargo test -p rinch --features desktop,gpu --lib -- --ignored gpu --test-threads=1`.
use super::gpu_partial_frame_1718_tests::readback;
use super::*;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

pub(super) const W: u32 = 320;
pub(super) const H: u32 = 200;

/// Two panes with translucent rows alternating between them. `second` is pane
/// B's left edge: clear of A at 171, over it at 139.
fn app(second: u32) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 320px; height: 200px; \
             background-color: white",
        );
        for (left, even) in [(13, 0), (second, 1)] {
            let pane = scope.create_element("div");
            pane.set_attribute(
                "style",
                &format!(
                    "position: absolute; left: {left}px; top: 9px; width: 141px; height: 170px; \
                     overflow: hidden"
                ),
            );
            for row in 0..6 {
                let z = 2 * row + even;
                let color = if even == 0 {
                    "rgba(200, 30, 40, 0.5)"
                } else {
                    "rgba(30, 50, 210, 0.5)"
                };
                let box_ = scope.create_element("div");
                box_.set_attribute(
                    "style",
                    &format!(
                        "position: absolute; left: 0; top: {}px; width: 141px; height: 47px; \
                         z-index: {z}; background-color: {color}",
                        7 + 11 * row
                    ),
                );
                pane.append_child(&box_);
            }
            root.append_child(&pane);
        }
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

pub(super) fn software(app: &mut RinchApp) -> Vec<u8> {
    let mut doc = app.doc.as_ref().unwrap().borrow_mut();
    let doc = &mut *doc;
    let mut painter = TinySkiaPainter::new(W, H);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

/// One device, one renderer and one target, for a run of scenes.
pub(super) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    target: wgpu::Texture,
}

impl Gpu {
    pub(super) fn new() -> Self {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
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
        let target = device.create_texture(&wgpu::TextureDescriptor {
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
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            device,
            queue,
            renderer,
            target,
        }
    }

    pub(super) fn render(&mut self, app: &mut RinchApp) -> Vec<u8> {
        let GpuFrame::Full(scene) = app.build_gpu_frame(
            1.0,
            (W, H),
            Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
        ) else {
            panic!("the first frame is full")
        };
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &self.target.create_view(&Default::default()),
                &vello::RenderParams {
                    base_color: vello::peniko::Color::WHITE,
                    width: W,
                    height: H,
                    antialiasing_method: crate::gpu_antialiasing::scene_aa(),
                },
            )
            .unwrap();
        readback(&self.device, &self.queue, &self.target, W, H)
    }
}

fn div(scope: &mut RenderScope, parent: &NodeHandle, style: String) -> NodeHandle {
    let d = scope.create_element("div");
    d.set_attribute("style", &style);
    parent.append_child(&d);
    d
}

/// A rounded pane A holding a full-pane `lower` row at `z` 1 and a full-pane
/// `upper` one at `z` 4, with pane B, clear of A, holding a row at `z` 2 between
/// them. Painted `grouped`, the two rows of A share one clip. Otherwise a 1px
/// box at `z` 3, which has no chain to prove it clear of anything, keeps them
/// apart, and they take a clip each, as all entries did before #1645.
fn rounded_pane(lower: &'static str, upper: &'static str, grouped: bool) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 320px; height: 200px; \
             background-color: white",
        );
        let row = "position: absolute; left: 0; top: 0; width: 100%; height: 100%";
        let a = div(
            scope,
            &root,
            "position: absolute; left: 13px; top: 9px; width: 181px; height: 170px; \
             overflow: hidden; border-radius: 30px"
                .to_string(),
        );
        let b = div(
            scope,
            &root,
            "position: absolute; left: 211px; top: 9px; width: 90px; height: 170px; \
             overflow: hidden"
                .to_string(),
        );
        div(scope, &a, format!("{row}; z-index: 1; {lower}"));
        div(
            scope,
            &b,
            format!("{row}; z-index: 2; background-color: rgb(90, 90, 90)"),
        );
        if !grouped {
            div(
                scope,
                &root,
                "position: absolute; left: 305px; top: 3px; width: 1px; height: 1px; \
                 z-index: 3; background-color: white"
                    .to_string(),
            );
        }
        div(scope, &a, format!("{row}; z-index: 4; {upper}"));
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

/// Away from a rounded edge or a blend, Vello and tiny-skia agree on the gathered
/// sequence: these panes are square, on whole pixels, so no edge has coverage.
#[test]
#[ignore]
fn gpu_clip_groups_match_the_software_painter() {
    let mut gpu = Gpu::new();
    for second in [171, 139] {
        let mut app = app(second);
        let cpu = software(&mut app);
        let gpu = gpu.render(&mut app);
        assert_eq!(cpu.len(), gpu.len());
        let differ = cpu
            .chunks(4)
            .zip(gpu.chunks(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 3))
            .count();
        assert_eq!(differ, 0, "pane B at {second}");
        // Control: a blank picture would agree too.
        let at = |x: u32, y: u32| &cpu[((y * W + x) * 4) as usize..][..3];
        assert_ne!(at(90, 70), at(250, 70), "control: the panes differ");
    }
}

/// Two opaque rows under a rounded clip, green under blue, on white. In one clip
/// layer the edge applies once: every edge pixel lies between white and blue,
/// with red and green equal. In a layer each, blue's partial coverage shows the
/// green row's fringe through it.
#[test]
#[ignore]
fn gpu_clip_edge_coverage_applies_once_to_gathered_rows() {
    let green = "background-color: rgb(0, 255, 0)";
    let blue = "background-color: rgb(0, 0, 255)";
    let fringe = |pixels: &[u8]| {
        let in_a = |i: usize| ((i % W as usize) as u32) < 200;
        pixels
            .chunks(4)
            .enumerate()
            .filter(|(i, _)| in_a(*i))
            .map(|(_, p)| p[1].abs_diff(p[0]))
            .collect::<Vec<u8>>()
    };
    let mut gpu = Gpu::new();
    let grouped = gpu.render(&mut rounded_pane(green, blue, true));
    let apart = gpu.render(&mut rounded_pane(green, blue, false));

    let worst = fringe(&grouped).into_iter().max().unwrap();
    assert!(worst <= 2, "one coverage leaves no green: {worst}");
    let edge = grouped.chunks(4).filter(|p| p[0] > 8 && p[0] < 247).count();
    assert!(edge > 50, "control: the corners antialias, {edge} pixels");
    let worst = fringe(&apart).into_iter().max().unwrap();
    assert!(worst > 30, "control: a layer each fringes, at most {worst}");
}

/// A grayscale row over a green one. In one clip layer it greys what is below
/// it right up to the rounded edge. Alone in its own clip layer it finds nothing
/// to blend with on the tiles that edge crosses, and green stays along the rim:
/// 82 pixels on RADV 890M, pinned as at least 50.
#[test]
#[ignore]
fn gpu_clip_edge_grayscale_reaches_the_edge_of_gathered_rows() {
    let green = "background-color: rgb(40, 200, 60)";
    let grey = "filter: grayscale(1)";
    // Pixels of pane A that kept their colour, and the middle of the pane.
    let green_left = |pixels: &[u8]| {
        pixels
            .chunks(4)
            .enumerate()
            .filter(|(i, p)| ((i % W as usize) as u32) < 200 && p[1] as i32 - p[0] as i32 > 30)
            .count()
    };
    let middle = |pixels: &[u8]| {
        let p = &pixels[((94 * W + 103) * 4) as usize..][..3];
        p[1] as i32 - p[0] as i32
    };
    let mut gpu = Gpu::new();
    let grouped = gpu.render(&mut rounded_pane(green, grey, true));
    let apart = gpu.render(&mut rounded_pane(green, grey, false));

    assert!(
        middle(&grouped) <= 3,
        "control: the row greys the middle: {}",
        middle(&grouped)
    );
    assert_eq!(green_left(&grouped), 0, "one layer leaves no green edge");
    assert!(
        green_left(&apart) >= 50,
        "control: a layer each leaves it, {} pixels",
        green_left(&apart)
    );
}
