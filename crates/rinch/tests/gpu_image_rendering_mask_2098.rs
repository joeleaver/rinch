//! #2098 (`image-rendering`) and #2099 (`mask-image: url(…)`) on the GPU
//! painter: the markup of `rinch-dom/tests/image_rendering_mask_2098_2099_tests.rs`,
//! painted by `VelloPainter` and rendered by Vello, against Chrome 156's
//! pixels. On the GPU `auto` is bilinear (as Chrome's is), so this is where
//! `pixelated` changes most. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_image_rendering_mask_2098 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 400;
const H: u32 = 220;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// (style, tag, src) per box: Chrome 156's oracle page, one box per case.
const BOXES: &[(&str, &str, &str)] = &[
    ("left: 0; top: 0; width: 40px; height: 40px", "img", "q.png"),
    (
        "left: 50px; top: 0; width: 40px; height: 40px; image-rendering: pixelated",
        "img",
        "q.png",
    ),
    (
        "left: 100px; top: 0; width: 40px; height: 40px; image-rendering: crisp-edges",
        "img",
        "q.png",
    ),
    (
        "left: 0; top: 50px; width: 6px; height: 6px",
        "img",
        "c.png",
    ),
    (
        "left: 50px; top: 50px; width: 6px; height: 6px; image-rendering: pixelated",
        "img",
        "c.png",
    ),
    (
        "left: 100px; top: 50px; width: 6px; height: 6px; image-rendering: crisp-edges",
        "img",
        "c.png",
    ),
    (
        "left: 200px; top: 100px; width: 80px; height: 40px; background: url(q.png) 0.25px 0 / 20px 20px repeat; image-rendering: pixelated",
        "div",
        "",
    ),
    (
        "left: 0; top: 150px; width: 40px; height: 40px; background: red; mask-image: url(m.png); mask-size: 100% 100%",
        "div",
        "",
    ),
    (
        "left: 100px; top: 150px; width: 40px; height: 40px; background: red; mask-image: url(h.png)",
        "div",
        "",
    ),
    (
        "left: 150px; top: 150px; width: 40px; height: 40px; background: red; mask-image: url(h.png); mask-repeat: no-repeat; mask-position: 8px 8px",
        "div",
        "",
    ),
    (
        "left: 200px; top: 150px; width: 40px; height: 40px; background: red; mask-image: url(/nonexistent-2099/heart.png)",
        "div",
        "",
    ),
    (
        "left: 250px; top: 150px; width: 40px; height: 40px; background: red; mask-image: url(m.png); mask-size: 100% 100%; image-rendering: pixelated",
        "div",
        "",
    ),
];

