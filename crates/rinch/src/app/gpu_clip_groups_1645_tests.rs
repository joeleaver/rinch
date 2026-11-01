//! #1645: both renderers paint a stacking sequence gathered by clip, and agree.
//! `rinch-dom`'s `clip_groups_1645_tests` pin the picture on the software
//! painter. The grouping happens above the `Painter` trait, so Vello is handed
//! the same sequence; this renders it. It needs an adapter:
//! `cargo test -p rinch --features desktop,gpu --lib -- --ignored gpu --test-threads=1`.
use super::gpu_partial_frame_1718_tests::readback;
use super::*;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 320;
const H: u32 = 200;

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

fn software(app: &mut RinchApp) -> Vec<u8> {
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

#[test]
#[ignore]
fn gpu_clip_groups_match_the_software_painter() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut renderer = vello::Renderer::new(
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
    for second in [171, 139] {
        let mut app = app(second);
        let cpu = software(&mut app);
        let GpuFrame::Full(scene) = app.build_gpu_frame(
            1.0,
            (W, H),
            Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
        ) else {
            panic!("the first frame is full")
        };
        renderer
            .render_to_texture(
                &device,
                &queue,
                scene,
                &target.create_view(&Default::default()),
                &vello::RenderParams {
                    base_color: vello::peniko::Color::WHITE,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .unwrap();
        let gpu = readback(&device, &queue, &target, W, H);
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
