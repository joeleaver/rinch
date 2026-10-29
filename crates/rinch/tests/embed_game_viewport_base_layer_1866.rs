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

/// How the host gets rinch's texture.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq)]
enum Via {
    /// `RinchOverlayRenderer::render(device, queue, &mut ctx)`: the viewports
    /// cut out by the split compositor (#1866).
    Overlay,
    /// `ctx.scene()` rendered by the host's own Vello: the viewports erased
    /// in the scene, approximate inside clips.
    Scene,
}

#[cfg(feature = "gpu")]
fn render(ctx: &mut RinchContext) -> Vec<[u8; 4]> {
    render_via(ctx, Via::Overlay)
}

#[cfg(feature = "gpu")]
fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("a GPU adapter");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap()
}

#[cfg(feature = "gpu")]
fn render_via(ctx: &mut RinchContext, via: Via) -> Vec<[u8; 4]> {
    let (device, queue) = gpu();
    let mut overlay =
        rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    match via {
        Via::Overlay => {
            let _view = overlay.render(&device, &queue, ctx);
        }
        Via::Scene => {
            let mut vello = vello::Renderer::new(
                &device,
                vello::RendererOptions {
                    antialiasing_support: vello::AaSupport::all(),
                    use_cpu: false,
                    num_init_threads: None,
                    pipeline_cache: None,
                },
            )
            .expect("vello");
            vello
                .render_to_texture(
                    &device,
                    &queue,
                    ctx.scene(),
                    &overlay.texture().create_view(&Default::default()),
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width: W,
                        height: H,
                        antialiasing_method: vello::AaConfig::Msaa16,
                    },
                )
                .expect("render");
        }
    }
    read_back(&device, &queue, overlay.texture())
}

