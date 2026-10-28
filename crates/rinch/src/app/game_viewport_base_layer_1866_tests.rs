//! #1866 on the desktop GPU window: a `GameViewport` is the base layer.
//!
//! The window's present path, step by step, minus the window, as
//! `RinchRuntime::paint_gpu` drives it: the viewport handed to paint as a
//! base layer (`rinch_dom::paint::set_base_viewports`), the frame built with
//! [`RinchApp::build_gpu_frame_split`] and rendered by Vello into the UI
//! texture over the window's white base (in full, or partially as
//! `render_partial` does), the split change applied to the window's
//! `SplitFrame` and composited with the game's texture (`SplitCompositor`).
//! A frame whose base split is refused falls back to the layered path
//! (R1): the box is erased in the scene and the game blitted under the UI
//! (`LayerCompositor`), which [`Mode::Layered`] drives. The pixel tests need
//! an adapter, so they are `#[ignore]`d:
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

/// #1883 review, D1: the runtime's R1 sequence, through the
/// `BaseFallback::build` that `RinchRuntime::paint_gpu` builds every frame
/// with. A refused base split (the viewport in an `opacity: 0.5` group) falls
/// back, and the frame is built again **in full**: the first build consumed
/// its damage, and a second build left to it was `Unchanged`, so a UI change
/// landing on the frame the fallback retries the split was never rendered.
/// The window stays layered for exactly `BASE_FALLBACK_FRAMES` frames, and
/// the frame after them tries the split again. Every frame after the first is
/// driven as a transparent window's is (#1883 verification): its UI texture's
/// base is transparent with layers and without, so the renderer never says
/// the texture is stale (`target` `None`), and only `BaseFallback` can make
/// the retry frame full.
#[test]
fn a_refused_base_split_rebuilds_the_frame_in_full_and_retries_after_the_fallback() {
    use crate::shell::base_fallback::{BASE_FALLBACK_FRAMES, BaseFallback};
    use rinch_dom::paint::BaseViewport;
    use rinch_dom::perf::FullRepaintReason;
    let name = "r1-retry";
    let html = format!(
        "<div style=\"opacity: 0.5\">{}</div>\
         <div id=\"t\" style=\"position: absolute; left: 0; top: 280px; width: 20px; \
         height: 20px; background: rgb(0,0,255)\"></div>",
        page(name)
    );
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let keep = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_inner_html(&html);
        *keep.borrow_mut() = root.query_selector("#t");
        root
    });
    app.mount_component(W as f32, H as f32);
    let names: std::collections::HashSet<String> = [name.to_string()].into_iter().collect();
    let bases = || {
        Some(
            [(
                name.to_string(),
                BaseViewport {
                    surface_id: 4242,
                    extents: (W, H),
                    letterbox: false,
                },
            )]
            .into_iter()
            .collect(),
        )
    };
    let kind = |app: &mut RinchApp| match app.last_gpu_frame() {
        GpuFrame::Full(_) => "full",
        GpuFrame::Partial { .. } => "partial",
        GpuFrame::Unchanged => "unchanged",
    };
    let mut fallback = BaseFallback::default();
    // One frame as paint_gpu builds it: `retry` is whether it tried the
    // split; `target` the renderer's reason the UI texture is stale.
    let mut frame = |app: &mut RinchApp, target: Option<FullRepaintReason>| {
        let retry = fallback.try_split();
        let mut fell_back = false;
        app.install_viewport_holes(Some(names.clone()));
        let _ = fallback.build(
            app,
            1.0,
            (W, H),
            if retry { bases() } else { None },
            true,
            target,
            || fell_back = true,
        );
        rinch_dom::paint::set_active_viewports(None);
        (retry, fell_back, kind(app))
    };
    // The first frame tries, is refused (positive control), and is full.
    assert_eq!(
        frame(&mut app, Some(FullRepaintReason::Invalidated)),
        (true, true, "full")
    );
    // Layered for exactly BASE_FALLBACK_FRAMES frames, nothing to paint.
    for i in 0..BASE_FALLBACK_FRAMES {
        assert_eq!(
            frame(&mut app, None),
            (false, false, "unchanged"),
            "frame {i}"
        );
    }
    // A UI change lands on the retry frame. Its first build (the UI texture's
    // base becomes the window's: stale) consumes the damage and is refused;
    // the frame presented must still draw the change.
    handle.borrow().as_ref().expect("#t").set_attribute(
        "style",
        "position: absolute; left: 0; top: 280px; width: 20px; height: 20px; \
         background: rgb(255,0,0)",
    );
    app.resolve_and_repaint(W as f32, H as f32);
    assert_eq!(
        frame(&mut app, None),
        (true, true, "full"),
        "the retry frame's change must be rendered"
    );
    // And back to the fallback.
    assert_eq!(frame(&mut app, None), (false, false, "unchanged"));
}

