//! #1866 on the desktop GPU window: a `GameViewport` is the base layer.
//!
//! The window's present path, step by step, minus the window: the app's
//! scene ([`RinchApp::install_viewport_holes`] with the viewport's name, then
//! [`RinchApp::build_scene`]) rendered by Vello over the base colour
//! `WgpuRenderer` picks with a layer present (`ui_base_color`), the game's
//! layer blitted at [`RinchApp::viewport_rect`] over the window's white base,
//! and the UI blended on top (`LayerCompositor`). The pixel tests need an
//! adapter, so they are `#[ignore]`d:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_game_viewport_base_layer`.
//!
//! The warning tests only build the scene, so they run everywhere (debug
//! builds). The software painter's pixels for the same rule are
//! `rinch-dom/tests/game_viewport_base_layer_1866_tests.rs`, and the embed
//! host's `tests/embed_game_viewport_base_layer_1866.rs`.

use super::*;

const W: u32 = 400;
const H: u32 = 300;

/// The issue's cases on one page, inside a green ancestor that does not
/// clip: a red earlier sibling under the viewport's top-left, the viewport
/// (`border-radius: 40px`), a blue later sibling over its bottom edge, and a
/// yellow popup (`z-index: 5`, earlier in the DOM) over its top-right.
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

fn app_with(html: String) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_inner_html(&html);
        root
    });
    app.mount_component(W as f32, H as f32);
    app
}

/// A viewport under a clipping ancestor that paints a background (review of
/// #1883, D1): Vello renders a clip as a layer in every tile its outline
/// crosses, so an erase made only inside the clip left the ancestor's navy
/// in a band along the clip's edges.
struct ClipCase {
    label: &'static str,
    html: String,
    /// `scrollTop` for the `#sc` scroller, if any.
    scroll: Option<f64>,
    /// Where the game must show: a rect, rounded by `radius` (pixels whose
    /// centre is within 1.5px of the arc are the arc's anti-aliasing and are
    /// not read).
    game: (u32, u32, u32, u32),
    radius: f64,
    /// Points that must keep the UI's own colour.
    ui: Vec<((u32, u32), [u8; 4])>,
}

const GREEN: [u8; 4] = [0, 128, 0, 255];
const NAVY: [u8; 4] = [0, 0, 128, 255];

/// The navy panel is (20, 20)..(380, 280) on a green page in every case.
fn clip_cases(prefix: &str) -> Vec<(ClipCase, String)> {
    let root = "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
                background: rgb(0,128,0)";
    let panel = "position: absolute; left: 20px; top: 20px; width: 360px; height: 260px; \
                 background: rgb(0,0,128)";
    let name = |s: &str| format!("{prefix}-{s}");
    vec![
        (
            ClipCase {
                label: "square overflow: hidden, content overflowing",
                html: format!(
                    "<div style=\"{root}\"><div style=\"{panel}; overflow: hidden\">\
                     <div data-viewport=\"{}\" style=\"width: 360px; height: 260px\"></div>\
                     <div style=\"width: 900px; height: 10px\"></div></div></div>",
                    name("square")
                ),
                scroll: None,
                game: (20, 20, 380, 280),
                radius: 0.0,
                ui: vec![((10, 150), GREEN), ((390, 150), GREEN), ((200, 290), GREEN)],
            },
            name("square"),
        ),
        (
            ClipCase {
                label: "rounded overflow: hidden (24px)",
                html: format!(
                    "<div style=\"{root}\"><div style=\"{panel}; overflow: hidden; \
                     border-radius: 24px\">\
                     <div data-viewport=\"{}\" style=\"width: 360px; height: 260px\"></div>\
                     </div></div>",
                    name("rounded")
                ),
                scroll: None,
                game: (20, 20, 380, 280),
                radius: 24.0,
                ui: vec![((21, 21), GREEN), ((378, 278), GREEN), ((10, 150), GREEN)],
            },
            name("rounded"),
        ),
        (
            ClipCase {
                label: "scroller, viewport part-scrolled out",
                html: format!(
                    "<div style=\"{root}\"><div id=\"sc\" style=\"{panel}; overflow: auto\">\
                     <div data-viewport=\"{}\" style=\"width: 300px; height: 600px\"></div>\
                     </div></div>",
                    name("scrolled")
                ),
                scroll: Some(100.0),
                game: (20, 20, 320, 280),
                radius: 0.0,
                ui: vec![((340, 150), NAVY), ((10, 150), GREEN), ((200, 290), GREEN)],
            },
            name("scrolled"),
        ),
    ]
}

