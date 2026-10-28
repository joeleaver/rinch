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
