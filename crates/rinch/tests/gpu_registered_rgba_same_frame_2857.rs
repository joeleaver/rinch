//! #2857 on the GPU painter: an element whose picture moves from one name
//! registered with `register_rgba` to another draws the new picture in the
//! frame that applies the change, painted by `VelloPainter` and rendered by
//! Vello, as `rinch-dom`'s `registered_rgba_same_frame_2857_tests` holds the
//! software painter to. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu-renderer --test gpu_registered_rgba_same_frame_2857 -- --ignored`.
#![cfg(feature = "gpu-renderer")]

use peniko::Brush;
use rinch::image::{register_rgba, rgba_src, unregister_rgba};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::FileImageLoader;
use rinch_dom::paint::vello_painter::VelloPainter;
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u32 = 60;
const H: u32 = 40;

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// A 2x2 picture all `rgb`, opaque.
fn solid(rgb: [u8; 3]) -> Arc<[u8]> {
    [[rgb[0], rgb[1], rgb[2], 255]; 4].concat().into()
}

/// Blue, red and green under names of `test`'s own: blue, registered first,
/// is never the one under test.
fn register(test: &str) -> [String; 3] {
    let names = [
        format!("gpu-2857-{test}-blue"),
        format!("gpu-2857-{test}-red"),
        format!("gpu-2857-{test}-green"),
    ];
    for (name, rgb) in names.iter().zip([BLUE, RED, GREEN]) {
        register_rgba(name, 2, 2, solid(rgb)).unwrap();
    }
    names
}

/// A 40x20 box at (10, 10).
const BOX: &str = "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px; \
     display: block";

fn background(src: &str) -> String {
    format!("{BOX}; background-image: url({src}); background-size: 40px 20px")
}

/// A document with one `tag` (an `<img>` or a `div` with a background)
/// showing `src`, settled on that picture however it arrives.
fn document(tag: &str, src: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    doc.tree.image_loader = Some(Arc::new(FileImageLoader));
    let body = doc.body();
    let node = doc.create_element(tag);
    if tag == "img" {
        doc.set_attribute(node, "src", src);
        doc.set_attribute(node, "style", BOX);
    } else {
        doc.set_attribute(node, "style", &background(src));
    }
    doc.append_child(body, node);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut wide = false;
    while doc.tree.image_cache.get(src).is_none() {
        assert!(Instant::now() < deadline, "the picture never arrived");
        wide = !wide;
        doc.resolve_layout(W as f32 + if wide { 1.0 } else { 0.0 }, H as f32);
        std::thread::sleep(Duration::from_millis(2));
    }
    (doc, node)
}

/// One frame (a layout, then a paint and a render): its box's middle pixel.
fn middle(doc: &mut RinchDocument) -> [u8; 3] {
    let p = render(doc)[(20 * W + 30) as usize];
    [p[0], p[1], p[2]]
}

fn render(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
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

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_background_moved_to_another_registered_name_draws_it_in_that_frame() {
    let names = register("bg");
    let (mut doc, div) = document("div", &rgba_src(&names[1]));
    assert_eq!(middle(&mut doc), RED);
    doc.set_attribute(div, "style", &background(&rgba_src(&names[2])));
    assert_eq!(
        middle(&mut doc),
        GREEN,
        "the frame that applies the new background draws its picture"
    );
    for name in &names {
        unregister_rgba(name);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_img_moved_to_another_registered_name_draws_it_in_that_frame() {
    let names = register("img");
    let (mut doc, img) = document("img", &rgba_src(&names[1]));
    assert_eq!(middle(&mut doc), RED);
    doc.set_attribute(img, "src", &rgba_src(&names[2]));
    assert_eq!(
        middle(&mut doc),
        GREEN,
        "the frame that applies the new src draws its picture"
    );
    for name in &names {
        unregister_rgba(name);
    }
}
