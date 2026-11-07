//! #2311 on the GPU painter: the `<img>`s of
//! `rinch-dom/tests/img_content_box_2311_tests.rs`, painted by `VelloPainter`
//! and rendered by Vello, against the same Chrome 156 pixels. A decoded
//! picture is drawn in its content box (inside the padding and border),
//! fitted there by `object-fit`, and clipped to the content edge's curve.
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_img_content_box_2311 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 400;
const H: u32 = 160;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

const R: [u8; 3] = [255, 0, 0];
const G: [u8; 3] = [0, 255, 0];
const B: [u8; 3] = [0, 0, 255];
const WH: [u8; 3] = [255, 255, 255];
/// The boxes' background, which shows in their padding.
const BG: [u8; 3] = [255, 0, 255];
/// Their border.
const BD: [u8; 3] = [128, 128, 128];

const CSS: &str = "html, body { margin: 0; background: #fff; }
.a { position: absolute; display: block; image-rendering: pixelated; }
.pb { padding: 7px 3px 5px 9px; border: 4px solid rgb(128, 128, 128);
      background: rgb(255, 0, 255); width: 40px; height: 20px; }
.r { padding: 6px; border: 4px solid rgb(128, 128, 128); background: rgb(255, 0, 255);
     width: 60px; height: 60px; border-radius: 20px; }";

/// (class, src, style) per `<img>`.
const BOXES: &[(&str, &str, &str)] = &[
    ("a pb", "q.png", "left: 10px; top: 10px"),
    (
        "a pb",
        "q.png",
        "left: 100px; top: 10px; object-fit: contain",
    ),
    ("a pb", "q.png", "left: 190px; top: 10px; object-fit: cover"),
    ("a pb", "g.png", "left: 280px; top: 10px; object-fit: none"),
    ("a r", "q.png", "left: 10px; top: 70px"),
    ("a r", "g.png", "left: 190px; top: 70px; object-fit: cover"),
    // A percentage padding (2% of the 400px containing block: 8px), which
    // only layout resolves (review of #2311).
    (
        "a",
        "q.png",
        "left: 300px; top: 70px; width: 16px; height: 16px; padding: 2%; \
         border: 1px solid rgb(128, 128, 128); background: rgb(255, 0, 255)",
    ),
    // A failed load, rounded: its #2309 placeholder is clipped to the curve.
    (
        "a",
        BROKEN,
        "left: 340px; top: 100px; width: 50px; height: 50px; border-radius: 25px; \
         background: rgb(255, 0, 255)",
    ),
];

/// A `data:` URL that is not a picture: its load fails, synchronously.
const BROKEN: &str = "data:image/png;base64,AAAA";

/// `q.png`: 2x2, red green / blue white. `g.png`: 80x40, its left half red
/// and its right half green.
fn images() -> [(&'static str, DecodedImage); 2] {
    let wide: Vec<[u8; 4]> = (0..80 * 40)
        .map(|i| if i % 80 < 40 { RED } else { GREEN })
        .collect();
    [
        (
            "q.png",
            DecodedImage::new([RED, GREEN, BLUE, WHITE].concat(), 2, 2),
        ),
        ("g.png", DecodedImage::new(wide.concat(), 80, 40)),
    ]
}

fn render() -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    for (url, image) in images() {
        doc.tree.image_cache.insert_decoded(url.to_string(), image);
    }
    let body = doc.body();
    for &(class, src, style) in BOXES {
        let n = doc.create_element("img");
        doc.set_attribute(n, "class", class);
        doc.set_attribute(n, "src", src);
        doc.set_attribute(n, "style", style);
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

/// Chrome 156's pixels for [`BOXES`], `((x, y), rgb)`.
const CHROME: &[((u32, u32), [u8; 3])] = &[
    // `fill`: border 10..14, left padding 14..23, content 23..63 x 21..41.
    ((11, 30), BD),
    ((18, 30), BG),
    ((22, 21), BG),
    ((23, 21), R),
    ((61, 22), G),
    ((24, 39), B),
    ((62, 40), WH),
    ((63, 40), BG),
    // `contain`: the 20x20 picture centred in the content box, the
    // letterbox the background.
    ((118, 30), BG),
    ((122, 30), BG),
    ((123, 30), R),
    ((124, 22), R),
    ((142, 22), G),
    ((143, 22), BG),
    // `cover`: 40x40 centred, cut at the content box, not the border box.
    ((205, 20), BG),
    ((205, 21), R),
    ((205, 30), R),
    ((205, 31), B),
    ((205, 40), B),
    ((205, 41), BG),
    ((244, 30), BG),
    // `none`: 80x40 at natural size, centred and cut at the content box.
    ((292, 30), BG),
    ((293, 30), R),
    ((312, 30), R),
    ((313, 30), G),
    ((300, 17), BG),
    // Rounded (radius 20px, border 4px, padding 6px: the content edge's
    // radius is 10px): the corners outside the border's curve are the page,
    // the content box's corner is the background, its inside the picture.
    ((10, 70), WH),
    ((11, 71), WH),
    ((20, 80), BG),
    ((21, 81), BG),
    ((25, 85), R),
    ((75, 135), WH),
    ((79, 139), BG),
    // Rounded and `cover`: 120x60, cut to the content edge's curve.
    ((200, 80), BG),
    ((205, 110), R),
    ((229, 110), R),
    ((230, 110), G),
    ((255, 110), G),
    // The percentage padding: border 300..301, padding 8px, content 309..325.
    // Its 1px border on all four sides (#2344: the software painter used to
    // blend the left edge over two pixels), and the page outside it: a solid
    // 1px border on whole pixels, as Chrome 156 draws one (the issue's table).
    ((299, 79), WH),
    ((300, 79), BD),
    ((333, 79), BD),
    ((334, 79), WH),
    ((316, 69), WH),
    ((316, 70), BD),
    ((316, 103), BD),
    ((316, 104), WH),
    ((308, 79), BG),
    ((309, 78), BG),
    ((309, 79), R),
    ((316, 79), R),
    ((317, 79), G),
    ((309, 87), B),
    ((324, 94), WH),
    ((325, 94), BG),
    ((324, 95), BG),
    // The rounded placeholder: nothing outside the curve, the page there.
    ((340, 100), WH),
    ((343, 103), WH),
    ((340, 149), WH),
    ((389, 100), WH),
];

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_an_img_is_drawn_in_its_content_box_as_chrome_draws_it() {
    let px = render();
    let at = |x: u32, y: u32| {
        let p = px[(y * W + x) as usize];
        [p[0], p[1], p[2]]
    };
    let bad: Vec<_> = CHROME
        .iter()
        .filter(|&&((x, y), want)| at(x, y).iter().zip(want).any(|(a, b)| a.abs_diff(b) > 2))
        .map(|&((x, y), want)| ((x, y), at(x, y), want))
        .collect();
    assert!(bad.is_empty(), "(point, vello, chrome): {bad:?}");
    // The placeholder's ring still runs along the top of the curve (Chrome
    // 156: (195, 183, 195) at (365, 100), silver anti-aliased over the
    // background), and the background is inside it.
    let ring = at(365, 100);
    assert!(
        ring.iter().all(|&c| c > 150),
        "the ring at the top of the rounded placeholder: {ring:?}"
    );
    assert_eq!(at(365, 101), BG, "inside the ring, the background");
}
