//! #2456 on the GPU painter: the picture of
//! `rinch-dom/tests/image_minify_2456_tests.rs` at `width: 40%` (0.4x),
//! painted by `VelloPainter` and rendered by Vello: every 1px line keeps at
//! least the darkness of Chrome 157's faintest there, less 10 (Chrome's run
//! 112, 80, 48, 112, 112, 48, 80, 112, 80, 80). Vello samples an image
//! bilinearly with no mipmaps: at 0.6x every line still falls within a pixel
//! of a sample (its faintest was 85, so the software test's scale does not
//! tell the two apart), but at 0.4x the samples are 2.5 source px apart and
//! before #2456 a line between two of them was dropped (darkness 0). Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_image_minify_2456 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 300;
const H: u32 = 260;
const SCALE: f64 = 0.4;
/// Chrome 157's faintest line at 0.4x (48), less the software test's margin
/// of 10: the copy is the picture's half-size mipmap level, drawn down
/// bilinearly from there as Chrome draws it (#2534; #2456's quarter-octave
/// copy was faintest 41). Before #2456, 0.
const CHROME_MIN: u8 = 38;

fn picture() -> DecodedImage {
    let (w, h) = (300usize, 200usize);
    let mut px = vec![255u8; w * h * 4];
    let mut ink = |x: usize, y: usize| px[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(0);
    for k in 0..20 {
        for y in 10..90 {
            ink(10 + 7 * k, y);
        }
    }
    for k in 0..14 {
        for x in 10..150 {
            ink(x, 100 + 7 * k);
        }
    }
    DecodedImage::new(px, w as u32, h as u32)
}

fn render() -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } div { width: 300px; }
         img { display: block; width: 40%; }",
    );
    doc.tree
        .image_cache
        .insert_decoded("lines.png".to_string(), picture());
    let body = doc.body();
    let div = doc.create_element("div");
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", "lines.png");
    doc.append_child(div, img);
    doc.append_child(body, div);
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
fn every_thin_line_survives_a_0_4x_draw_on_the_gpu() {
    let pixels = render();
    let grey = |x: u32, y: u32| {
        let p = pixels[(y * W + x) as usize];
        ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
    };
    let covered = |s: u32| (SCALE * s as f64) as u32..=(SCALE * (s + 1) as f64) as u32;
    let v: Vec<u8> = (0..20)
        .map(|k| 255 - covered(10 + 7 * k).map(|x| grey(x, 20)).min().unwrap())
        .collect();
    let h: Vec<u8> = (0..14)
        .map(|k| 255 - covered(100 + 7 * k).map(|y| grey(32, y)).min().unwrap())
        .collect();
    let faintest = *v.iter().chain(&h).min().unwrap();
    assert!(
        faintest >= CHROME_MIN,
        "a line fainter than Chrome's faintest ({CHROME_MIN}): vertical {v:?}, horizontal {h:?}"
    );
}
