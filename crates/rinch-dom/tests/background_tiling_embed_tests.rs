//! #468, review round 2 of PR #1143 (N1): a build without the software
//! painter (`embed`) has no rasteriser for the pattern path, so it tiles a
//! gradient one fill per tile — and must never drop a layer, however many
//! tiles it has. A `url()` layer needs no raster and is one image brush
//! (#1158).
//! Round 2 dropped any layer past 4096 tiles: the 800x600 dot grid emitted
//! one draw (its underlay). Run with `--no-default-features`.
#![cfg(not(feature = "software-renderer"))]
use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

fn draws(style: &str) -> (usize, usize) {
    let (all, grads, _) = draws_with_images(style);
    (all, grads)
}

/// A 3x2 image of distinct, partly translucent (straight-alpha) pixels.
fn dots() -> rinch_dom::image_cache::DecodedImage {
    let data: Vec<u8> = (0..6u8)
        .flat_map(|i| [40 * i, 200 - 30 * i, 90, 255 - 20 * i])
        .collect();
    rinch_dom::image_cache::DecodedImage::new(data, 3, 2)
}

/// Every draw, the gradient draws, and the image draws.
fn draws_with_images(style: &str) -> (usize, usize, usize) {
    let mut doc = RinchDocument::new();
    doc.tree
        .image_cache
        .insert_decoded("test://dots.png".into(), dots());
    doc.load_css("html, body { margin: 0; }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position:absolute;left:0;top:0;{style}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(900.0, 700.0);
    let mut vp = VelloPainter::new();
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut vp,
        1.0,
        (900.0, 700.0),
        &mut doc.font_cx,
        &mut lcx,
    );
    let e = vp.scene().encoding();
    // DrawTag::RADIAL_GRADIENT / LINEAR_GRADIENT / SWEEP_GRADIENT (vello_encoding 0.7)
    let grads = e
        .draw_tags
        .iter()
        .filter(|t| t.0 == 0x29c || t.0 == 0x114 || t.0 == 0x254)
        .count();
    // DrawTag::IMAGE
    let images = e.draw_tags.iter().filter(|t| t.0 == 0x28C).count();
    (e.draw_tags.len(), grads, images)
}

#[test]
fn a_many_tile_layer_is_drawn_without_the_software_painter() {
    let (all, _) = draws(
        "width:800px;height:600px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px",
    );
    assert!(
        all >= 4800,
        "80x60 tiles, one draw each (plus the underlay): {all}"
    );
}

#[test]
fn embed_probe() {
    eprintln!(
        "dot grid 800x600 10px: {:?}",
        draws(
            "width:800px;height:600px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px"
        )
    );
    eprintln!(
        "dot grid 400x300 10px: {:?}",
        draws(
            "width:400px;height:300px;background-color:rgb(250,250,250);background-image:radial-gradient(rgb(0,0,0) 20%, transparent 20%);background-size:10px 10px"
        )
    );
    eprintln!(
        "stripes 300x16: {:?}",
        draws(
            "width:300px;height:16px;background-image:linear-gradient(45deg, rgba(255,255,255,.15) 25%, transparent 25%, transparent 50%, rgba(255,255,255,.15) 50%, rgba(255,255,255,.15) 75%, transparent 75%, transparent);background-size:16px 16px"
        )
    );
}

/// #1158: a `url()` layer needs no rasteriser. The decoded image is one
/// `Extend::Repeat` image brush, however many tiles it covers: 800x600 of a
/// 3x2 image is 80,000 tiles, which was 80,000 image draws.
#[test]
fn a_repeating_url_layer_is_one_image_draw_without_the_software_painter() {
    for style in [
        // Natural size, both axes repeat.
        "width:800px;height:600px;background-image:url(test://dots.png)",
        // Scaled tile, off the identity.
        "width:800px;height:600px;background-image:url(test://dots.png);background-size:7px 5px",
        // One axis repeats: clipped to the strip.
        "width:800px;height:600px;background-image:url(test://dots.png);background-repeat:repeat-x;background-position:0 13px",
        // Rounded box (a BezPath / RoundedRect shape).
        "width:800px;height:600px;border-radius:20px;background-image:url(test://dots.png);background-position:1.5px 2.5px",
    ] {
        let (all, _, images) = draws_with_images(style);
        assert_eq!(images, 1, "{style}: {all} draws, {images} images");
    }
}

/// Gradients still go tile by tile: the image arm must not take them.
#[test]
fn a_gradient_layer_is_still_tiled_without_the_software_painter() {
    let (all, grads, images) = draws_with_images(
        "width:100px;height:100px;background-image:radial-gradient(red 20%, transparent 20%);background-size:10px 10px",
    );
    assert_eq!(images, 0, "{all}");
    assert_eq!(grads, 100, "{all}");
}

/// #1158 on a real GPU: the one image brush draws what the tile-by-tile
/// path drew (one `draw_image` per tile, under a clip of the box), and, at
/// the image's natural size, what the desktop GPU path draws from its
/// premultiplied raster. `cargo test -p rinch-dom --no-default-features
/// --test background_tiling_embed_tests -- --ignored` on a host with a GPU.
mod gpu {
    use super::*;
    use peniko::kurbo::{Affine, Rect};
    use peniko::{Color, Fill};
    use rinch_dom::paint::painter::{PaintImage, PaintShape, Painter, RepeatTile};
    use vello::wgpu;

