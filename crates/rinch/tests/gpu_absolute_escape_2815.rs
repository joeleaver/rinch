//! #2815 on the GPU painter: an absolute box whose containing block is
//! above the stacking context that paints it escapes the clips between the
//! two, and is drawn by Vello where the software painter and Chrome 158
//! draw it. Vello clips a layer's content to the layer's bounds, so a
//! translucent context's layer narrowed by a clip its absolute box escapes
//! (`layer_bounds`), or a clip left pushed, loses the box here and only
//! here. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_absolute_escape_2815 -- --ignored`.
//!
//! The markup and probes are `rinch-dom/tests/absolute_escape_2815_tests.rs`'s.
#![cfg(feature = "gpu-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 600;
const H: u32 = 400;

const WHITE: [u8; 4] = [255, 255, 255, 255];
/// The green absolute box at `opacity: 0.5` over white (Chrome 158).
const FADED_GREEN: [u8; 4] = [126, 227, 126, 255];
/// The blue context background at `opacity: 0.5` over white.
const FADED_BLUE: [u8; 4] = [126, 126, 227, 255];

const CB: &str = "position: relative; width: 300px; height: 300px";
const CLIPPER: &str = "overflow: hidden; width: 100px; height: 100px";
const CONTEXT: &str = "opacity: 0.5; width: 50px; height: 50px; background-color: rgb(0, 0, 200)";
const ABSOLUTE: &str = "position: absolute; left: 25px; top: 25px; width: 200px; height: 200px; \
                        background-color: rgb(0, 200, 0)";
const BG: (u32, u32) = (10, 10);
const OVERLAP: (u32, u32) = (40, 40);
const OUT: (u32, u32) = (200, 200);

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// `body > s[0] > … > last`, laid out.
fn chain(styles: &[&str]) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let mut parent = body;
    for s in styles {
        parent = el(&mut doc, parent, s);
    }
    doc.resolve_layout(W as f32, H as f32);
    doc
}

/// The pixels Vello renders for `doc`, on white.
fn render(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );

    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("a GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
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
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_texture(
            &device,
            &queue,
            painter.scene(),
            &view,
            &vello::RenderParams {
                base_color: peniko::Color::WHITE,
                width: W,
                height: H,
                antialiasing_method: vello::AaConfig::Area,
            },
        )
        .unwrap();
    let row = (W * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("the buffer is mapped");
    (0..H)
        .flat_map(|y| {
            let data = &data;
            (0..W).map(move |x| {
                let i = (y * row + x * 4) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            })
        })
        .collect()
}

fn at(px: &[[u8; 4]], (x, y): (u32, u32)) -> [u8; 4] {
    px[(y * W + x) as usize]
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

/// `assert_eq!` within 3 per channel: Vello's and Skia's rounding of an
/// `opacity` composite, not a tolerance for a wrong colour.
macro_rules! assert_near {
    ($got:expr, $want:expr) => {
        assert_near!($got, $want, "")
    };
    ($got:expr, $want:expr, $($msg:tt)+) => {{
        let (got, want) = ($got, $want);
        assert!(near(got, want), "{got:?} != {want:?}: {}", format!($($msg)+));
    }};
}

/// The issue's shape: the clipper is not between the box and its
/// containing block, so Vello draws the box whole, in its context's group.
///
/// Kills: the context's group not sunk under the escapable clips (the
/// overlap is faded twice), and no lift (`OUT` is white).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_absolute_box_escapes_a_clipper_below_its_containing_block() {
    let px = render(&mut chain(&[CB, CLIPPER, CONTEXT, ABSOLUTE]));
    assert_near!(at(&px, BG), FADED_BLUE, "the context");
    assert_near!(at(&px, OVERLAP), FADED_GREEN, "over the context");
    assert_near!(at(&px, OUT), FADED_GREEN, "outside the clipper");
}

/// The context clips its own overflow and is not the box's containing
/// block: Chrome draws the box past that clip. The context's layer is the
/// one Vello clips to, so its bounds must not be narrowed by its own clip.
///
/// Kills: `layer_bounds` narrowing a root by its own clip although an
/// absolute inside escapes it (the box is cut to the context's 100x100).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_absolute_box_escapes_the_clip_of_its_own_context() {
    let context = "opacity: 0.5; overflow: hidden; width: 100px; height: 100px; \
                   background-color: rgb(0, 0, 200)";
    let px = render(&mut chain(&[CB, context, ABSOLUTE]));
    assert_near!(at(&px, BG), FADED_BLUE, "the context");
    assert_near!(at(&px, OVERLAP), FADED_GREEN, "over the context");
    assert_near!(at(&px, OUT), FADED_GREEN, "outside its clip");
}

/// A `clip-path` element between the clipper and the box keeps the
/// clipper's clip on (Chrome 158).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_clip_path_between_keeps_the_clipper() {
    let px = render(&mut chain(&[
        CB,
        CLIPPER,
        "clip-path: inset(0); width: 150px; height: 150px",
        CONTEXT,
        ABSOLUTE,
    ]));
    assert_near!(at(&px, (75, 75)), FADED_GREEN, "positive control");
    assert_near!(at(&px, (120, 120)), WHITE, "past the clipper");
}
