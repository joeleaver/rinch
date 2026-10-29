//! #549 on the GPU painter: a `position: fixed` box escapes every clip
//! between it and the viewport — the clips above the stacking context that
//! owns it included — and is drawn by Vello where the software painter and
//! Chrome 155 draw it. Vello clips a layer's content to the layer, so a clip
//! left pushed (or an opacity layer re-pushed with too small a bounds) is lost
//! here and only here. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_fixed_escape_549 -- --ignored`.
//!
//! The markup and probes are `rinch-dom/tests/fixed_escape_549_tests.rs`'s.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 800;
const H: u32 = 600;

const GREEN: [u8; 4] = [0, 200, 0, 255];
const BLUE: [u8; 4] = [0, 0, 200, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

const FIXED: &str = "position: fixed; left: 130px; top: 110px; width: 270px; height: 290px; \
                     background-color: rgb(0, 200, 0)";
const IN: (u32, u32) = (150, 130);
const OUT: (u32, u32) = (380, 380);
const CLIPPER: &str = "overflow: hidden; width: 200px; height: 200px";
const OWNER: &str = "position: relative; z-index: 1; width: 50px; height: 50px";

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// `body > s[0] > … > last`, laid out.
fn chain(styles: &[&str], last: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let mut parent = body;
    for s in styles.iter().chain(std::iter::once(&last)) {
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
        .all(|(x, y)| (*x as i32 - y as i32).abs() <= 2)
}

/// `assert_eq!` within 2 per channel: Vello's quantisation, not a tolerance
/// for a wrong colour (every expectation is a solid fill or white).
macro_rules! assert_near {
    ($got:expr, $want:expr) => {
        assert_near!($got, $want, "")
    };
    ($got:expr, $want:expr, $($msg:tt)+) => {{
        let (got, want) = ($got, $want);
        assert!(near(got, want), "{got:?} != {want:?}: {}", format!($($msg)+));
    }};
}

/// Every shape of the software fixtures, rendered by Vello, against the
/// colours Chrome 155 draws there.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_fixed_box_escapes_the_clips_above_its_context() {
    // The issue's shape, and two levels of contexts that both clip.
    for styles in [
        &[CLIPPER, OWNER][..],
        &[
            CLIPPER,
            "position: relative; z-index: 1; width: 100px; height: 100px",
            "position: relative; z-index: 1; overflow: hidden; width: 50px; height: 50px",
        ][..],
    ] {
        let px = render(&mut chain(styles, FIXED));
        assert_near!(at(&px, IN), GREEN, "inside, {styles:?}");
        assert_near!(at(&px, OUT), GREEN, "outside, {styles:?}");
    }

    // Two clippers above an `opacity` owner: drawn as with no clipper.
    let owner = "opacity: 0.5; width: 50px; height: 50px";
    let px = render(&mut chain(
        &[
            CLIPPER,
            "overflow: clip; width: 150px; height: 150px",
            owner,
        ],
        FIXED,
    ));
    let reference = render(&mut chain(&[owner], FIXED));
    let faded = at(&reference, OUT);
    assert!(
        faded[0] > 100 && faded[0] < 160 && faded[1] > 200,
        "positive control: half-faded green on white, {faded:?}"
    );
    assert!(near(at(&px, OUT), faded), "{:?} vs {faded:?}", at(&px, OUT));
    assert!(near(at(&px, IN), faded), "{:?} vs {faded:?}", at(&px, IN));

    // A rounded clipper's cut corner.
    let px = render(&mut chain(
        &[
            "overflow: hidden; border-radius: 60px; width: 200px; height: 200px",
            OWNER,
        ],
        "position: fixed; left: 0; top: 0; width: 300px; height: 300px; \
         background-color: rgb(0, 200, 0)",
    ));
    assert_near!(at(&px, (4, 4)), GREEN, "the cut corner");
    assert_near!(at(&px, (250, 250)), GREEN, "outside the clipper");

    // A transformed ancestor keeps the clip above it.
    let px = render(&mut chain(
        &[
            CLIPPER,
            "transform: translate(0px, 0px); width: 100px; height: 100px",
            OWNER,
        ],
        FIXED,
    ));
    assert_near!(at(&px, IN), GREEN, "positive control");
    assert_near!(at(&px, OUT), WHITE, "clipped above the transform");
}

/// What the owner and the clipper paint after the fixed box is clipped again.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_what_paints_after_the_fixed_box_is_clipped_again() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(&mut doc, body, CLIPPER);
    let owner = el(&mut doc, clipper, OWNER);
    el(&mut doc, owner, FIXED);
    el(
        &mut doc,
        owner,
        "position: absolute; z-index: 2; left: 0; top: 20px; width: 700px; height: 30px; \
         background-color: rgb(0, 0, 200)",
    );
    el(
        &mut doc,
        clipper,
        "position: relative; z-index: 3; top: 100px; width: 700px; height: 30px; \
         background-color: rgb(0, 0, 200)",
    );
    doc.resolve_layout(W as f32, H as f32);
    let px = render(&mut doc);
    assert_near!(at(&px, (100, 30)), BLUE, "positive control");
    assert_near!(at(&px, (600, 30)), WHITE);
    assert_near!(at(&px, (100, 160)), BLUE, "positive control");
    assert_near!(at(&px, (600, 160)), WHITE);
    assert_near!(at(&px, OUT), GREEN);
}