fn images() -> [(&'static str, DecodedImage); 4] {
    let quad = DecodedImage::new([RED, GREEN, BLUE, WHITE].concat(), 2, 2);
    let checker: Vec<[u8; 4]> = (0..64)
        .map(|i| {
            if (i % 8 + i / 8) % 2 == 0 {
                BLACK
            } else {
                WHITE
            }
        })
        .collect();
    let half: Vec<[u8; 4]> = (0..16)
        .map(|i| if i % 4 < 2 { BLACK } else { CLEAR })
        .collect();
    let holed: Vec<[u8; 4]> = (0..16)
        .map(|i| {
            let (x, y) = (i % 4, i / 4);
            if (1..=2).contains(&x) && (1..=2).contains(&y) {
                CLEAR
            } else if (x, y) == (0, 0) {
                [0, 0, 0, 128]
            } else {
                BLACK
            }
        })
        .collect();
    [
        ("q.png", quad),
        ("c.png", DecodedImage::new(checker.concat(), 8, 8)),
        ("m.png", DecodedImage::new(half.concat(), 4, 4)),
        ("h.png", DecodedImage::new(holed.concat(), 4, 4)),
    ]
}

fn render() -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } .a { position: absolute; display: block; }",
    );
    for (url, image) in images() {
        doc.tree.image_cache.insert_decoded(url.to_string(), image);
    }
    let body = doc.body();
    for &(style, tag, src) in BOXES {
        let n = doc.create_element(tag);
        doc.set_attribute(n, "class", "a");
        doc.set_attribute(n, "style", style);
        if !src.is_empty() {
            doc.set_attribute(n, "src", src);
        }
        doc.append_child(body, n);
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

const R: [u8; 3] = [255, 0, 0];
const G: [u8; 3] = [0, 255, 0];
const B: [u8; 3] = [0, 0, 255];
const WH: [u8; 3] = [255, 255, 255];

/// Chrome 156's pixels for the same markup (on white).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_image_rendering_and_url_masks_match_chrome() {
    let px = render();
    if let Ok(path) = std::env::var("RINCH_2098_GPU_DUMP") {
        let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
        for p in &px {
            ppm.extend_from_slice(&p[..3]);
        }
        std::fs::write(path, ppm).unwrap();
    }
    let at = |x: u32, y: u32| -> [u8; 3] {
        let p = px[(y * W + x) as usize];
        [p[0], p[1], p[2]]
    };
    let mut want: Vec<((u32, u32), [u8; 3])> = Vec::new();
    let mut row = |x0: u32, y: u32, cols: &[[u8; 3]]| {
        for (i, &c) in cols.iter().enumerate() {
            want.push(((x0 + i as u32, y), c));
        }
    };
    // `pixelated` and `crisp-edges` upscale nearest: 20px texels.
    for x0 in [50, 100] {
        row(x0 + 15, 5, &[R, R, R, R, R, G, G, G, G, G]);
        row(x0 + 5, 25, &[B]);
        row(x0 + 35, 35, &[WH]);
    }
    // A repeating background at `0.25px`, `pixelated`: hard tile edges.
    let mut bg = vec![R; 10];
    bg.extend([G; 10]);
    bg.extend([R; 4]);
    row(200, 105, &bg);
    // A url mask at `mask-size: 100% 100%`: the left half shows (the blended
    // middle columns are compared loosely below).
    row(2, 170, &[R; 12]);
    row(27, 170, &[WH; 12]);
    // `pixelated` on the mask: a hard edge at x = 20.
    row(264, 170, &[R, R, R, R, R, R, WH, WH, WH, WH, WH, WH]);
    // The holed mask at its natural size, repeated, and once at 8px 8px.
    let half = [255, 127, 127];
    row(100, 150, &[half, R, R, R, half, R, R, R, half, R]);
    row(100, 151, &[R, WH, WH, R, R, WH, WH, R, R, WH]);
    row(156, 158, &[WH, WH, half, R, R, R, WH]);
    row(156, 159, &[WH, WH, R, WH, WH, R, WH]);
    row(156, 161, &[WH, WH, R, R, R, R, WH]);
    row(170, 170, &[WH]);
    // A mask image that never loads masks the box away.
    row(220, 170, &[WH]);
    let bad: Vec<_> = want
        .iter()
        .filter(|&&((x, y), w)| at(x, y).iter().zip(w).any(|(a, b)| a.abs_diff(b) > 2))
        .map(|&((x, y), w)| ((x, y), at(x, y), w))
        .collect();
    assert!(bad.is_empty(), "(point, vello, chrome): {bad:?}");

    // `auto` is smooth on the GPU, as in Chrome: the pixel between the red
    // and green texels is a blend (Chrome 156: (127, 127, 0) at x = 20).
    let mid = at(20, 5);
    assert!(
        mid[0] > 60 && mid[1] > 60 && mid[2] < 10,
        "auto blends: {mid:?}"
    );
    // So is the url mask's edge under `auto` (Chrome: (255, 128, 128) at 20).
    let edge = at(20, 170);
    assert!(
        edge[0] == 255 && (40..220).contains(&edge[1]),
        "auto mask edge blends: {edge:?}"
    );
    // A 1px checker downscaled 8 -> 6: `auto` averages into greys, the
    // nearest keywords keep every pixel black or white (Chrome 156 both).
    let pure = |x0: u32| {
        (0..6).all(|y| (0..6).all(|x| at(x0 + x, 50 + y).iter().all(|&c| c <= 2 || c >= 253)))
    };
    assert!(!pure(0), "auto downscale is smooth");
    assert!(pure(50), "pixelated downscale is nearest");
    assert!(pure(100), "crisp-edges downscale is nearest");
}