/// `texture`'s pixels, row-major.
#[cfg(feature = "gpu")]
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let row = (W * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
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

/// Review of #1883, D1, on the embed overlay: under a clipping ancestor that
/// paints a background — square (content overflowing, so the clip is kept),
/// rounded, and a scroller with the viewport part-scrolled out — every pixel
/// of the viewport's visible part is transparent in `RinchOverlayRenderer`'s
/// texture. Vello renders a clip as a layer in every tile its outline
/// crosses, and an erase made only inside the clip left the panel's navy in
/// a band along the clip's edges. The navy panel is (20, 20)..(380, 280) on
/// a green page in every case; the desktop window's version is
/// `app::game_viewport_base_layer_1866_tests`.
#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_game_viewport_base_layer_under_clipping_ancestors_on_the_embed_overlay() {
    use std::cell::RefCell;
    use std::rc::Rc;
    const GREEN: [u8; 4] = [0, 128, 0, 255];
    const NAVY: [u8; 4] = [0, 0, 128, 255];
    let root = "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
                background: rgb(0,128,0)";
    let panel = "position: absolute; left: 20px; top: 20px; width: 360px; height: 260px; \
                 background: rgb(0,0,128)";
    // (label, html, scrollTop of #sc, game rect, its radius, UI points)
    #[allow(clippy::type_complexity)]
    let cases: Vec<(
        &str,
        String,
        Option<f64>,
        (u32, u32, u32, u32),
        f64,
        Vec<((u32, u32), [u8; 4])>,
    )> = vec![
        (
            "square overflow: hidden, content overflowing",
            format!(
                "<div style=\"{root}\"><div style=\"{panel}; overflow: hidden\">\
                 <div data-viewport=\"embed-clip-square\" style=\"width: 360px; height: 260px\"></div>\
                 <div style=\"width: 900px; height: 10px\"></div></div></div>"
            ),
            None,
            (20, 20, 380, 280),
            0.0,
            vec![((10, 150), GREEN), ((390, 150), GREEN), ((200, 290), GREEN)],
        ),
        (
            "rounded overflow: hidden (24px)",
            format!(
                "<div style=\"{root}\"><div style=\"{panel}; overflow: hidden; border-radius: 24px\">\
                 <div data-viewport=\"embed-clip-rounded\" style=\"width: 360px; height: 260px\"></div>\
                 </div></div>"
            ),
            None,
            (20, 20, 380, 280),
            24.0,
            vec![((21, 21), GREEN), ((378, 278), GREEN), ((10, 150), GREEN)],
        ),
        (
            "scroller, viewport part-scrolled out",
            format!(
                "<div style=\"{root}\"><div id=\"sc\" style=\"{panel}; overflow: auto\">\
                 <div data-viewport=\"embed-clip-scrolled\" style=\"width: 300px; height: 600px\"></div>\
                 </div></div>"
            ),
            Some(100.0),
            (20, 20, 320, 280),
            0.0,
            vec![((340, 150), NAVY), ((10, 150), GREEN), ((200, 290), GREEN)],
        ),
    ];
    let mut failures = Vec::new();
    let mut bands = Vec::new();
    for (label, html, scroll, (x0, y0, x1, y1), r, ui) in cases {
        let scroller: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let keep = scroller.clone();
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
                *keep.borrow_mut() = root.query_selector("#sc");
                root
            },
        );
        ctx.update(&[]);
        if let Some(top) = scroll {
            let sc = scroller.borrow().clone().expect("the #sc scroller");
            sc.set_scroll_top(top);
            assert_eq!(sc.scroll_top(), top, "{label}: positive control");
            ctx.update(&[]);
        }
        let px = render(&mut ctx);
        let scene_px = render_via(&mut ctx, Via::Scene);
        let at = |x: u32, y: u32| px[(y * W + x) as usize];
        let read = |x: u32, y: u32| {
            if x < x0 || x >= x1 || y < y0 || y >= y1 {
                return false;
            }
            if r == 0.0 {
                return true;
            }
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let cx = px.clamp(x0 as f64 + r, x1 as f64 - r);
            let cy = py.clamp(y0 as f64 + r, y1 as f64 - r);
            ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() <= r - 1.5
        };
        let mut wrong = 0;
        let mut first = None;
        for y in 0..H {
            for x in 0..W {
                if read(x, y) && at(x, y)[3] != 0 {
                    wrong += 1;
                    first.get_or_insert(((x, y), at(x, y)));
                }
            }
        }
        if wrong > 0 {
            failures.push(format!("{label}: {wrong} px not clear, first {first:?}"));
        }
        // `ctx.scene()` erases the box in the scene, where Vello's clip
        // layers keep what the clip saved in the tiles its edge crosses:
        // the band the overlay's compositor removes.
        let band = (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| read(x, y) && scene_px[(y * W + x) as usize][3] != 0)
            .count();
        bands.push((label, band));
        for ((x, y), want) in ui {
            if at(x, y).iter().zip(want).any(|(a, b)| a.abs_diff(b) > 2) {
                failures.push(format!("{label}: ({x}, {y}) {:?}, want {want:?}", at(x, y)));
            }
        }
    }
    eprintln!("the band ctx.scene() leaves, by case: {bands:?}");
    assert!(
        bands.iter().all(|&(_, n)| n > 0),
        "positive control: ctx.scene() leaves a band under every clip: {bands:?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The halo page of the desktop fixture on the embed overlay: a rounded
/// clipping card holding a grey panel painted before the viewport and a
/// half-transparent red overlay after it, the viewport in the middle.
/// Through `RinchOverlayRenderer::render` the texture equals the same page
/// without a viewport everywhere outside the viewport's neighbourhood, bit
/// for bit: no halo along the card's ring, which closing, erasing and
/// reopening the clips put there (the second review of 461d5e9e). Inside,
/// the overlay's red at half alpha over transparent.
#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_game_viewport_base_layer_no_halo_on_the_embed_overlay() {
    let page = |viewport: bool| {
        let attr = if viewport {
            " data-viewport=\"embed-halo\""
        } else {
            ""
        };
        format!(
            "<div style=\"position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
             background: rgb(0,128,0)\">\
             <div style=\"position: absolute; left: 40px; top: 40px; width: 320px; \
             height: 220px; overflow: hidden; border-radius: 30px; \
             background: rgb(0,0,128)\">\
             <div style=\"position: absolute; left: 0; top: 0; width: 320px; height: 220px; \
             background: rgb(200,200,200)\"></div>\
             <div{attr} style=\"position: absolute; left: 110px; top: 70px; width: 100px; \
             height: 80px\"></div>\
             <div style=\"position: absolute; left: 0; top: 0; width: 320px; \
             height: 220px; background: rgba(255,0,0,0.5)\"></div>\
             </div></div>"
        )
    };
    let got = render(&mut context(page(true)));
    let want = render(&mut context(page(false)));
    // R, (150, 110)..(250, 190), grown by a Vello tile and a pixel (D10).
    let near_r = |x: u32, y: u32| (133..267).contains(&x) && (93..207).contains(&y);
    let differ: Vec<_> = (0..W * H)
        .map(|i| (i % W, i / W))
        .filter(|&(x, y)| !near_r(x, y) && got[(y * W + x) as usize] != want[(y * W + x) as usize])
        .collect();
    assert!(
        differ.is_empty(),
        "{} px outside the viewport differ from the page without it, first {:?}: {:?} vs {:?}",
        differ.len(),
        differ.first(),
        differ.first().map(|&(x, y)| got[(y * W + x) as usize]),
        differ.first().map(|&(x, y)| want[(y * W + x) as usize]),
    );
    let mid = got[(150 * W + 200) as usize];
    assert!(
        mid[0] >= 253 && mid[1] <= 2 && mid[2] <= 2 && mid[3].abs_diff(128) <= 1,
        "the viewport's middle: the overlay at half alpha over nothing, got {mid:?}"
    );
}