/// #1883 verification, D2: on a transparent window the renderer never asks
/// for a full frame when the fallback ends (its UI base is transparent with
/// layers and without). The viewport's group effect goes away during the
/// fallback; the retry frame must paint the viewport and split it. Before,
/// it was a partial (or no) frame that did not reach the viewport: nothing
/// split, nothing refused, the layers dropped, and the viewport a hole with
/// no game until something repainted it.
#[test]
fn the_frame_that_ends_a_fallback_splits_the_viewport_on_a_transparent_window() {
    use crate::shell::base_fallback::{BASE_FALLBACK_FRAMES, BaseFallback};
    use rinch_dom::paint::BaseViewport;
    let name = "r1-transparent";
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let keep = handle.clone();
    let html = format!("<div id=\"g\" style=\"opacity: 0.5\">{}</div>", page(name));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_inner_html(&html);
        *keep.borrow_mut() = root.query_selector("#g");
        root
    });
    app.mount_component(W as f32, H as f32);
    let names: std::collections::HashSet<String> = [name.to_string()].into_iter().collect();
    let mut fallback = BaseFallback::default();
    let mut frame = |app: &mut RinchApp, target| {
        let retry = fallback.try_split();
        let mut fell_back = false;
        app.install_viewport_holes(Some(names.clone()));
        let bases = retry.then(|| {
            [(
                name.to_string(),
                BaseViewport {
                    surface_id: 4242,
                    extents: (W, H),
                    letterbox: false,
                },
            )]
            .into_iter()
            .collect()
        });
        let change = fallback.build(app, 1.0, (W, H), bases, true, target, || fell_back = true);
        rinch_dom::paint::set_active_viewports(None);
        let added: Vec<usize> = change
            .map(|c| c.add.iter().map(|s| s.surface_id).collect())
            .unwrap_or_default();
        (retry, fell_back, added)
    };
    let (_, fell_back, _) = frame(
        &mut app,
        Some(rinch_dom::perf::FullRepaintReason::Invalidated),
    );
    assert!(fell_back, "positive control: refused under opacity");
    for i in 0..BASE_FALLBACK_FRAMES {
        if i == 10 {
            handle
                .borrow()
                .as_ref()
                .expect("#g")
                .set_attribute("style", "opacity: 1");
            app.resolve_and_repaint(W as f32, H as f32);
        }
        let (retry, ..) = frame(&mut app, None);
        assert!(!retry, "frame {i} is in the fallback");
    }
    assert_eq!(
        frame(&mut app, None),
        (true, false, vec![4242]),
        "the retry frame splits the viewport"
    );
    // And keeps it: the next frame has nothing to paint, and splits nothing
    // new (the slice is kept).
    assert_eq!(frame(&mut app, None), (true, false, vec![]));
}

mod gpu {
    use super::*;
    use crate::shell::compositor::LayerCompositor;
    use crate::shell::desktop::{PartialFrame, render_partial};
    use crate::split_compositor::SplitCompositor;
    use crate::split_frame::SplitFrame;
    use rinch_dom::paint::BaseViewport;
    use wgpu::{Device, Queue, Texture, TextureFormat, TextureViewDescriptor};

    const MAGENTA: [u8; 4] = [255, 0, 255, 255];
    /// The id the game's slice is keyed by.
    const GAME: usize = 9001;