/// Whether pixel `(x, y)` is one to read for `case`: inside its game rect,
/// and, under a radius, at least 1.5px inside the rounded shape.
fn read_pixel(case: &ClipCase, x: u32, y: u32) -> bool {
    let (x0, y0, x1, y1) = case.game;
    if x < x0 || x >= x1 || y < y0 || y >= y1 {
        return false;
    }
    let r = case.radius;
    if r == 0.0 {
        return true;
    }
    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
    let cx = px.clamp(x0 as f64 + r, x1 as f64 - r);
    let cy = py.clamp(y0 as f64 + r, y1 as f64 - r);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() <= r - 1.5
}

/// Mount `case`, scrolled as it says.
fn mount_case(case: &ClipCase) -> RinchApp {
    let scroller: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let keep = scroller.clone();
    let html = case.html.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_inner_html(&html);
        *keep.borrow_mut() = root.query_selector("#sc");
        root
    });
    app.mount_component(W as f32, H as f32);
    if let Some(top) = case.scroll {
        let sc = scroller.borrow().clone().expect("the #sc scroller");
        sc.set_scroll_top(top);
        assert_eq!(sc.scroll_top(), top, "{}: positive control", case.label);
    }
    app
}

/// Build the scene the GPU window builds for a frame in which `name` has a
/// compositor layer.
fn build(app: &mut RinchApp, name: &str) {
    app.install_viewport_holes(Some([name.to_string()].into_iter().collect()));
    app.build_scene(1.0, (W, H));
    rinch_dom::paint::set_active_viewports(None);
}

#[cfg(debug_assertions)]
#[test]
fn an_opacity_ancestor_warns_on_the_desktop_scene_and_a_plain_one_does_not() {
    let _ = rinch_dom::paint::take_viewport_warnings();
    let mut plain = app_with(page("desk-warn-plain"));
    build(&mut plain, "desk-warn-plain");
    assert_eq!(
        rinch_dom::paint::take_viewport_warnings(),
        Vec::<String>::new()
    );

    let mut faded = app_with(format!(
        "<div class=\"hud\" style=\"opacity: 0.5\">{}</div>",
        page("desk-warn-faded")
    ));
    build(&mut faded, "desk-warn-faded");
    let warnings = rinch_dom::paint::take_viewport_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("\"desk-warn-faded\"") && warnings[0].contains("div.hud"),
        "{}",
        warnings[0]
    );
}

mod gpu {
    use super::*;
    use crate::shell::compositor::LayerCompositor;
    use wgpu::{Device, Queue, Texture, TextureFormat, TextureViewDescriptor};

    const MAGENTA: [u8; 4] = [255, 0, 255, 255];

