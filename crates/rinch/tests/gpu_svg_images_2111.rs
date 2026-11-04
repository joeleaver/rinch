//! #2111 (SVG files as images) on the GPU painter: the markup of
//! `rinch-dom/tests/svg_images_2111_tests.rs`, painted by `VelloPainter` and
//! rendered by Vello, against Chrome 156's pixels at 1x and 2x. The rasters
//! are resvg's (the same as the software painter's); what this pins is that
//! Vello draws each one at the size it was made for, crisp. Needs an
//! adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_svg_images_2111 -- --ignored`.
#![cfg(feature = "gpu")]

use base64::Engine;
use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 200;
const H: u32 = 120;

const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><rect x="2.5" y="2" width="5" height="12" fill="#f00"/><circle cx="12" cy="8" r="3" fill="#00f"/></svg>"##;
const WIDE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8" viewBox="0 0 16 8"><rect width="16" height="8" fill="#f00"/></svg>"##;

fn b64(svg: &str) -> String {
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    )
}

/// Chrome 156's page: (tag, style, src).
fn boxes() -> Vec<(&'static str, String, Option<String>)> {
    vec![
        ("img", "left: 10px; top: 10px".into(), Some(b64(ICON))),
        (
            "img",
            "left: 40px; top: 10px; width: 32px; height: 32px".into(),
            Some(b64(ICON)),
        ),
        (
            "div",
            format!(
                "left: 80px; top: 10px; width: 60px; height: 40px; \
                 background: url(\"{}\") 0 0 / 20px 20px",
                b64(ICON)
            ),
            None,
        ),
        (
            "div",
            format!(
                "left: 150px; top: 10px; width: 32px; height: 32px; background-color: #0a0; \
                 mask-image: url(\"{}\"); mask-size: 100% 100%; mask-repeat: no-repeat",
                b64(ICON)
            ),
            None,
        ),
        (
            "img",
            "left: 10px; top: 60px; width: 40px; height: 40px".into(),
            Some(b64(WIDE)),
        ),
    ]
}

fn render(scale: u32) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (W * scale, H * scale);
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } .a { position: absolute; display: block; }",
    );
    let body = doc.body();
    for (tag, style, src) in boxes() {
        let n = doc.create_element(tag);
        doc.set_attribute(n, "class", "a");
        doc.set_attribute(n, "style", &style);
        if let Some(src) = src {
            doc.set_attribute(n, "src", &src);
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

const WH: [u8; 3] = [255, 255, 255];
const R: [u8; 3] = [255, 0, 0];
const B: [u8; 3] = [0, 0, 255];
const G: [u8; 3] = [0, 170, 0];

/// `(point, Chrome 156, tolerance)`; an interior pixel exact to a few
/// levels, an anti-aliased edge within Vello's area coverage of Skia's.
fn check(scale: u32, want: &[((u32, u32), [u8; 3], u8)]) {
    let (pw, px) = render(scale);
    let at = |x: u32, y: u32| -> [u8; 3] {
        let p = px[(y * pw + x) as usize];
        [p[0], p[1], p[2]]
    };
    let bad: Vec<_> = want
        .iter()
        .filter_map(|&((x, y), w, tol)| {
            let got = at(x, y);
            (got.iter().zip(w).any(|(a, b)| a.abs_diff(b) > tol)).then_some(((x, y), got, w))
        })
        .collect();
    assert!(bad.is_empty(), "{scale}x (point, rinch, chrome): {bad:?}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_svg_images_match_chrome() {
    check(
        1,
        &[
            // `<img>` at its natural 16px: the bar's half-pixel edges, the dot.
            ((11, 12), WH, 4),
            ((12, 12), [255, 127, 127], 24),
            ((13, 12), R, 4),
            ((16, 12), R, 4),
            ((17, 12), [255, 127, 127], 24),
            ((18, 12), WH, 4),
            ((21, 18), B, 4),
            // Drawn at 32px: crisp from 45, where a 16px raster scaled up
            // blends 44.
            ((44, 24), WH, 4),
            ((45, 24), R, 4),
            ((54, 24), R, 4),
            ((55, 24), WH, 4),
            ((63, 24), B, 4),
            // The 20px background tiles.
            ((84, 18), R, 4),
            ((104, 18), R, 4),
            ((124, 18), R, 4),
            ((90, 18), WH, 4),
            ((94, 18), B, 4),
            // The tinted icon.
            ((155, 26), G, 4),
            ((164, 26), G, 4),
            ((165, 26), WH, 4),
            ((172, 26), G, 4),
            // Letterboxed by its viewBox.
            ((30, 69), WH, 4),
            ((30, 70), R, 4),
            ((30, 89), R, 4),
            ((30, 90), WH, 4),
        ],
    );
    check(
        2,
        &[
            ((24, 36), WH, 4),
            ((25, 36), R, 4),
            ((34, 36), R, 4),
            ((35, 36), WH, 4),
            ((38, 36), [23, 23, 255], 40),
            ((40, 36), B, 4),
            ((89, 48), WH, 4),
            ((90, 48), R, 4),
            ((109, 48), R, 4),
            ((110, 48), WH, 4),
            ((167, 36), R, 4),
            ((207, 36), R, 4),
            ((310, 52), G, 4),
            ((329, 52), G, 4),
            ((330, 52), WH, 4),
        ],
    );
}
