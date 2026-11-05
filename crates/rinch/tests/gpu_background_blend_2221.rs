//! #2221 (`background-blend-mode`) on the GPU painter: the boxes of
//! `rinch-dom/tests/background_blend_2221_tests.rs`, in one document (the
//! Chrome oracle page's layout), painted by `VelloPainter` and rendered by
//! Vello, against Chrome 156's pixels. Vello blends in f32 and stores the
//! result rounded, where Chrome's Skia rounds its 8-bit premultiplied
//! intermediates, so the two differ by a step either way; [`TOL`] is the
//! most any case here does. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_background_blend_2221 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 400;
const H: u32 = 220;

/// Per channel, against Chrome (see the module docs).
const TOL: u8 = 1;

const MODES: [&str; 16] = [
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

/// Chrome 156's pixel at the centre of each mode's box: `Cs = rgb(51, 153,
/// 230)` blended onto `Cb = rgb(204, 102, 51)`.
const CHROME_MODES: [[u8; 3]; 16] = [
    [51, 153, 230],
    [40, 61, 46],
    [214, 194, 235],
    [173, 122, 92],
    [51, 102, 51],
    [204, 153, 230],
    [254, 254, 255],
    [0, 0, 28],
    [81, 132, 214],
    [179, 114, 102],
    [153, 51, 179],
    [174, 133, 189],
    [59, 146, 212],
    [217, 98, 38],
    [47, 149, 226],
    [207, 105, 54],
];

const CS: &str = "linear-gradient(rgb(51, 153, 230), rgb(51, 153, 230))";
const CS_HALF: &str = "linear-gradient(rgba(51, 153, 230, 0.5), rgba(51, 153, 230, 0.5))";
const C2: &str = "linear-gradient(rgb(120, 200, 40), rgb(120, 200, 40))";
const C2_HALF: &str = "linear-gradient(rgba(120, 200, 40, 0.5), rgba(120, 200, 40, 0.5))";
const CB: &str = "rgb(204, 102, 51)";

/// The boxes after the first row, as `(style, parent index)`: a parent index
/// names an earlier box of this list.
fn boxes() -> Vec<(String, Option<usize>)> {
    vec![
        (
            format!(
                "left: 0; top: 30px; width: 20px; height: 20px; background-image: {CS_HALF}; \
                 background-color: {CB}; background-blend-mode: multiply"
            ),
            None,
        ),
        (
            "left: 30px; top: 30px; width: 40px; height: 20px; background: red".into(),
            None,
        ),
        (
            format!(
                "left: 10px; top: 0; width: 20px; height: 20px; background-image: {CS}; \
                 background-blend-mode: multiply"
            ),
            Some(1),
        ),
        (
            format!(
                "left: 80px; top: 30px; width: 20px; height: 20px; background-image: {CS}, {C2_HALF}; \
                 background-color: {CB}; background-blend-mode: multiply, screen"
            ),
            None,
        ),
        (
            format!(
                "left: 110px; top: 30px; width: 20px; height: 20px; background-image: {CS_HALF}, {C2}; \
                 background-color: {CB}; background-blend-mode: difference"
            ),
            None,
        ),
        (
            "left: 0; top: 60px; width: 40px; height: 40px; background-image: url(t2221.png); \
             background-size: 100% 100%; background-color: rgb(255, 128, 64); \
             background-blend-mode: multiply; mask-image: url(t2221.png); mask-size: 100% 100%; \
             image-rendering: pixelated"
                .into(),
            None,
        ),
        (
            format!(
                "left: 50px; top: 60px; width: 20px; height: 20px; opacity: 0.5; \
                 background-image: {CS}; background-color: {CB}; background-blend-mode: screen"
            ),
            None,
        ),
        (
            format!(
                "left: 80px; top: 60px; width: 20px; height: 20px; transform: scale(2); \
                 transform-origin: 0 0; background-image: {CS}; background-color: {CB}; \
                 background-blend-mode: color-burn"
            ),
            None,
        ),
        (
            format!(
                "left: 130px; top: 60px; width: 40px; height: 40px; border-radius: 20px; \
                 background-image: {CS}; background-color: {CB}; background-blend-mode: luminosity"
            ),
            None,
        ),
        (
            "left: 180px; top: 60px; width: 20px; height: 20px; overflow: hidden".into(),
            None,
        ),
        (
            format!(
                "left: 0; top: 0; width: 40px; height: 40px; background-image: {CS}; \
                 background-color: {CB}; background-blend-mode: hue"
            ),
            Some(9),
        ),
    ]
}

fn render() -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } div { position: absolute; display: block; }",
    );
    doc.tree.image_cache.insert_decoded(
        "t2221.png".into(),
        DecodedImage::new(
            [
                [255, 255, 255, 255],
                [200, 150, 100, 255],
                [80, 160, 240, 255],
                [0, 0, 0, 0],
            ]
            .concat(),
            2,
            2,
        ),
    );
    let body = doc.body();
    for (i, m) in MODES.iter().enumerate() {
        let n = doc.create_element("div");
        doc.set_attribute(
            n,
            "style",
            &format!(
                "left: {}px; top: 0; width: 20px; height: 20px; background-image: {CS}; \
                 background-color: {CB}; background-blend-mode: {m}",
                i * 20
            ),
        );
        doc.append_child(body, n);
    }
    let mut made: Vec<NodeId> = Vec::new();
    for (style, parent) in boxes() {
        let n = doc.create_element("div");
        doc.set_attribute(n, "style", &style);
        doc.append_child(parent.map_or(body, |p| made[p]), n);
        made.push(n);
    }
    doc.resolve_layout(W as f32, H as f32);
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

/// Chrome 156's pixels for the same markup (on white).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_background_blend_modes_match_chrome() {
    let px = render();
    let at = |x: u32, y: u32| -> [u8; 3] {
        let p = px[(y * W + x) as usize];
        [p[0], p[1], p[2]]
    };
    let mut want: Vec<(&str, (u32, u32), [u8; 3])> = MODES
        .iter()
        .zip(CHROME_MODES)
        .enumerate()
        .map(|(i, (m, c))| (*m, (i as u32 * 20 + 10, 10), c))
        .collect();
    want.extend([
        ("half-alpha multiply", (10, 40), [122, 81, 48]),
        ("isolated", (50, 40), [51, 153, 230]),
        ("parent", (35, 40), [255, 0, 0]),
        ("multiply, screen", (90, 40), [43, 97, 60]),
        ("difference repeated", (120, 40), [59, 76, 115]),
        ("white texel", (10, 70), [255, 128, 64]),
        ("orange texel", (30, 70), [200, 75, 25]),
        ("blue texel", (10, 90), [80, 80, 60]),
        ("clear texel", (30, 90), [255, 255, 255]),
        ("opacity 0.5 over white", (60, 70), [234, 224, 244]),
        ("scale(2)", (100, 80), [0, 0, 28]),
        ("rounded", (150, 80), [207, 105, 54]),
        ("rounded corner", (132, 62), [255, 255, 255]),
        ("clipped", (190, 70), [59, 146, 212]),
        ("outside the clip", (210, 70), [255, 255, 255]),
    ]);
    let bad: Vec<_> = want
        .iter()
        .filter(|&&(_, (x, y), w)| at(x, y).iter().zip(w).any(|(a, b)| a.abs_diff(b) > TOL))
        .map(|&(name, (x, y), w)| (name, at(x, y), w))
        .collect();
    assert!(bad.is_empty(), "(case, vello, chrome): {bad:?}");
}