/// [`page`] with the later sibling (the HUD over the viewport's bottom edge)
/// in `hud`'s colour, and its handle.
#[cfg(feature = "gpu")]
fn hud_context(name: &str, hud: &str) -> (RinchContext, NodeHandle) {
    use std::cell::RefCell;
    use std::rc::Rc;
    let html = page(name).replace(
        "background: rgb(0,0,255)\"",
        &format!("background: {hud}\" id=\"hud\""),
    );
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let keep = handle.clone();
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
            *keep.borrow_mut() = root.query_selector("#hud");
            root
        },
    );
    ctx.update(&[]);
    let hud = handle.borrow().clone().expect("the #hud element");
    (ctx, hud)
}

/// What a texture of [`page`] shows: the HUD's colour where it covers the
/// viewport (130, 215), and whether the viewport is clear (200, 150).
#[cfg(feature = "gpu")]
fn hud_and_hole(px: &[[u8; 4]]) -> ([u8; 4], bool) {
    let at = |x: u32, y: u32| px[(y * W + x) as usize];
    (at(130, 215), at(200, 150)[3] == 0)
}

#[cfg(feature = "gpu")]
const BLUE: [u8; 4] = [0, 0, 255, 255];
#[cfg(feature = "gpu")]
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
#[cfg(feature = "gpu")]
const CYAN: [u8; 4] = [0, 255, 255, 255];

/// #1976: a host replaces its `RinchOverlayRenderer` on the `RinchContext`
/// it keeps — the step after a wgpu device loss, the renderer being
/// device-bound and the context not — and renders with nothing changed in
/// the document. The new renderer's first texture has the HUD painted over
/// the `GameViewport`: it used to have nothing painted after the viewport,
/// because the context's scene was not dirty, so it handed out no slices,
/// and the new renderer's `SplitFrame` started empty.
#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_new_overlay_renderer_on_an_existing_context_draws_the_hud_1976() {
    let (mut ctx, _hud) = hud_context("embed-1976-replace", "rgb(0,0,255)");
    let first = {
        let (device, queue) = gpu();
        let mut overlay =
            rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
        let _ = overlay.render(&device, &queue, &mut ctx);
        read_back(&device, &queue, overlay.texture())
        // The device and the renderer are dropped here: the device is lost.
    };
    assert_eq!(hud_and_hole(&first), (BLUE, true), "positive control");

    let (device, queue) = gpu();
    let mut overlay =
        rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let _ = overlay.render(&device, &queue, &mut ctx);
    let second = read_back(&device, &queue, overlay.texture());
    assert_eq!(
        hud_and_hole(&second),
        (BLUE, true),
        "the replacing renderer's first frame"
    );
    assert!(first == second, "the two renderers' textures differ");
}