    fn device() -> (Device, Queue) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("a GPU adapter (these fixtures are #[ignore]d for hosts without one)");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("a device")
    }

    fn texture(device: &Device, usage: wgpu::TextureUsages) -> Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game_viewport_base_layer_1866"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage,
            view_formats: &[],
        })
    }

    fn read(device: &Device, queue: &Queue, texture: &Texture) -> Vec<[u8; 4]> {
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

    /// The window's pixels: the UI over a magenta game layer at the
    /// viewport's rect, over the window's white base.
    fn composite(app: &mut RinchApp, name: &str) -> Vec<[u8; 4]> {
        let (device, queue) = device();
        build(app, name);
        let usage = wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let ui = texture(&device, usage);
        let mut renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("vello");
        let base = crate::shell::desktop::ui_base_color(vello::peniko::Color::WHITE, false, true);
        renderer
            .render_to_texture(
                &device,
                &queue,
                app.painter.scene(),
                &ui.create_view(&TextureViewDescriptor::default()),
                &vello::RenderParams {
                    base_color: base,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .expect("render");

        let layer = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let layer_view = layer.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &layer_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        let ((x, y, w, h), radius) = app.viewport_rect_with_radius(name).expect("the viewport");
        let compositor = LayerCompositor::new(&device, TextureFormat::Rgba8Unorm);
        let target = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = target.create_view(&TextureViewDescriptor::default());
        compositor.blit_layer_view(
            &device,
            &queue,
            &mut encoder,
            &layer_view,
            &view,
            (x, y, w, h),
            (W, H),
            Some(wgpu::Color::WHITE),
            radius,
            None,
        );
        compositor.overlay_ui(&device, &mut encoder, &ui, &view);
        queue.submit(Some(encoder.finish()));
        read(&device, &queue, &target)
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    /// Review of #1883, D1: under a clipping ancestor that paints a
    /// background — square, rounded, and a scroller with the viewport
    /// part-scrolled out — every pixel of the viewport's visible part is the
    /// game on the desktop window. An erase made only inside the clip left
    /// the navy in a band along the clip's edges (12,960 px in the square case).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_under_clipping_ancestors_on_the_desktop_window() {
        let mut failures = Vec::new();
        for (case, name) in clip_cases("desk-clip") {
            let mut app = mount_case(&case);
            let px = composite(&mut app, &name);
            let at = |x: u32, y: u32| px[(y * W + x) as usize];
            let mut wrong = 0;
            let mut first = None;
            for y in 0..H {
                for x in 0..W {
                    if read_pixel(&case, x, y) && !near(at(x, y), MAGENTA) {
                        wrong += 1;
                        first.get_or_insert(((x, y), at(x, y)));
                    }
                }
            }
            if wrong > 0 {
                failures.push(format!(
                    "{}: {wrong} px not the game, first {first:?}",
                    case.label
                ));
            }
            for &((x, y), want) in &case.ui {
                if !near(at(x, y), want) {
                    failures.push(format!(
                        "{}: ({x}, {y}) {:?}, want {want:?}",
                        case.label,
                        at(x, y)
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The issue's tests on the desktop GPU window, one page: the game shows
    /// through the non-clipping ancestor's background and the earlier
    /// sibling; the later sibling and the popup cover it; its own corners keep
    /// what is beneath them.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_on_the_desktop_window() {
        let name = "desk-base";
        let mut app = app_with(page(name));
        let px = composite(&mut app, name);
        let at = |x: u32, y: u32| px[(y * W + x) as usize];
        let want: &[((u32, u32), [u8; 4], &str)] = &[
            ((200, 150), MAGENTA, "the game, through the ancestor"),
            ((100, 100), MAGENTA, "the game, through the earlier sibling"),
            ((25, 25), [255, 0, 0, 255], "the earlier sibling beside it"),
            ((52, 52), [255, 0, 0, 255], "outside the top-left arc"),
            ((130, 215), [0, 0, 255, 255], "the later sibling over it"),
            ((270, 60), [255, 255, 0, 255], "the popup over it"),
            ((375, 150), [0, 128, 0, 255], "the ancestor beside it"),
            ((347, 247), [0, 128, 0, 255], "outside the bottom-right arc"),
        ];
        let bad: Vec<_> = want
            .iter()
            .filter(|&&((x, y), w, _)| !near(at(x, y), w))
            .map(|&((x, y), w, what)| (what, (x, y), at(x, y), w))
            .collect();
        assert!(bad.is_empty(), "(what, point, got, want): {bad:?}");
    }
}
