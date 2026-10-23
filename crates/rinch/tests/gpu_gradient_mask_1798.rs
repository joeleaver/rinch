//! #1798 on the GPU painter: the same markup and Chrome 155 pixels as
//! `rinch-dom/tests/gradient_mask_1798_tests.rs`, painted by `VelloPainter`
//! and rendered by Vello. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_gradient_mask_1798 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 660;
const H: u32 = 630;

const CASE_CSS: &str = "html, body { margin: 0; background: #fff; }
div { position: absolute; width: 320px; height: 200px; }
#a { left: 0; top: 0; background: repeating-linear-gradient(90deg, transparent 0 31px, #262a33 31px 32px); }
#b { left: 330px; top: 0; background: radial-gradient(120% 60% at 50% -8%, rgba(224,162,74,1), transparent 55%); }
#c { left: 0; top: 210px; background: red; mask-image: linear-gradient(180deg, #000, transparent 62%); }
#d { left: 330px; top: 210px; background: radial-gradient(120% 60% at 50% -8%, rgba(224,162,74,0.5), transparent 55%), repeating-linear-gradient(90deg, transparent 0 31px, #262a33 31px 32px); mask-image: linear-gradient(180deg, #000, transparent 62%); }
#e { left: 0; top: 420px; background: radial-gradient(circle 40px at 30px 50px, blue, transparent); }
#f { left: 330px; top: 420px; background: repeating-linear-gradient(45deg, red 0 10px, blue 10px 20px); }
#g { left: 200px; top: 560px; width: 100px; height: 50px; background: blue; mask-image: linear-gradient(90deg, #000, transparent); }
#g > div { position: absolute; left: 50px; top: 20px; width: 200px; height: 100px; background: red; }";

fn render() -> Vec<[u8; 4]> {
    let mut doc = RinchDocument::new();
    doc.load_css(CASE_CSS);
    let body = doc.body();
    for id in ["a", "b", "c", "d", "e", "f", "g"] {
        let n = doc.create_element("div");
        doc.set_attribute(n, "id", id);
        doc.append_child(body, n);
        if id == "g" {
            let child = doc.create_element("div");
            doc.append_child(n, child);
        }
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

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
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
    let data = buffer.slice(..).get_mapped_range();
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

/// Chrome 155's pixels for the same markup (on white), within 4 per channel.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_gradients_and_mask_match_chrome() {
    let px = render();
    if let Ok(path) = std::env::var("RINCH_1798_GPU_DUMP") {
        let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
        for p in &px {
            ppm.extend_from_slice(&p[..3]);
        }
        std::fs::write(path, ppm).unwrap();
    }
    let at = |x: u32, y: u32| px[(y * W + x) as usize];
    let want: &[((u32, u32), [u8; 3])] = &[
        ((30, 100), [255, 255, 255]),
        ((31, 100), [38, 42, 51]),
        ((32, 100), [255, 255, 255]),
        ((63, 100), [38, 42, 51]),
        ((490, 0), [231, 185, 119]),
        ((490, 30), [245, 227, 201]),
        ((600, 5), [243, 219, 186]),
        ((490, 100), [255, 255, 255]),
        ((100, 240), [255, 63, 63]),
        ((100, 280), [255, 145, 145]),
        ((100, 340), [255, 255, 255]),
        ((361, 215), [75, 69, 64]),
        ((361, 260), [127, 129, 134]),
        ((490, 212), [244, 222, 190]),
        ((30, 470), [5, 5, 255]),
        ((60, 470), [194, 194, 255]),
    ];
    let bad: Vec<_> = want
        .iter()
        .filter(|&&((x, y), w)| at(x, y)[..3].iter().zip(w).any(|(a, b)| a.abs_diff(b) > 4))
        .map(|&((x, y), w)| ((x, y), at(x, y), w))
        .collect();
    assert!(bad.is_empty(), "(point, vello, chrome): {bad:?}");
    // The masked box's positioned child is clipped to its border box: red
    // inside (faded by the mask), nothing outside.
    let inside = at(260, 595);
    assert!(
        inside[0] == 255 && (140..170).contains(&inside[1]),
        "inside, 40% red over white: {inside:?}"
    );
    assert_eq!(at(310, 595)[..3], [255, 255, 255], "outside the border box");
}