/// #1976's general case: each renderer's slices are those of the frame the
/// context painted last *for it*. Two renderers taking turns on one context
/// (the second one renders the change, so the context is clean when the
/// first comes back), and one renderer taking turns on two contexts, each
/// draw what their context shows now.
#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_overlay_renderers_taking_turns_draw_their_contexts_current_frame_1976() {
    let (device, queue) = gpu();
    let new =
        || rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let frame = |overlay: &mut rinch::embed::RinchOverlayRenderer, ctx: &mut RinchContext| {
        let _ = overlay.render(&device, &queue, ctx);
        hud_and_hole(&read_back(&device, &queue, overlay.texture()))
    };

    // Two renderers, one context.
    let (mut ctx, hud) = hud_context("embed-1976-turns", "rgb(0,0,255)");
    let (mut a, mut b) = (new(), new());
    assert_eq!(frame(&mut a, &mut ctx), (BLUE, true), "a, first frame");
    assert_eq!(frame(&mut b, &mut ctx), (BLUE, true), "b, first frame");
    hud.set_style("background", "rgb(255,0,255)");
    ctx.update(&[]);
    assert_eq!(
        frame(&mut b, &mut ctx),
        (MAGENTA, true),
        "b, after the change (positive control)"
    );
    assert_eq!(
        frame(&mut a, &mut ctx),
        (MAGENTA, true),
        "a, after b painted the change"
    );

    // One renderer, two contexts.
    let (mut one, _) = hud_context("embed-1976-one", "rgb(0,0,255)");
    let (mut two, _) = hud_context("embed-1976-two", "rgb(0,255,255)");
    let mut r = new();
    assert_eq!(frame(&mut r, &mut one), (BLUE, true), "the first context");
    assert_eq!(frame(&mut r, &mut two), (CYAN, true), "the second context");
    assert_eq!(
        frame(&mut r, &mut one),
        (BLUE, true),
        "the first context again"
    );
}

/// REVIEW #1978: the steady state — one renderer, one context, nothing
/// changed — paints once and then serves the cached scene: the stamp check
/// asks for no repaint. Off the fixed point: several frames, then a change,
/// then several frames again (exactly one more paint).
#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_review_1978_steady_state_paints_no_extra_frame() {
    use rinch::perf::Counter;
    let (device, queue) = gpu();
    let (mut ctx, hud) = hud_context("embed-1978-steady", "rgb(0,0,255)");
    let mut r =
        rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    ctx.reset_perf();
    for _ in 0..4 {
        let _ = r.render(&device, &queue, &mut ctx);
    }
    let t = ctx.perf_total();
    assert_eq!(
        (
            t.get(Counter::PaintFrames),
            t.get(Counter::PaintCachedFrames)
        ),
        (1, 3),
        "four frames, nothing changed"
    );
    hud.set_style("background", "rgb(255,0,255)");
    ctx.update(&[]);
    for _ in 0..3 {
        let _ = r.render(&device, &queue, &mut ctx);
    }
    let t = ctx.perf_total();
    assert_eq!(
        (
            t.get(Counter::PaintFrames),
            t.get(Counter::PaintCachedFrames)
        ),
        (2, 5),
        "one change, three more frames"
    );
    assert_eq!(
        hud_and_hole(&read_back(&device, &queue, r.texture())),
        (MAGENTA, true)
    );
}

/// REVIEW #1978: a host that calls `ctx.scene()` between overlay renders
/// (an unsliced paint of the same context) still gets the HUD from the
/// overlay, which paints sliced again.
#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_review_1978_scene_between_overlay_renders() {
    let (device, queue) = gpu();
    let (mut ctx, _hud) = hud_context("embed-1978-mixed", "rgb(0,0,255)");
    let mut r =
        rinch::embed::RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let _ = r.render(&device, &queue, &mut ctx);
    let _ = ctx.scene();
    let _ = r.render(&device, &queue, &mut ctx);
    assert_eq!(
        hud_and_hole(&read_back(&device, &queue, r.texture())),
        (BLUE, true)
    );
}
