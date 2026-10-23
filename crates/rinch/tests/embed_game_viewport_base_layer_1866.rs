//! #1866 in an embedded context: a `GameViewport` is the base layer.
//!
//! The host composites its game under rinch's UI texture, so what this pins
//! is that texture, rendered by `RinchOverlayRenderer` from
//! `RinchContext::scene`: transparent inside the viewport — through a
//! non-clipping ancestor's background and an earlier sibling — except where a
//! later sibling or a popup covers it, and painted in the corners its own
//! `border-radius` leaves. The pixel test needs an adapter, so it is
//! `#[ignore]`d, and needs `gpu` for `pollster`:
//!     cargo test -p rinch --features gpu --test embed_game_viewport_base_layer_1866 -- --ignored
//!
//! The warning test builds the scene only, and runs everywhere (debug).
//! The desktop GPU window's version of the same page is
//! `src/app/game_viewport_base_layer_1866_tests.rs`.

#![cfg(any(feature = "gpu", feature = "embed"))]

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

const W: u32 = 400;
const H: u32 = 300;

/// The same page as the desktop fixture: a green non-clipping ancestor, a
/// red earlier sibling under the viewport's top-left, the viewport
/// (`border-radius: 40px`), a blue later sibling over its bottom edge, and a
/// yellow `z-index: 5` popup over its top-right.
fn page(name: &str) -> String {
    format!(
        "<div style=\"position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
         background: rgb(0,128,0)\">\
         <div style=\"position: absolute; z-index: 5; left: 250px; top: 40px; width: 40px; \
         height: 40px; background: rgb(255,255,0)\"></div>\
         <div style=\"position: absolute; left: 0; top: 0; width: 120px; height: 120px; \
         background: rgb(255,0,0)\"></div>\
         <div data-viewport=\"{name}\" style=\"position: absolute; left: 50px; top: 50px; \
         width: 300px; height: 200px; border-radius: 40px\"></div>\
         <div style=\"position: absolute; left: 100px; top: 200px; width: 60px; height: 30px; \
         background: rgb(0,0,255)\"></div>\
         </div>"
    )
}

fn context(html: String) -> RinchContext {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: W,
            height: H,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_inner_html(&html);
            root
        },
    );
    ctx.update(&[]);
    ctx
}

#[cfg(debug_assertions)]
#[test]
fn an_opacity_ancestor_warns_on_the_embed_scene_and_a_plain_one_does_not() {
    let _ = rinch_dom::paint::take_viewport_warnings();
    let mut plain = context(page("embed-warn-plain"));
    let _ = plain.scene();
    assert_eq!(
        rinch_dom::paint::take_viewport_warnings(),
        Vec::<String>::new()
    );

    let mut faded = context(format!(
        "<div id=\"hud\" style=\"opacity: 0.5\">{}</div>",
        page("embed-warn-faded")
    ));
    let _ = faded.scene();
    let warnings = rinch_dom::paint::take_viewport_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("\"embed-warn-faded\"") && warnings[0].contains("div#hud"),
        "{}",
        warnings[0]
    );
}

#[cfg(feature = "gpu")]
fn render(ctx: &mut RinchContext) -> Vec<[u8; 4]> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("a GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut overlay =
        rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let _view = overlay.render(&device, &queue, ctx.scene());

    let row = (W * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        overlay.texture().as_image_copy(),
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

#[cfg(feature = "gpu")]
/// The issue's tests on `RinchOverlayRenderer`'s texture, one page: clear
/// (for the host's game) through the non-clipping ancestor and the earlier
/// sibling, covered by the later sibling and the popup, painted in the
/// viewport's own rounded-off corners.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_game_viewport_base_layer_on_the_embed_overlay() {
    let mut ctx = context(page("embed-base"));
    let px = render(&mut ctx);
    let at = |x: u32, y: u32| px[(y * W + x) as usize];
    let clear = [0, 0, 0, 0];
    let want: &[((u32, u32), [u8; 4], &str)] = &[
        ((200, 150), clear, "clear through the ancestor"),
        ((100, 100), clear, "clear through the earlier sibling"),
        ((25, 25), [255, 0, 0, 255], "the earlier sibling beside it"),
        ((52, 52), [255, 0, 0, 255], "outside the top-left arc"),
        ((130, 215), [0, 0, 255, 255], "the later sibling over it"),
        ((270, 60), [255, 255, 0, 255], "the popup over it"),
        ((375, 150), [0, 128, 0, 255], "the ancestor beside it"),
        ((347, 247), [0, 128, 0, 255], "outside the bottom-right arc"),
    ];
    let near = |a: [u8; 4], b: [u8; 4]| {
        if b[3] == 0 {
            a[3] == 0
        } else {
            a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
        }
    };
    let bad: Vec<_> = want
        .iter()
        .filter(|&&((x, y), w, _)| !near(at(x, y), w))
        .map(|&((x, y), w, what)| (what, (x, y), at(x, y), w))
        .collect();
    assert!(bad.is_empty(), "(what, point, got, want): {bad:?}");
}