    const W: u32 = 96;
    const H: u32 = 64;

    fn render(scene: &vello::Scene) -> Vec<[u8; 4]> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("a GPU adapter (these fixtures are #[ignore]d for hosts without one)");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("a device");
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
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
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
        .expect("vello");
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .render_to_texture(
                &device,
                &queue,
                scene,
                &view,
                &vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("render");
        let row = 512u32;
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
                let r = &data[(y * row) as usize..];
                (0..W as usize)
                    .map(|x| [r[x * 4], r[x * 4 + 1], r[x * 4 + 2], r[x * 4 + 3]])
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The document's own scene: an 80x50 box at (5, 4) with the layer.
    fn document(layer: &str) -> VelloPainter {
        let mut doc = RinchDocument::new();
        doc.tree
            .image_cache
            .insert_decoded("test://dots.png".into(), dots());
        doc.load_css("html, body { margin: 0; }");
        let body = doc.body();
        let node = doc.create_element("div");
        doc.set_attribute(
            node,
            "style",
            &format!(
                "position:absolute;left:5px;top:4px;width:80px;height:50px;\
                 background-image:url(test://dots.png);{layer}"
            ),
        );
        doc.append_child(body, node);
        doc.resolve_layout(W as f32, H as f32);
        let mut vp = VelloPainter::new();
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut vp,
            1.0,
            (W as f32, H as f32),
            &mut doc.font_cx,
            &mut lcx,
        );
        vp
    }

    /// What the tile-by-tile path drew before #1158: the box clipped, one
    /// image draw per `(tw, th)` tile from `origin` (whole device pixels).
    fn tile_by_tile(origin: (f64, f64), (tw, th): (f64, f64)) -> VelloPainter {
        let img = dots();
        let boxr = Rect::new(5.0, 4.0, 85.0, 54.0);
        let mut vp = VelloPainter::new();
        vp.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(boxr));
        let i0 = ((boxr.x0 - origin.0) / tw).floor() as i64;
        let j0 = ((boxr.y0 - origin.1) / th).floor() as i64;
        for j in j0.. {
            let y = origin.1 + j as f64 * th;
            if y >= boxr.y1 {
                break;
            }
            for i in i0.. {
                let x = origin.0 + i as f64 * tw;
                if x >= boxr.x1 {
                    break;
                }
                vp.draw_image(
                    &PaintImage {
                        data: &img.data,
                        width: img.width,
                        height: img.height,
                        decoded: None,
                        opaque: false,
                    },
                    Affine::translate((x, y))
                        * Affine::scale_non_uniform(tw / img.width as f64, th / img.height as f64),
                );
            }
        }
        vp.pop_layer();
        vp
    }

    fn worst(a: &[[u8; 4]], b: &[[u8; 4]]) -> (i16, usize) {
        let mut out = (0, 0);
        for (i, (p, q)) in a.iter().zip(b).enumerate() {
            let d = (0..4)
                .map(|c| (p[c] as i16 - q[c] as i16).abs())
                .max()
                .unwrap();
            if d > out.0 {
                out = (d, i);
            }
        }
        out
    }

    fn painted(px: &[[u8; 4]]) -> usize {
        px.iter().filter(|p| p[3] != 0).count()
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_one_image_brush_draws_what_the_tiles_drew() {
        // (layer CSS, the tile-by-tile reference's origin and tile size).
        let cases = [
            ("", (5.0, 4.0), (3.0, 2.0)),
            // A fractional position: both round to a whole pixel (the
            // pattern by the painter's #1151 step, the tiles by `DeviceSnap`).
            ("background-position:1.5px 2.5px", (7.0, 7.0), (3.0, 2.0)),
            // Off the identity: a scaled tile, sampled bilinearly.
            ("background-size:7px 5px", (5.0, 4.0), (7.0, 5.0)),
        ];
        for (layer, origin, size) in cases {
            let new = render(document(layer).scene());
            let old = render(tile_by_tile(origin, size).scene());
            assert!(painted(&new) > 3000, "{layer}: positive control");
            let (d, at) = worst(&new, &old);
            eprintln!(
                "{layer:?}: worst {d} at ({}, {})",
                at % W as usize,
                at / W as usize
            );
            assert!(
                d <= 1,
                "{layer:?}: {d} at ({}, {}): {:?} vs {:?}",
                at % W as usize,
                at / W as usize,
                new[at],
                old[at]
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_natural_size_matches_the_premultiplied_raster_path() {
        let img = dots();
        let pixels = peniko::Blob::from(rinch_dom::image_cache::premultiply_rgba(&img.data));
        let mut vp = VelloPainter::new();
        vp.fill_repeating(
            Fill::NonZero,
            Affine::IDENTITY,
            &RepeatTile {
                pixels: &pixels,
                width: 3,
                height: 2,
                transform: Affine::translate((5.0, 4.0)),
                smooth: false,
                premultiplied: true,
            },
            &PaintShape::Rect(Rect::new(5.0, 4.0, 85.0, 54.0)),
        );
        let raster = render(vp.scene());
        let new = render(document("").scene());
        let (d, at) = worst(&new, &raster);
        eprintln!("natural vs raster: worst {d}");
        assert!(d <= 1, "{d} at {at}: {:?} vs {:?}", new[at], raster[at]);
    }
}