    /// How the window composites the viewport.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub(super) enum Mode {
        /// The split path: a base slice (#1866).
        Split,
        /// The fallback for a refused base split (R1): the box erased in the
        /// scene, the game blitted under the UI.
        Layered,
    }

    fn device() -> (Device, Queue) {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
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

    /// A desktop GPU window, as `paint_gpu` and `WgpuRenderer` drive one.
    pub(super) struct Window {
        device: Device,
        queue: Queue,
        vello: vello::Renderer,
        ui: Texture,
        partial: Option<Texture>,
        split: SplitFrame,
        compositor: SplitCompositor,
        layers: LayerCompositor,
        game: Texture,
        target: Texture,
        mode: Mode,
        /// The game's frame size, and whether it is letterboxed over black
        /// (a CPU frame's way) rather than stretched (a GPU texture's).
        extents: (u32, u32),
        letterbox: bool,
        /// The UI texture holds a frame (the next may be partial).
        drawn: bool,
        /// The base colour the UI texture was last rendered over
        /// ([`Self::runtime_frame`]).
        ui_base: Option<vello::peniko::Color>,
    }

    impl Window {
        pub(super) fn new(mode: Mode) -> Self {
            Self::with_game(mode, [1.0, 0.0, 1.0, 1.0], (W, H), false)
        }

        /// A window whose game is `extents` texels of the premultiplied
        /// colour `rgba`, `letterbox`ed or stretched.
        pub(super) fn with_game(
            mode: Mode,
            rgba: [f64; 4],
            extents: (u32, u32),
            letterbox: bool,
        ) -> Self {
            let (device, queue) = device();
            let vello = vello::Renderer::new(
                &device,
                vello::RendererOptions {
                    antialiasing_support: vello::AaSupport::all(),
                    use_cpu: false,
                    num_init_threads: None,
                    pipeline_cache: None,
                },
            )
            .expect("vello");
            let ui = texture(
                &device,
                wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
            );
            let game = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("game_viewport_base_layer_1866 game"),
                size: wgpu::Extent3d {
                    width: extents.0,
                    height: extents.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let target = texture(
                &device,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let view = game.create_view(&TextureViewDescriptor::default());
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: rgba[0],
                                g: rgba[1],
                                b: rgba[2],
                                a: rgba[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            queue.submit(Some(encoder.finish()));
            Self {
                compositor: SplitCompositor::new(&device, TextureFormat::Rgba8Unorm),
                layers: LayerCompositor::new(&device, TextureFormat::Rgba8Unorm),
                device,
                queue,
                vello,
                ui,
                partial: None,
                split: SplitFrame::default(),
                game,
                target,
                mode,
                extents,
                letterbox,
                drawn: false,
                ui_base: None,
            }
        }

        /// Paint, render and composite `app`'s next frame with the viewport
        /// `name` holding the magenta game (`name` empty: no viewport has a
        /// game). The first frame is full; later ones are what the damage
        /// makes them. The window's pixels.
        pub(super) fn frame(&mut self, app: &mut RinchApp, name: &str) -> Vec<[u8; 4]> {
            let split = self.mode == Mode::Split;
            let named = (!name.is_empty()).then(|| name.to_string());
            app.install_viewport_holes(Some(named.iter().cloned().collect()));
            if split && let Some(name) = &named {
                rinch_dom::paint::set_base_viewports(Some(
                    [(
                        name.clone(),
                        BaseViewport {
                            surface_id: GAME,
                            extents: self.extents,
                            letterbox: self.letterbox,
                        },
                    )]
                    .into_iter()
                    .collect(),
                ));
            }
            let _ = rinch_dom::paint::take_base_refused();
            app.set_split_enabled(split);
            // The base colour `WgpuRenderer` renders the UI over: the
            // window's, or transparent over layers.
            let base = crate::shell::desktop::ui_base_color(
                vello::peniko::Color::WHITE,
                false,
                !split && named.is_some(),
            );
            let target = (!self.drawn).then_some(rinch_dom::perf::FullRepaintReason::Invalidated);
            let (frame, change) = app.build_gpu_frame_split(1.0, (W, H), target);
            let ui_view = self.ui.create_view(&TextureViewDescriptor::default());
            match frame {
                GpuFrame::Full(scene) => self
                    .vello
                    .render_to_texture(
                        &self.device,
                        &self.queue,
                        scene,
                        &ui_view,
                        &vello::RenderParams {
                            base_color: base,
                            width: W,
                            height: H,
                            antialiasing_method: vello::AaConfig::Msaa16,
                        },
                    )
                    .expect("render"),
                GpuFrame::Partial { scene, size, rects } => render_partial(
                    &self.device,
                    &self.queue,
                    &mut self.vello,
                    &self.ui,
                    &mut self.partial,
                    PartialFrame {
                        scene,
                        size,
                        rects: &rects,
                        base,
                    },
                )
                .expect("partial"),
                GpuFrame::Unchanged => {}
            }
            self.drawn = true;
            rinch_dom::paint::set_base_viewports(None);
            rinch_dom::paint::set_active_viewports(None);
            assert!(
                !(split && rinch_dom::paint::take_base_refused()),
                "positive control: the base split was taken"
            );
            if let Some(change) = change {
                self.split
                    .apply(&self.device, &self.queue, &mut self.vello, change)
                    .expect("slices");
            }
            let game_view = self.game.create_view(&TextureViewDescriptor::default());
            self.split
                .set_base_views([(GAME, game_view.clone())].into_iter().collect());
            let view = self.target.create_view(&TextureViewDescriptor::default());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if split {
                assert_eq!(
                    self.split.surface_ids().count(),
                    usize::from(named.is_some()),
                    "positive control: the window composites the viewport split"
                );
                self.split.composite(
                    &self.compositor,
                    &self.device,
                    &mut encoder,
                    &ui_view,
                    &view,
                    (W, H),
                    &Default::default(),
                );
            } else {
                let mut clear = Some(wgpu::Color::WHITE);
                if let Some(name) = &named {
                    let ((x, y, w, h), radius) =
                        app.viewport_rect_with_radius(name).expect("the viewport");
                    let clip = app.viewport_clip_rect(name);
                    self.layers.blit_layer_view(
                        &self.device,
                        &self.queue,
                        &mut encoder,
                        &game_view,
                        &view,
                        (x, y, w, h),
                        (W, H),
                        clear.take(),
                        radius,
                        clip,
                    );
                }
                if clear.is_some() {
                    // No game: the UI alone, as the window presents it.
                    self.split.composite(
                        &self.compositor,
                        &self.device,
                        &mut encoder,
                        &ui_view,
                        &view,
                        (W, H),
                        &Default::default(),
                    );
                } else {
                    self.layers
                        .overlay_ui(&self.device, &mut encoder, &self.ui, &view);
                }
            }
            self.queue.submit(Some(encoder.finish()));
            read(&self.device, &self.queue, &self.target)
        }
    }

    impl Window {
        /// One frame as `RinchRuntime::paint_gpu` makes it, through
        /// `BaseFallback` (the mode is the fallback's to pick, not
        /// `self.mode`): the viewport `name` named as a base while the
        /// fallback allows, the layers otherwise (or after a refusal), the
        /// renderer's stale-texture reason worked out as `ui_invalid_reason`
        /// does for a window that is `transparent` or not, the split slices
        /// dropped while layered, and the composite the window presents.
        pub(super) fn runtime_frame(
            &mut self,
            app: &mut RinchApp,
            name: &str,
            fallback: &mut crate::shell::base_fallback::BaseFallback,
            transparent: bool,
        ) -> Vec<[u8; 4]> {
            let window_base = if transparent {
                vello::peniko::Color::TRANSPARENT
            } else {
                vello::peniko::Color::WHITE
            };
            let try_base = fallback.try_split();
            app.install_viewport_holes(Some([name.to_string()].into_iter().collect()));
            let bases = try_base.then(|| {
                [(
                    name.to_string(),
                    BaseViewport {
                        surface_id: GAME,
                        extents: self.extents,
                        letterbox: self.letterbox,
                    },
                )]
                .into_iter()
                .collect()
            });
            let mut layered = !try_base;
            let base = crate::shell::desktop::ui_base_color(window_base, false, layered);
            let target = match self.ui_base {
                Some(b) if b == base => None,
                _ => Some(rinch_dom::perf::FullRepaintReason::Invalidated),
            };
            let mut fell_back = false;
            let change = fallback.build(app, 1.0, (W, H), bases, !layered, target, || {
                fell_back = true
            });
            rinch_dom::paint::set_active_viewports(None);
            layered |= fell_back;
            let base = crate::shell::desktop::ui_base_color(window_base, false, layered);
            let ui_view = self.ui.create_view(&TextureViewDescriptor::default());
            match app.last_gpu_frame() {
                GpuFrame::Full(scene) => self
                    .vello
                    .render_to_texture(
                        &self.device,
                        &self.queue,
                        scene,
                        &ui_view,
                        &vello::RenderParams {
                            base_color: base,
                            width: W,
                            height: H,
                            antialiasing_method: vello::AaConfig::Msaa16,
                        },
                    )
                    .expect("render"),
                GpuFrame::Partial { scene, size, rects } => render_partial(
                    &self.device,
                    &self.queue,
                    &mut self.vello,
                    &self.ui,
                    &mut self.partial,
                    PartialFrame {
                        scene,
                        size,
                        rects: &rects,
                        base,
                    },
                )
                .expect("partial"),
                GpuFrame::Unchanged => {}
            }
            self.ui_base = Some(base);
            if let Some(change) = change {
                self.split
                    .apply(&self.device, &self.queue, &mut self.vello, change)
                    .expect("slices");
            }
            if layered {
                self.split.clear();
            }
            let game_view = self.game.create_view(&TextureViewDescriptor::default());
            self.split
                .set_base_views([(GAME, game_view.clone())].into_iter().collect());
            let view = self.target.create_view(&TextureViewDescriptor::default());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if layered {
                let ((x, y, w, h), radius) =
                    app.viewport_rect_with_radius(name).expect("the viewport");
                let c = window_base.components;
                self.layers.blit_layer_view(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &game_view,
                    &view,
                    (x, y, w, h),
                    (W, H),
                    Some(wgpu::Color {
                        r: c[0] as f64,
                        g: c[1] as f64,
                        b: c[2] as f64,
                        a: c[3] as f64,
                    }),
                    radius,
                    app.viewport_clip_rect(name),
                );
                self.layers
                    .overlay_ui(&self.device, &mut encoder, &self.ui, &view);
            } else {
                // The split path, or no layer and nothing split: the UI.
                self.split.composite(
                    &self.compositor,
                    &self.device,
                    &mut encoder,
                    &ui_view,
                    &view,
                    (W, H),
                    &Default::default(),
                );
            }
            self.queue.submit(Some(encoder.finish()));
            read(&self.device, &self.queue, &self.target)
        }
    }

    /// #1883 verification, D2, end to end on a **transparent** window: the
    /// viewport in an `opacity: 0.5` group is refused, the window falls back
    /// to the layered path, the group's opacity goes back to 1 during the
    /// fallback, and the frame that ends it splits the viewport: the game
    /// shows in its middle on that frame and the next. Before, that frame
    /// reached no viewport (the renderer asks no full frame on a transparent
    /// window), the layers were dropped and nothing was split: a hole.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_survives_the_end_of_a_fallback_on_a_transparent_window() {
        use crate::shell::base_fallback::{BASE_FALLBACK_FRAMES, BaseFallback};
        let name = "desk-transparent-retry";
        let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let keep = handle.clone();
        let html = format!("<div id=\"g\" style=\"opacity: 0.5\">{}</div>", page(name));
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_inner_html(&html);
            *keep.borrow_mut() = root.query_selector("#g");
            root
        });
        app.mount_component(W as f32, H as f32);
        let mut window = Window::new(Mode::Split);
        let mut fallback = BaseFallback::default();
        let mid = |px: &[[u8; 4]]| px[(150 * W + 200) as usize];
        let first = window.runtime_frame(&mut app, name, &mut fallback, true);
        // Positive control: layered, the game under the half-faded UI.
        assert!(
            mid(&first)[0] > 200 && mid(&first)[2] > 200,
            "{:?}",
            mid(&first)
        );
        for i in 0..BASE_FALLBACK_FRAMES {
            if i == 10 {
                handle
                    .borrow()
                    .as_ref()
                    .expect("#g")
                    .set_attribute("style", "opacity: 1");
                app.resolve_and_repaint(W as f32, H as f32);
            }
            let _ = window.runtime_frame(&mut app, name, &mut fallback, true);
        }
        for which in ["the retry frame", "the frame after it"] {
            let px = window.runtime_frame(&mut app, name, &mut fallback, true);
            assert!(
                near(mid(&px), MAGENTA),
                "{which}: the viewport's middle is {:?}, not the game",
                mid(&px)
            );
        }
    }

    /// #1883 verification, M5: a CPU `GameViewport` frame is uploaded only
    /// when it is new. `collect_surface_frames_shared` marks a submitted
    /// frame new once; the split path uploads it then, not on a frame that
    /// collects it unchanged, again when the game submits a new one, and
    /// again after a fallback dropped the window's textures (unchanged, but
    /// the window holds none).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_uploads_a_cpu_frame_only_when_new() {
        use crate::render_surface as rs;
        use crate::shell::base_fallback::pending_base_frame;
        use rinch_dom::paint::slicing::{SliceCoverage, SurfaceSlice};
        let _presenting = rs::compositor_routing_tests::presenting_lock();
        crate::shell::renderer::set_gpu_presenting(true);
        let game = rs::create_render_surface_with_name("m5-game");
        let mut window = Window::new(Mode::Split);
        let slice = || SurfaceSlice {
            surface_id: game.id(),
            region: (0, 0, 4, 2),
            above_in: None,
            above_out: None,
            own: SliceCoverage::Rect([0, 0, 4, 2]),
            ancestors: SliceCoverage::Rect([0, 0, 4, 2]),
            device_from_image: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            extents: (4, 2),
            straight_alpha: false,
            tainted: false,
            ink: peniko::kurbo::Rect::new(0.0, 0.0, 4.0, 2.0),
            base: None,
        };
        let keep = |window: &mut Window| {
            window
                .split
                .apply(
                    &window.device,
                    &window.queue,
                    &mut window.vello,
                    crate::split_frame::SplitChange {
                        drop: Vec::new(),
                        all: true,
                        add: vec![slice()],
                    },
                )
                .expect("slices");
        };
        keep(&mut window);
        let upload = |window: &mut Window| {
            let pending: Vec<_> = rs::collect_surface_frames_shared()
                .into_iter()
                .filter(|f| f.0 == game.id())
                .map(|(id, _, px, w, h, fresh)| pending_base_frame(id, &px, (w, h), fresh))
                .collect();
            assert_eq!(pending.len(), 1, "positive control: the frame is collected");
            window
                .split
                .upload_split_frames(&window.device, &window.queue, pending)
        };
        let frame = [7u8; 4 * 4 * 2];
        game.writer().submit_frame(&frame, 4, 2);
        let new = upload(&mut window);
        let unchanged = upload(&mut window);
        game.writer().submit_frame(&frame, 4, 2);
        let new_again = upload(&mut window);
        // A fallback drops the window's split textures; the split comes back.
        window.split.clear();
        keep(&mut window);
        let after_fallback = upload(&mut window);
        crate::shell::renderer::set_gpu_presenting(false);
        rs::unregister_render_surface(game.id());
        assert_eq!(
            (new, unchanged, new_again, after_fallback),
            (1, 0, 1, 1),
            "uploads: (new, unchanged, new again, after a fallback)"
        );
    }

    /// The window's pixels for `app`'s first frame, on the split path.
    fn composite(app: &mut RinchApp, name: &str) -> Vec<[u8; 4]> {
        Window::new(Mode::Split).frame(app, name)
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

    /// The game *replaces* what is beneath it (the base rule), it is not
    /// drawn over it: a half-transparent game over the red earlier sibling
    /// reads as the game alone (premultiplied, alpha 128), with nothing of
    /// the red or the green under it. An opaque game cannot tell the two
    /// apart. And a letterboxed frame (a CPU frame, #354) has black bars
    /// inside the box, not what is beneath it.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_replaces_what_is_beneath() {
        let name = "desk-replace";
        let mut app = app_with(page(name));
        let half = [128.0 / 255.0, 0.0, 128.0 / 255.0, 128.0 / 255.0];
        let px = Window::with_game(Mode::Split, half, (W, H), false).frame(&mut app, name);
        let at = |x: u32, y: u32| px[(y * W + x) as usize];
        let mut bad = Vec::new();
        for (p, what) in [
            ((100, 100), "over the red sibling"),
            ((200, 150), "over the green"),
        ] {
            if !near(at(p.0, p.1), [128, 0, 128, 128]) {
                bad.push((what, p, at(p.0, p.1)));
            }
        }
        // A 300x100 frame letterboxed into the 300x200 box: bars of 50px
        // above and below it, black.
        let mut app = app_with(page("desk-letterbox"));
        let px = Window::with_game(Mode::Split, [1.0, 0.0, 1.0, 1.0], (300, 100), true)
            .frame(&mut app, "desk-letterbox");
        let at = |x: u32, y: u32| px[(y * W + x) as usize];
        for (p, want, what) in [
            (
                (100, 75),
                [0, 0, 0, 255],
                "the top bar, over the red sibling",
            ),
            ((200, 225), [0, 0, 0, 255], "the bottom bar"),
            ((200, 150), MAGENTA, "the frame"),
        ] {
            if !near(at(p.0, p.1), want) {
                bad.push((what, p, at(p.0, p.1)));
            }
        }
        // The same frame in a square box (no radius): the clear is a rect
        // there, and it is the box, not the image's rect (#1883 review, M2).
        let square = page("desk-letterbox-square").replace("; border-radius: 40px", "");
        assert!(
            !square.contains("border-radius"),
            "positive control: square"
        );
        let mut app = app_with(square);
        let px = Window::with_game(Mode::Split, [1.0, 0.0, 1.0, 1.0], (300, 100), true)
            .frame(&mut app, "desk-letterbox-square");
        let at = |x: u32, y: u32| px[(y * W + x) as usize];
        for (p, want, what) in [
            (
                (100, 75),
                [0, 0, 0, 255],
                "square: the top bar, over the red sibling",
            ),
            ((52, 52), [0, 0, 0, 255], "square: the top bar's corner"),
            ((200, 225), [0, 0, 0, 255], "square: the bottom bar"),
            ((200, 150), MAGENTA, "square: the frame"),
        ] {
            if !near(at(p.0, p.1), want) {
                bad.push((what, p, at(p.0, p.1)));
            }
        }
        assert!(bad.is_empty(), "(what, point, got): {bad:?}");
    }

    /// A rounded clipping card (navy, 30px radius) holding, inside its clip,
    /// a grey panel painted before the viewport and a half-transparent red
    /// overlay painted after it; the viewport sits in the middle, far from
    /// the card's edges. `viewport: false` makes the same box a plain `div`.
    fn halo_page(name: &str, viewport: bool) -> String {
        let attr = if viewport {
            format!(" data-viewport=\"{name}\"")
        } else {
            String::new()
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
             <div id=\"over\" style=\"position: absolute; left: 0; top: 0; width: 320px; \
             height: 220px; background: rgba(255,0,0,0.5)\"></div>\
             </div></div>"
        )
    }

    /// The viewport's region R, `(150, 110)..(250, 190)`, grown by one Vello
    /// tile and a pixel: D10's band, where slicing may round once.
    fn near_r(x: u32, y: u32) -> bool {
        (133..267).contains(&x) && (93..207).contains(&y)
    }

    /// The second review of 461d5e9e: closing the clips open around the
    /// viewport, erasing, and opening them again composited what the card's
    /// clip holds in two halves, so every pixel of its antialiased ring where
    /// both halves drew (the grey panel before, the red overlay after) came
    /// out up to 64/255 off — along the whole ring, far from the viewport.
    /// On both paths the window equals the same page with no viewport
    /// everywhere outside the viewport's neighbourhood (bit for bit), and the
    /// game shows inside it under the overlay.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_no_halo_along_closed_clip_edges() {
        let mut failures = Vec::new();
        for mode in [Mode::Split, Mode::Layered] {
            let name = format!("desk-halo-{mode:?}");
            let mut app = app_with(halo_page(&name, true));
            let got = Window::new(mode).frame(&mut app, &name);
            let mut plain = app_with(halo_page(&name, false));
            let want = Window::new(mode).frame(&mut plain, "");
            let mut wrong = 0;
            let mut worst = (0u8, (0, 0));
            for y in 0..H {
                for x in 0..W {
                    if near_r(x, y) {
                        continue;
                    }
                    let (g, w) = (got[(y * W + x) as usize], want[(y * W + x) as usize]);
                    let d = g.iter().zip(w).map(|(a, b)| a.abs_diff(b)).max().unwrap();
                    if d > 0 {
                        wrong += 1;
                        if d > worst.0 {
                            worst = (d, (x, y));
                        }
                    }
                }
            }
            if wrong > 0 {
                failures.push(format!(
                    "{mode:?}: {wrong} px outside the viewport differ from the page without \
                     it, worst {} at {:?}",
                    worst.0, worst.1
                ));
            }
            // Positive control: the game, under the half-red overlay.
            let mid = got[(150 * W + 200) as usize];
            if !near(mid, [255, 0, 128, 255]) {
                failures.push(format!("{mode:?}: the viewport's middle is {mid:?}"));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The second review of 461d5e9e: on the GPU a partial frame must give
    /// the pixels a full one gives. On the halo page a small box at the
    /// card's top-left corner, on its ring and far from the viewport, changes
    /// colour: the partial frame paints neither the viewport nor its clear,
    /// and its window equals a fresh window's full frame of the changed page,
    /// bit for bit, on the split path and on the layered fallback. (The
    /// closed-and-reopened clips put their halo in the full frame and not in
    /// a partial one that culls the viewport.)
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_partial_frame_equals_full() {
        let mut failures = Vec::new();
        for mode in [Mode::Split, Mode::Layered] {
            let name = format!("desk-partial-{mode:?}");
            let with_box = |html: String, color: &str| {
                let end = "</div></div>";
                assert!(html.ends_with(end));
                format!(
                    "{}<div id=\"chg\" style=\"position: absolute; left: 0; top: 0; \
                     width: 50px; height: 50px; background: {color}\"></div>{end}",
                    &html[..html.len() - end.len()]
                )
            };
            let html = with_box(halo_page(&name, true), "rgb(0,200,0)");
            let over: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
            let keep = over.clone();
            let mut app = RinchApp::new(move |scope: &mut RenderScope| {
                let root = scope.create_element("div");
                root.set_inner_html(&html);
                *keep.borrow_mut() = root.query_selector("#chg");
                root
            });
            app.mount_component(W as f32, H as f32);
            let mut window = Window::new(mode);
            let _ = window.frame(&mut app, &name);
            let changed = "position: absolute; left: 0; top: 0; width: 50px; height: 50px; \
                           background: rgb(200,200,0)";
            over.borrow()
                .clone()
                .expect("#chg")
                .set_attribute("style", changed);
            app.resolve_and_repaint(W as f32, H as f32);
            let _ = app.end_perf_frame();
            let got = window.frame(&mut app, &name);
            let stats = app.end_perf_frame().expect("mounted");
            // Positive control: the second frame was a partial one.
            if stats.get(rinch_dom::perf::Counter::RepaintPartial) != 1 {
                failures.push(format!(
                    "{mode:?}: the second frame was not partial: {stats:?}"
                ));
            }

            let fresh_html = with_box(halo_page(&name, true), "rgb(200,200,0)");
            let mut fresh = app_with(fresh_html);
            let want = Window::new(mode).frame(&mut fresh, &name);
            let wrong: Vec<_> = (0..W * H)
                .filter(|&i| got[i as usize] != want[i as usize])
                .map(|i| ((i % W, i / W), got[i as usize], want[i as usize]))
                .collect();
            if !wrong.is_empty() {
                failures.push(format!(
                    "{mode:?}: {} px of the partial frame differ from a full one, first {:?}",
                    wrong.len(),
                    wrong[0]
                ));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
