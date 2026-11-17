//! #2309 (image placeholders) on the GPU painter: the boxes of
//! `rinch-dom/tests/img_placeholder_2309_tests.rs`, painted by `VelloPainter`
//! and rendered by Vello, at 1x and 2x. The ring is Chrome 156's 1px
//! `silver` inside the content box, the alt text inside its 1px of padding.
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_img_placeholder_2309 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 300;
const H: u32 = 120;

fn render(scale: u32) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (W * scale, H * scale);
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; font: 16px/20px sans-serif; color: #000; } \
         img { position: absolute; top: 10px }",
    );
    let body = doc.body();
    // No `src`, sized by its attributes; a failed load sized by CSS; an
    // unsized one with alt text (inline).
    for attrs in [
        &[
            ("style", "left: 10px"),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello world"),
        ][..],
        &[
            ("style", "left: 150px; width: 100px; height: 50px"),
            ("src", "data:image/png;base64,AAAA"),
            ("alt", "Hello world"),
        ][..],
        &[("style", "left: 10px; top: 80px"), ("alt", "Hello")][..],
    ] {
        let n = doc.create_element("img");
        for (k, v) in attrs {
            doc.set_attribute(n, k, v);
        }
        doc.append_child(body, n);
    }
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
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
            width: pw,
            height: ph,
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
                width: pw,
                height: ph,
                antialiasing_method: vello::AaConfig::Area,
            },
        )
        .unwrap();
    let row = (pw * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * ph) as u64,
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
                rows_per_image: Some(ph),
            },
        },
        wgpu::Extent3d {
            width: pw,
            height: ph,
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
    let px = (0..ph)
        .flat_map(|y| {
            let data = &data;
            (0..pw).map(move |x| {
                let i = (y * row + x * 4) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            })
        })
        .collect();
    (pw, px)
}

const WHITE: [u8; 3] = [255, 255, 255];
const SILVER: [u8; 3] = [0xc0, 0xc0, 0xc0];

fn check(scale: u32) {
    let (pw, px) = render(scale);
    let at = |x: u32, y: u32| -> [u8; 3] {
        let p = px[(y * pw + x) as usize];
        [p[0], p[1], p[2]]
    };
    let s = scale;
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(u, v)| u.abs_diff(v) <= 2);
    let ink = |c: [u8; 3]| c.iter().all(|&v| v < 0x90);
    let mut bad = Vec::new();
    for x0 in [10, 150] {
        // The ring: one CSS px, `s` device px, on the content box's edge.
        for (x, y) in [
            (x0 * s, 10 * s),
            ((x0 + 100) * s - 1, 60 * s - 1),
            (x0 * s + s - 1, 35 * s),
            ((x0 + 50) * s, 60 * s - s),
        ] {
            if !near(at(x, y), SILVER) {
                bad.push(((x, y), at(x, y), "ring"));
            }
        }
        for (x, y) in [
            (x0 * s + s, 11 * s),
            (x0 * s - 1, 9 * s),
            ((x0 + 100) * s, 35 * s),
        ] {
            if !near(at(x, y), WHITE) {
                bad.push(((x, y), at(x, y), "white"));
            }
        }
        let text = (12 * s..58 * s)
            .flat_map(|y| ((x0 + 2) * s..(x0 + 98) * s).map(move |x| (x, y)))
            .filter(|&(x, y)| ink(at(x, y)))
            .count();
        if text == 0 {
            bad.push(((x0, 12), WHITE, "no alt text"));
        }
    }
    let inline = (80 * s..110 * s)
        .flat_map(|y| (10 * s..80 * s).map(move |x| (x, y)))
        .filter(|&(x, y)| ink(at(x, y)))
        .count();
    if inline == 0 {
        bad.push(((10, 80), WHITE, "no inline alt text"));
    }
    assert!(bad.is_empty(), "{scale}x (point, rinch, what): {bad:?}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_img_placeholders_draw_the_box_and_alt_text() {
    check(1);
    check(2);
}
