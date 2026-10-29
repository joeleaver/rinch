//! #1865 PR 3: the split path through the runtime's own steps, against the
//! in-scene render of the same document. Each frame is made as `paint_gpu`
//! makes it: the surface steps, the GPU frames registered, a surface the
//! window composites split not marked as damage, the frame built with
//! `build_gpu_frame_split`, the UI texture rendered (in full or partially),
//! the split change applied and the frame composited (`SplitFrame`, the
//! window's own). The in-scene reference is a second window that never
//! splits and renders every frame in full.
//!
//! **The rule (D10).** Bit-equal everywhere except, each where the frame's
//! own slices say so: inside a region, where an above slice or a mask is
//! fractional, one level; on a rounded ancestor's ring inside a region, D8's
//! halo (zero here: the clipping card paints no background, so what it holds
//! and what is behind it are the same page), plus one level; and, in the
//! cases that paint something translucent across a region's edge, one level
//! within a Vello tile (16 px) outside the region. `#[ignore]`d: needs an
//! adapter (`cargo test -p rinch --features gpu --lib -- --ignored gpu_split_runtime`).
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::compositor_routing_tests::{
    clear_to, paint_like_paint_gpu, presenting_lock, test_gpu, vello_renderer,
};
use super::*;
use crate::app::{GpuFrame, RinchApp};
use crate::shell::desktop::{PartialFrame, SurfaceImages, render_partial};
use crate::shell::split_compositor::SplitCompositor;
use crate::shell::split_frame::SplitFrame;
use rinch_core::Component;
use wgpu::{Texture, TextureFormat, TextureUsages};

const W: u32 = 400;
const H: u32 = 300;

fn texture(size: (u32, u32), usage: TextureUsages) -> Texture {
    texture_of(size, TextureFormat::Rgba8Unorm, usage)
}

fn texture_of(size: (u32, u32), format: TextureFormat, usage: TextureUsages) -> Texture {
    test_gpu().device.create_texture(&wgpu::TextureDescriptor {
        label: Some("split_runtime_1865_tests"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn read(t: &Texture) -> Vec<[u8; 4]> {
    let gpu = test_gpu();
    let (w, h) = (t.width(), t.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        t.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        t.size(),
    );
    gpu.queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("the buffer is mapped");
    (0..h)
        .flat_map(|y| {
            let data = &data;
            (0..w).map(move |x| {
                let i = (y * row + x * 4) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            })
        })
        .collect()
}

/// What a test may break in a split window, to show the oracle sees it.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// The frame's split change is not applied (stale slices).
    StaleSlices,
    /// The window presents the UI texture, not the composite.
    NoComposite,
}

/// A window as `paint_gpu` and `WgpuRenderer` drive one.
struct Window {
    vello: vello::Renderer,
    registry: SurfaceImages,
    ui: Texture,
    partial: Option<Texture>,
    split: SplitFrame,
    compositor: SplitCompositor,
    target: Texture,
    splits: bool,
    slice_renders: usize,
    /// The ring class's pixels so far (see `allowances`).
    ring: Vec<i32>,
    /// Device rects of frames drawn in the scene whose Vello image-atlas
    /// neighbours differ from the reference's (D10's note): one level.
    atlas: Vec<(u32, u32, u32, u32)>,
    /// The window's scale factor, and its size in device pixels.
    scale: f64,
    dev: (u32, u32),
    /// Presented to a surface of this format and alpha mode, through
    /// `present_step` as `WgpuRenderer::present` does, and read back from
    /// there; `None` reads the composited (or UI) texture.
    surface: Option<(TextureFormat, wgpu::CompositeAlphaMode)>,
    /// A transparent window: the UI renders over transparent.
    transparent: bool,
}

/// Every frame's `split_below_kept` so far (S4).
fn below_kept_total(app: &RinchApp) -> u64 {
    app.doc.as_ref().map_or(0, |d| {
        d.borrow()
            .tree
            .perf
            .total()
            .get(rinch_dom::perf::Counter::SplitBelowKept)
    })
}

/// A frame: what it presented, which surfaces it composited split, the
/// frame's kind (`"full"`, `"partial"`, `"unchanged"`), and how many video
/// frames it uploaded for the compositor.
struct Shown {
    pixels: Vec<[u8; 4]>,
    split_ids: Vec<usize>,
    kind: &'static str,
    uploads: u64,
    /// Kept regions the frame did not re-render below over (S4).
    below_kept: u64,
}

impl Window {
    fn new(splits: bool) -> Self {
        Self::with(splits, 1.0, None, false)
    }

    /// A window at `scale`, presented to `surface` (format, alpha mode) when
    /// given, transparent or not.
    fn with(
        splits: bool,
        scale: f64,
        surface: Option<(TextureFormat, wgpu::CompositeAlphaMode)>,
        transparent: bool,
    ) -> Self {
        let dev = (
            (W as f64 * scale).round() as u32,
            (H as f64 * scale).round() as u32,
        );
        let format = surface.map_or(TextureFormat::Rgba8Unorm, |s| s.0);
        Self {
            vello: vello_renderer(),
            registry: SurfaceImages::default(),
            ui: texture(
                dev,
                TextureUsages::STORAGE_BINDING
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC
                    | TextureUsages::COPY_DST,
            ),
            partial: None,
            split: SplitFrame::default(),
            compositor: SplitCompositor::new(&test_gpu().device, format),
            target: texture_of(
                dev,
                format,
                TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC
                    | TextureUsages::TEXTURE_BINDING,
            ),
            splits,
            slice_renders: 0,
            ring: Vec::new(),
            atlas: Vec::new(),
            scale,
            dev,
            surface,
            transparent,
        }
    }

    fn frame(&mut self, app: &mut RinchApp, full: bool, fault: Fault) -> Shown {
        let gpu = test_gpu();
        let kept_before = below_kept_total(app);
        paint_like_paint_gpu(app, self.scale as f32);
        let frames = collect_gpu_surface_frames();
        let split_ids: Vec<usize> = self.split.surface_ids().collect();
        let fresh: Vec<usize> = frames
            .iter()
            .filter(|(_, _, fresh)| *fresh)
            .map(|(id, ..)| *id)
            .collect();
        let images = self.registry.sync(
            &mut self.vello,
            frames.into_iter().map(|(id, texture, _)| (id, texture)),
        );
        let stale = self.split.stale_ids(&self.registry.textures());
        app.mark_fresh_surfaces(&fresh, &split_ids, &stale);
        rinch_dom::paint::set_surface_gpu_images(Some(images));
        let mut viewports = collect_viewport_frames_by_name();
        let uploads = crate::shell::split_frame::prepare_split_viewports(
            &mut viewports,
            &mut self.split,
            self.splits,
        );
        app.install_gpu_viewport_frames(viewports, Some(Default::default()));
        app.set_split_enabled(self.splits);
        let target = full.then_some(rinch_dom::perf::FullRepaintReason::FirstFrame);
        let (frame, change) = app.build_gpu_frame_split(self.scale, (W, H), target);
        // As `WgpuRenderer` renders the UI: over the window's base colour.
        let base = if self.transparent {
            vello::peniko::Color::TRANSPARENT
        } else {
            vello::peniko::Color::WHITE
        };
        let (dw, dh) = self.dev;
        let ui_view = self.ui.create_view(&Default::default());
        let params = |w, h| vello::RenderParams {
            base_color: base,
            width: w,
            height: h,
            antialiasing_method: vello::AaConfig::Msaa16,
        };
        let kind = match frame {
            GpuFrame::Full(scene) => {
                self.vello
                    .render_to_texture(&gpu.device, &gpu.queue, scene, &ui_view, &params(dw, dh))
                    .expect("render");
                "full"
            }
            GpuFrame::Partial { scene, size, rects } => {
                if self.splits || std::env::var("PROBE_ALL").is_ok() {
                    eprintln!("[probe]   partial rects {rects:?} (splits {})", self.splits);
                }
                render_partial(
                    &gpu.device,
                    &gpu.queue,
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
                .expect("partial");
                "partial"
            }
            GpuFrame::Unchanged => "unchanged",
        };
        RinchApp::clear_viewport_frames();
        rinch_dom::paint::set_surface_gpu_images(None);
        rinch_dom::paint::set_viewport_split_ids(None);
        if let Some(c) = &change {
            self.slice_renders += c.add.len();
        }
        if let Some(change) = change
            && fault != Fault::StaleSlices
        {
            self.split
                .apply(&gpu.device, &gpu.queue, &mut self.vello, change)
                .expect("apply");
        }
        let uploads = self
            .split
            .upload_split_frames(&gpu.device, &gpu.queue, uploads);
        let split_ids: Vec<usize> = self.split.surface_ids().collect();
        let layered = !(self.split.is_empty() || fault == Fault::NoComposite);
        if layered {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            self.split.composite(
                &self.compositor,
                &gpu.device,
                &mut encoder,
                &ui_view,
                &self.target.create_view(&Default::default()),
                self.dev,
                &self.registry.textures(),
            );
            gpu.queue.submit(Some(encoder.finish()));
        }
        let pixels = match self.surface {
            None if layered => read(&self.target),
            None => read(&self.ui),
            // `WgpuRenderer::present`'s last step, into a surface-format
            // texture standing in for the swapchain's.
            Some((format, alpha_mode)) => {
                use crate::shell::desktop::{PresentStep, present_step};
                let out = texture_of(
                    self.dev,
                    format,
                    TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::COPY_DST
                        | TextureUsages::COPY_SRC,
                );
                let (source, step) = if layered {
                    (&self.target, present_step(alpha_mode, true, true))
                } else {
                    (
                        &self.ui,
                        present_step(alpha_mode, false, format == TextureFormat::Rgba8Unorm),
                    )
                };
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                match step {
                    PresentStep::Copy => {
                        encoder.copy_texture_to_texture(
                            source.as_image_copy(),
                            out.as_image_copy(),
                            source.size(),
                        );
                    }
                    PresentStep::Pass(alpha) => {
                        crate::shell::compositor::LayerCompositor::new(&gpu.device, format)
                            .present(
                                &gpu.device,
                                &mut encoder,
                                source,
                                &out.create_view(&Default::default()),
                                alpha,
                            );
                    }
                }
                gpu.queue.submit(Some(encoder.finish()));
                let mut px = read(&out);
                if format == TextureFormat::Bgra8Unorm {
                    for p in &mut px {
                        p.swap(0, 2);
                    }
                }
                px
            }
        };
        Shown {
            pixels,
            split_ids,
            kind,
            uploads,
            below_kept: below_kept_total(app) - kept_before,
        }
    }

    /// The allowance at each pixel (D10, from this window's own slices). The
    /// ring class (one level within a tile of a region, outside it) stays on
    /// a pixel once granted: a below pixel painted while a region was near
    /// keeps its rounding until something repaints it, after the region has
    /// moved on.
    fn allowances(&mut self, tol: Tol) -> Vec<i32> {
        let edges = tol.edges;
        let (w, h) = self.dev;
        let mut allow = vec![0; (w * h) as usize];
        if self.ring.len() != allow.len() {
            self.ring = vec![0; allow.len()];
        }
        let fractional = |a: u8| a != 0 && a != 255;
        for ((rx, ry, rw, rh), textures) in self.split.slices() {
            let read: Vec<Option<Vec<[u8; 4]>>> = textures.iter().map(|t| t.map(read)).collect();
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) as usize;
                    let inside = x >= rx && x < rx + rw && y >= ry && y < ry + rh;
                    if inside {
                        let local = ((y - ry) * rw + (x - rx)) as usize;
                        if tol.resampled || read.iter().flatten().any(|px| fractional(px[local][3]))
                        {
                            allow[i] = allow[i].max(1);
                        }
                    } else if edges
                        && x + 16 >= rx
                        && x < rx + rw + 16
                        && y + 16 >= ry
                        && y < ry + rh + 16
                    {
                        self.ring[i] = 1;
                    }
                }
            }
        }
        for (a, r) in allow.iter_mut().zip(&self.ring) {
            *a = (*a).max(*r);
        }
        for &(ax, ay, aw, ah) in &self.atlas {
            for y in ay..(ay + ah).min(h) {
                for x in ax..(ax + aw).min(w) {
                    let i = (y * w + x) as usize;
                    allow[i] = allow[i].max(1);
                }
            }
        }
        allow
    }
}

/// Which of D10's classes a case reaches beyond the ones its slices show:
/// something translucent across a region's edge (one level within a tile
/// outside it), a resampled frame (one level inside the region, the
/// amendment).
#[derive(Clone, Copy)]
struct Tol {
    edges: bool,
    resampled: bool,
}

const EXACT: Tol = Tol {
    edges: false,
    resampled: false,
};
const EDGES: Tol = Tol {
    edges: true,
    resampled: false,
};
const RESAMPLED: Tol = Tol {
    edges: false,
    resampled: true,
};

/// A pixel past its allowance: `(x, y, split, in-scene, allowance)`.
type Violation = (u32, u32, [u8; 4], [u8; 4], i32);

/// The pixels past their allowance.
fn compare(got: &[[u8; 4]], want: &[[u8; 4]], allow: &[i32], width: u32) -> Vec<Violation> {
    let mut out = Vec::new();
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let off = (0..4)
            .map(|c| (g[c] as i32 - w[c] as i32).abs())
            .max()
            .unwrap();
        if off > allow[i] {
            out.push((i as u32 % width, i as u32 / width, *g, *w, allow[i]));
        }
    }
    out
}

/// A page: a slate body, and `parts` in order, each `(style, Some(surface))`
/// for a `RenderSurface` or `(style, None)` for a plain box. A part's style
/// may name a wrapper (`"wrap:…|…"`): the surface or box goes inside a
/// wrapper of that style. Each box's handle is kept, for later changes.
struct Page {
    app: RinchApp,
    boxes: Rc<RefCell<Vec<NodeHandle>>>,
    surfaces: Vec<RenderSurfaceHandle>,
    videos: Vec<Video>,
}

/// A part of a page: its style, and the colour a surface draws (`None`: a
/// plain box).
type Part = (&'static str, Option<Rc<Cell<[u8; 4]>>>);

fn page(spec: &[Part]) -> Page {
    // A `video:WxH|style` part is a video viewport: a video surface of its
    // own name, fed a `W`x`H` frame of the part's colour.
    let mut videos = Vec::new();
    let parts: Vec<(&'static str, Option<RenderSurfaceHandle>, Option<String>)> = spec
        .iter()
        .map(|(style, color)| match style.strip_prefix("video:") {
            Some(rest) => {
                let (size, style) = rest.split_once('|').unwrap();
                let (w, h) = size.split_once('x').unwrap();
                let size = (w.parse().unwrap(), h.parse().unwrap());
                let name = format!("split-video-{}", NEXT_VIDEO.fetch_add(1, Ordering::Relaxed));
                let video = create_video_surface(&name);
                let color = color.clone().unwrap();
                video
                    .writer()
                    .submit_frame(&video_frame(color.get(), size), size.0, size.1);
                videos.push(Video {
                    surface: video.clone(),
                    size,
                    color,
                });
                (style, Some(video), Some(name))
            }
            None => (*style, color.clone().map(surface), None),
        })
        .collect();
    let surfaces: Vec<RenderSurfaceHandle> =
        parts.iter().filter_map(|(_, s, _)| s.clone()).collect();
    let boxes: Rc<RefCell<Vec<NodeHandle>>> = Default::default();
    let kept = boxes.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
             background: rgb(60, 70, 90)",
        );
        for (style, surface, video) in &parts {
            let (wrap, style) = match style.strip_prefix("wrap:") {
                Some(rest) => {
                    let (w, s) = rest.split_once('|').unwrap();
                    (Some(w), s)
                }
                None => (None, *style),
            };
            let node = match (surface, video) {
                (_, Some(name)) => {
                    let n = scope.create_element("div");
                    n.set_attribute("data-viewport", name);
                    n.set_attribute("style", style);
                    n
                }
                (Some(s), None) => {
                    let n = RenderSurface {
                        surface: Some(s.clone()),
                    }
                    .render(scope, &[]);
                    for decl in style.split(';') {
                        if let Some((k, v)) = decl.split_once(':') {
                            n.set_style(k.trim(), v.trim());
                        }
                    }
                    n
                }
                (None, None) => {
                    let n = scope.create_element("div");
                    n.set_attribute("style", style);
                    n
                }
            };
            kept.borrow_mut().push(node.clone());
            match wrap {
                Some(w) => {
                    let wrapper = scope.create_element("div");
                    wrapper.set_attribute("style", w);
                    wrapper.append_child(&node);
                    root.append_child(&wrapper);
                }
                None => root.append_child(&node),
            }
        }
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    Page {
        app,
        boxes,
        surfaces,
        videos,
    }
}

static NEXT_VIDEO: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
use std::sync::atomic::Ordering;

/// A video viewport on a page: its surface, frame size and colour.
struct Video {
    surface: RenderSurfaceHandle,
    size: (u32, u32),
    color: Rc<Cell<[u8; 4]>>,
}

impl Video {
    /// Submit a new frame in the colour's current value.
    fn redraw(&self) {
        self.surface.writer().submit_frame(
            &video_frame(self.color.get(), self.size),
            self.size.0,
            self.size.1,
        );
    }
}

/// A `w`x`h` opaque frame around `color`, every texel different, so a
/// misplaced or misscaled sample shows.
fn video_frame([r, g, b, _]: [u8; 4], (w, h): (u32, u32)) -> Vec<u8> {
    (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| {
                [
                    r.wrapping_add((x * 7 + y * 3) as u8),
                    g.wrapping_add((x * 2 + y * 5) as u8),
                    b.wrapping_add((x + y * 11) as u8),
                    255,
                ]
            })
        })
        .collect()
}

/// A surface drawing a pattern of `color` (a frame where every texel
/// differs, so a misplaced sample shows).
fn surface(color: Rc<Cell<[u8; 4]>>) -> RenderSurfaceHandle {
    let s = create_render_surface();
    s.set_continuous(false);
    s.set_gpu_render_callback(move |f| clear_to(f, color.get()));
    s
}

const SURFACE: &str = "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                       box-sizing: border-box";
const OPAQUE_BAR: &str = "position: absolute; left: 30px; top: 100px; width: 200px; height: 14px; \
                          background: rgb(20, 20, 20)";
const TRANSLUCENT_BAR: &str = "position: absolute; left: 30px; top: 100px; width: 200px; \
                               height: 14px; background: rgba(0, 0, 0, 0.55)";
const BADGE: &str = "position: absolute; left: 60px; top: 48px; width: 20px; height: 12px; \
                     background: rgb(250, 250, 250)";
const SIDEBAR: &str = "position: absolute; left: 320px; top: 0; width: 80px; height: 300px; \
                       background: rgb(0, 0, 128)";

/// The window `check` builds: scale, surface (format, alpha mode),
/// transparent. Scale 1, no surface, opaque unless a test sets it.
type WindowConfig = (f64, Option<(TextureFormat, wgpu::CompositeAlphaMode)>, bool);

thread_local! {
    static WINDOW: Cell<WindowConfig> = const { Cell::new((1.0, None, false)) };
    /// CSS rects of frames `check_split` expects in the scene beside split
    /// ones, resampled: D10's atlas note, one level inside each.
    static ATLAS: RefCell<Vec<(f64, f64, f64, f64)>> = const { RefCell::new(Vec::new()) };
}

/// One page, split against in-scene, over the frames `steps` makes. `split`
/// says whether the surfaces must end up split (or must not).
fn check(
    name: &str,
    parts: Vec<Part>,
    tol: Tol,
    split: &[bool],
    steps: &[&dyn Fn(&Page)],
    kinds: &[&str],
    fault: Fault,
) -> Vec<String> {
    let want = |n: usize, surfaces: &[usize]| {
        if split[n.min(split.len() - 1)] {
            surfaces.to_vec()
        } else {
            Vec::new()
        }
    };
    check_split(name, parts, tol, &want, steps, kinds, fault)
}

/// [`check`], with the surfaces that must be split after frame `n` named by
/// `want(n, surfaces)` (`surfaces` in paint order). Every frame also holds
/// the window's split memory to its cap (D6), as allocated.
fn check_split(
    name: &str,
    parts: Vec<Part>,
    tol: Tol,
    want_split: &dyn Fn(usize, &[usize]) -> Vec<usize>,
    steps: &[&dyn Fn(&Page)],
    kinds: &[&str],
    fault: Fault,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut split_page = page(&parts);
    let mut scene_page = page(&parts);
    let surfaces: Vec<usize> = split_page.surfaces.iter().map(|s| s.id()).collect();
    let (scale, surface, transparent) = WINDOW.with(|w| w.get());
    let mut win = Window::with(true, scale, surface, transparent);
    win.atlas = ATLAS
        .with(|a| a.borrow().clone())
        .into_iter()
        .map(|(x, y, w, h): (f64, f64, f64, f64)| {
            let (x0, y0) = ((x * scale).floor(), (y * scale).floor());
            let (x1, y1) = (((x + w) * scale).ceil(), ((y + h) * scale).ceil());
            (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32)
        })
        .collect();
    let mut reference = Window::with(false, scale, surface, transparent);
    let noop = |_: &Page| {};
    let all: Vec<&dyn Fn(&Page)> = std::iter::once(&noop as &dyn Fn(&Page))
        .chain(steps.iter().copied())
        .collect();
    for (n, step) in all.into_iter().enumerate() {
        step(&split_page);
        step(&scene_page);
        let got = win.frame(&mut split_page.app, n == 0, fault);
        let want = reference.frame(&mut scene_page.app, true, Fault::None);
        let mut want_ids = want_split(n, &surfaces);
        want_ids.sort_unstable();
        if let Some(kind) = kinds.get(n).filter(|k| !k.is_empty())
            && got.kind != *kind
        {
            failures.push(format!(
                "{name}, frame {n}: a {} frame, want {kind}",
                got.kind
            ));
        }
        let mut got_ids = got.split_ids.clone();
        got_ids.sort_unstable();
        if got_ids != want_ids {
            failures.push(format!(
                "{name}, frame {n}: split {got_ids:?}, want {want_ids:?} (of {surfaces:?})"
            ));
        }
        let cap = crate::shell::split_frame::split_memory_cap(win.dev);
        // Uploaded frames are held but not counted (D6, second amendment).
        let held = win.split.resident_bytes() - win.split.upload_bytes();
        if held > cap {
            failures.push(format!(
                "{name}, frame {n}: split memory {held} bytes, past the cap of {cap}"
            ));
        }
        let wrong = compare(&got.pixels, &want.pixels, &win.allowances(tol), win.dev.0);
        if !wrong.is_empty() {
            failures.push(format!(
                "{name}, frame {n} ({}): {} pixels past their allowance (x, y, split, in-scene, allowance): {:?}",
                got.kind,
                wrong.len(),
                &wrong[..wrong.len().min(6)]
            ));
        }
    }
    for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
        unregister_render_surface(s.id());
    }
    failures
}

fn red() -> Rc<Cell<[u8; 4]>> {
    Rc::new(Cell::new([200, 30, 40, 255]))
}

fn run(cases: Vec<Vec<String>>) {
    let failures: Vec<String> = cases.into_iter().flatten().collect();
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Element behaviours, one full frame each.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_matches_in_scene() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let s = |st: &'static str| -> Part { (st, Some(red())) };
    run(vec![
        check(
            "plain, a sidebar after it",
            vec![s(SURFACE), (SIDEBAR, None)],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "border, padding, own radius",
            vec![s(
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                    box-sizing: border-box; border: 5px solid rgb(0, 128, 0); padding: 10px; \
                    border-radius: 20px; background: rgb(0, 0, 255)",
            )],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "in a whole-pixel scroller cutting it",
            vec![(
                "wrap:position: absolute; left: 30px; top: 60px; width: 100px; height: 40px; \
                 overflow: hidden|position: absolute; left: 20px; top: -20px; width: 120px; \
                 height: 80px",
                Some(red()),
            )],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "an opaque bar across it",
            vec![s(SURFACE), (OPAQUE_BAR, None)],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "a translucent bar across it",
            vec![s(SURFACE), (TRANSLUCENT_BAR, None)],
            EDGES,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "a rounded card (no background) around it",
            vec![(
                "wrap:position: absolute; left: 40px; top: 30px; width: 140px; height: 100px; \
                 overflow: hidden; border-radius: 14px|position: absolute; left: 10px; \
                 top: 10px; width: 120px; height: 80px",
                Some(red()),
            )],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "two surfaces apart",
            vec![
                s(SURFACE),
                s("position: absolute; left: 200px; top: 150px; width: 90px; height: 60px"),
            ],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "rotated: in the scene",
            vec![s(
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                    transform: rotate(7deg)",
            )],
            EXACT,
            &[false],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "in an opacity group: in the scene",
            vec![(
                "wrap:position: absolute; left: 0; top: 0; opacity: 0.6|position: absolute; \
                 left: 50px; top: 40px; width: 120px; height: 80px",
                Some(red()),
            )],
            EXACT,
            &[false],
            &[],
            &["full"],
            Fault::None,
        ),
    ]);
}

/// Frames after the first: what is over a surface changes (a partial frame
/// that repaints its region), only the surface draws (no repaint at all),
/// the surface leaves.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_frames_after_the_first() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let color = red();
    let recolor = |p: &Page| {
        p.boxes.borrow()[1].set_style("background", "rgb(240, 200, 0)");
    };
    let c2 = color.clone();
    let redraw = move |p: &Page| {
        c2.set([30, 160, 90, 255]);
        for s in &p.surfaces {
            s.request_frame();
        }
    };
    let hide = |p: &Page| {
        p.boxes.borrow()[0].set_style("display", "none");
    };
    let elsewhere = |p: &Page| {
        p.boxes.borrow()[2].set_style("background", "rgb(128, 0, 128)");
    };
    run(vec![
        check(
            "the bar over it recoloured, then only the surface redrawn",
            vec![
                (SURFACE, Some(color.clone())),
                (OPAQUE_BAR, None),
                (SIDEBAR, None),
                // A badge over the surface the bar's damage does not reach:
                // the repainted region must still hold it.
                (BADGE, None),
            ],
            EXACT,
            &[true],
            &[&recolor, &redraw, &elsewhere],
            // The bar over it: its region repainted; only the surface: no
            // repaint at all; the sidebar: a partial frame away from it.
            &["full", "partial", "unchanged", "partial"],
            Fault::None,
        ),
        check(
            "the surface hidden",
            vec![(SURFACE, Some(red())), (OPAQUE_BAR, None), (SIDEBAR, None)],
            EXACT,
            &[true, false],
            &[&hide],
            &["full", "partial"],
            Fault::None,
        ),
    ]);
}

/// The oracle sees a window that keeps stale slices, and one that presents
/// without compositing.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_controls() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let recolor = |p: &Page| {
        p.boxes.borrow()[1].set_style("background", "rgb(240, 200, 0)");
    };
    for (fault, what) in [
        (Fault::StaleSlices, "stale slices"),
        (Fault::NoComposite, "no composite"),
    ] {
        let failures = check(
            what,
            vec![(SURFACE, Some(red())), (OPAQUE_BAR, None)],
            EXACT,
            &[true],
            &[&recolor],
            &[],
            fault,
        );
        assert!(
            !failures.is_empty(),
            "control ({what}): no failure; the oracle cannot see it"
        );
    }
    crate::shell::renderer::set_gpu_presenting(false);
}

// ---------------------------------------------------------------------------
// Review probes (#1899 adversarial review). Pixels only: which surfaces end
// up split is reported, not asserted.
// ---------------------------------------------------------------------------

/// A step of a random sequence.
type Step = Box<dyn Fn(&Page)>;

/// `check` without the split assertions: every frame against in-scene.
fn probe(
    name: &str,
    parts: Vec<Part>,
    edges: bool,
    steps: &[&dyn Fn(&Page)],
    toggles: &[bool],
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut split_page = page(&parts);
    let mut scene_page = page(&parts);
    let mut win = Window::new(true);
    let mut reference = Window::new(false);
    let noop = |_: &Page| {};
    let all: Vec<&dyn Fn(&Page)> = std::iter::once(&noop as &dyn Fn(&Page))
        .chain(steps.iter().copied())
        .collect();
    for (n, step) in all.into_iter().enumerate() {
        step(&split_page);
        step(&scene_page);
        win.splits = toggles.get(n).copied().unwrap_or(true);
        let p0 = split_page.app.split_paint_passes();
        let got = win.frame(&mut split_page.app, n == 0, Fault::None);
        // One paint a frame, two when it tainted (painted again in the
        // scene): the fallback that paints again for a region the damage
        // missed is a safety net (#1899 second review).
        let passes = split_page.app.split_paint_passes() - p0;
        let tainted = split_page.app.split_tainted_this_frame();
        if passes > 1 + u32::from(tainted) {
            failures.push(format!("{name}, frame {n}: painted {passes} times"));
        }
        let want = reference.frame(&mut scene_page.app, true, Fault::None);
        let wrong = compare(
            &got.pixels,
            &want.pixels,
            &win.allowances(if edges { EDGES } else { EXACT }),
            win.dev.0,
        );
        eprintln!(
            "[probe] {name}, frame {n}: {} frame, split {:?}, {} wrong, slices so far {}",
            got.kind,
            got.split_ids,
            wrong.len(),
            win.slice_renders
        );
        if !wrong.is_empty() {
            failures.push(format!(
                "{name}, frame {n} ({}, split {:?}): {} pixels past their allowance: {:?}",
                got.kind,
                got.split_ids,
                wrong.len(),
                &wrong[..wrong.len().min(6)]
            ));
        }
    }
    for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
        unregister_render_surface(s.id());
    }
    failures
}

/// Damage that meets the surface's box (its padding) but not its region R:
/// the painter paints the surface node (its ink meets the damage) and splits
/// it again, with what is over R outside the damage culled.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_damage_on_padding_only() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let tick = |p: &Page| {
        p.boxes.borrow()[2].set_style("background", "rgb(255, 0, 255)");
    };
    let failures = probe(
        "padding tick",
        vec![
            (
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                 box-sizing: border-box; padding: 12px; background: rgb(0, 0, 255)",
                Some(red()),
            ),
            // A badge over the content box (inside R, outside the damage).
            (
                "position: absolute; left: 110px; top: 70px; width: 30px; height: 14px; \
                 background: rgb(250, 250, 250)",
                None,
            ),
            // A tick over the padding only.
            (
                "position: absolute; left: 52px; top: 42px; width: 6px; height: 6px; \
                 background: rgb(0, 255, 0)",
                None,
            ),
        ],
        false,
        &[&tick],
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A surface refused for overlapping another (in the scene) becomes
/// splittable when the other moves away, on a partial frame whose damage
/// meets only part of it.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_overlap_refusal_then_partial() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let away = |p: &Page| {
        p.boxes.borrow()[0].set_style("top", "200px");
        p.boxes.borrow()[0].set_style("left", "20px");
    };
    let failures = probe(
        "overlap then move",
        vec![
            (
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px",
                Some(red()),
            ),
            (
                "position: absolute; left: 140px; top: 60px; width: 120px; height: 80px",
                Some(Rc::new(Cell::new([30, 160, 90, 255]))),
            ),
            (
                "position: absolute; left: 230px; top: 100px; width: 20px; height: 12px; \
                 background: rgb(250, 250, 250)",
                None,
            ),
        ],
        false,
        &[&away],
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Splitting turned off (a GameViewport layer came) and back on, then a
/// partial frame meeting part of the surface.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_reenabled_then_partial() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let noop = |_: &Page| {};
    let tick = |p: &Page| {
        p.boxes.borrow()[2].set_style("background", "rgb(255, 0, 255)");
    };
    let failures = probe(
        "re-enabled",
        vec![
            (SURFACE, Some(red())),
            (BADGE, None),
            // A tick across the surface's bottom-right corner.
            (
                "position: absolute; left: 160px; top: 110px; width: 20px; height: 20px; \
                 background: rgb(0, 255, 0)",
                None,
            ),
        ],
        false,
        &[&noop, &noop, &tick],
        &[true, false, true, true],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A random sequence of page changes, every frame against in-scene.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_random_sequences() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let seeds: Vec<u64> = std::env::var("PROBE_SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 2, 3, 4]);
    let steps_n: usize = std::env::var("PROBE_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let mut failures = Vec::new();
    for seed in seeds {
        let rng = Rc::new(Cell::new(
            seed.wrapping_mul(6364136223846793005).wrapping_add(1),
        ));
        let next = {
            let rng = rng.clone();
            move |n: u32| -> u32 {
                let x = rng
                    .get()
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                rng.set(x);
                ((x >> 33) as u32) % n
            }
        };
        // Pre-draw the ops so both pages get the same ones.
        let color = red();
        let mut ops: Vec<Step> = Vec::new();
        let mut log = Vec::new();
        for _ in 0..steps_n {
            let kind = next(7);
            let b = 1 + next(4) as usize; // boxes 1..=4
            let x = next(360) as i32 - 20;
            let y = next(260) as i32 - 20;
            let c = [next(256) as u8, next(256) as u8, next(256) as u8];
            log.push(format!("{kind}:{b}:{x}:{y}"));
            let color = color.clone();
            ops.push(match kind {
                0 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[b]
                        .set_style("background", &format!("rgb({}, {}, {})", c[0], c[1], c[2]));
                }),
                1 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[b].set_style("left", &format!("{x}px"));
                    p.boxes.borrow()[b].set_style("top", &format!("{y}px"));
                }),
                2 => Box::new(move |p: &Page| {
                    color.set([c[0], c[1], c[2], 255]);
                    for s in &p.surfaces {
                        s.request_frame();
                    }
                }),
                3 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[0].set_style("left", &format!("{}px", x.max(0) / 2));
                    p.boxes.borrow()[0].set_style("top", &format!("{}px", y.max(0) / 2));
                }),
                4 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[0].set_style("padding", &format!("{}px", c[0] % 16));
                }),
                5 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[b]
                        .set_style("box-shadow", &format!("0 0 {}px rgb(0, 0, 0)", c[1] % 12));
                }),
                // The surface's own shadow: damage beside its box, not on R.
                _ => Box::new(move |p: &Page| {
                    p.boxes.borrow()[0].set_style(
                        "box-shadow",
                        &format!("0 0 {}px rgb({}, 0, 0)", c[1] % 12, c[2]),
                    );
                }),
            });
        }
        eprintln!("[probe] seed {seed}: {log:?}");
        let refs: Vec<&dyn Fn(&Page)> = ops.iter().map(|o| o.as_ref()).collect();
        failures.extend(probe(
            &format!("seed {seed}"),
            vec![
                (
                    "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                     box-sizing: border-box; background: rgb(0, 0, 255)",
                    Some(color.clone()),
                ),
                (BADGE, None),
                (OPAQUE_BAR, None),
                (SIDEBAR, None),
                (
                    "position: absolute; left: 180px; top: 60px; width: 24px; height: 24px; \
                     background: rgb(0, 255, 0)",
                    None,
                ),
            ],
            true,
            &refs,
            &[],
        ));
    }
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Two kept surfaces; damage meets the second's region only, and adding that
/// region merges into a rect that meets the first's, which was checked
/// before it (not added).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_merge_after_check() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let tick = |p: &Page| {
        p.boxes.borrow()[3].set_style("background", "rgb(255, 0, 255)");
    };
    let failures = probe(
        "merge after check",
        vec![
            (SURFACE, Some(red())),
            (
                "position: absolute; left: 200px; top: 40px; width: 60px; height: 100px",
                Some(Rc::new(Cell::new([30, 160, 90, 255]))),
            ),
            (BADGE, None),
            (
                "position: absolute; left: 100px; top: 125px; width: 105px; height: 5px; \
                 background: rgb(0, 255, 0)",
                None,
            ),
        ],
        false,
        &[&tick],
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The surface's own box-shadow: damage over the shadow, not the box.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_damage_on_own_shadow() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let tick = |p: &Page| {
        p.boxes.borrow()[2].set_style("background", "rgb(255, 0, 255)");
    };
    let failures = probe(
        "own shadow tick",
        vec![
            (
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                 box-shadow: 0 0 12px rgb(0, 0, 0)",
                Some(red()),
            ),
            (BADGE, None),
            // A tick in the shadow, outside the box.
            (
                "position: absolute; left: 174px; top: 60px; width: 6px; height: 6px; \
                 background: rgb(0, 255, 0)",
                None,
            ),
        ],
        true,
        &[&tick],
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Taint (D5a): a grayscale badge over the surface, then 130 frames of
/// changes elsewhere, then the badge recoloured.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_taint_sequence() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let gray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "grayscale(1)");
        p.boxes.borrow()[1].set_style("background", "rgb(250, 100, 100)");
    };
    let flip = Rc::new(Cell::new(0u32));
    let f2 = flip.clone();
    let elsewhere = move |p: &Page| {
        // Applied to both pages in turn: flip every other call.
        let n = f2.get();
        f2.set(n + 1);
        let on = (n / 2).is_multiple_of(2);
        let k = p.boxes.borrow().len();
        let _ = k;
        p.boxes.borrow()[2].set_style(
            "background",
            if on {
                "rgb(128, 0, 128)"
            } else {
                "rgb(0, 0, 128)"
            },
        );
    };
    let recolor = |p: &Page| {
        p.boxes.borrow()[1].set_style("background", "rgb(100, 250, 100)");
    };
    let ungray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "none");
    };
    let mut steps: Vec<&dyn Fn(&Page)> = vec![&gray];
    for _ in 0..130 {
        steps.push(&elsewhere);
    }
    steps.push(&recolor);
    steps.push(&ungray);
    let failures = probe(
        "taint",
        vec![(SURFACE, Some(red())), (BADGE, None), (SIDEBAR, None)],
        true,
        &steps,
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A continuous surface over an idle UI: how many slice renders and which
/// frame kinds.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_continuous_idle() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let color = red();
    let c2 = color.clone();
    let play = move |p: &Page| {
        let [r, g, b, a] = c2.get();
        c2.set([g, b, r, a]);
        for s in &p.surfaces {
            s.set_continuous(true);
        }
    };
    let steps: Vec<&dyn Fn(&Page)> = vec![&play; 6];
    let failures = probe(
        "continuous",
        vec![
            (SURFACE, Some(color.clone())),
            (BADGE, None),
            (SIDEBAR, None),
        ],
        false,
        &steps,
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The scratch target's cost: one slice (a rounded own mask and an above
/// scene) of a 64 px region at the window's top-left, at its bottom-right,
/// and a full-window region, in a 1920x1080 window, against the same scenes
/// rendered into a region-sized target.
#[test]
#[ignore = "a measurement; needs a GPU adapter"]
fn split_runtime_scratch_cost_probe() {
    use rinch_dom::paint::slicing::{SliceCoverage, SurfaceSlice};
    use vello::kurbo::{Affine, Rect, RoundedRect};
    use vello::peniko::{Color, Fill};
    let _p = presenting_lock();
    let gpu = test_gpu();
    let mut vello = vello_renderer();
    let scenes = |r: (u32, u32, u32, u32), at: Affine| {
        let (x, y, w, h) = (r.0 as f64, r.1 as f64, r.2 as f64, r.3 as f64);
        let mut mask = vello::Scene::new();
        mask.push_clip_layer(
            Fill::NonZero,
            at,
            &RoundedRect::new(x, y, x + w, y + h, 12.0),
        );
        mask.fill(
            Fill::NonZero,
            at,
            Color::WHITE,
            None,
            &Rect::new(x, y, x + w, y + h),
        );
        mask.pop_layer();
        let mut above = vello::Scene::new();
        above.fill(
            Fill::NonZero,
            at,
            Color::from_rgba8(0, 0, 0, 140),
            None,
            &Rect::new(x + 4.0, y + h - 20.0, x + w - 4.0, y + h - 4.0),
        );
        (mask, above)
    };
    let cases = [
        ("64px at top-left", (0, 0, 64, 64)),
        ("64px at bottom-right", (1856, 1016, 64, 64)),
        ("full window", (0, 0, 1920, 1080)),
    ];
    for (what, r) in cases {
        let mut frame = SplitFrame::default();
        let n = 6;
        let mut total = std::time::Duration::ZERO;
        for i in 0..n {
            let (mask, above) = scenes(r, Affine::IDENTITY);
            let change = crate::shell::split_frame::SplitChange {
                drop: vec![],
                all: true,
                add: vec![SurfaceSlice {
                    surface_id: 1,
                    region: r,
                    above_in: Some(above),
                    above_out: None,
                    own: SliceCoverage::Mask(Box::new(mask)),
                    ancestors: SliceCoverage::Rect([r.0, r.1, r.0 + r.2, r.1 + r.3]),
                    device_from_image: [1.0, 0.0, 0.0, 1.0, r.0 as f64, r.1 as f64],
                    extents: (r.2, r.3),
                    straight_alpha: false,
                    tainted: false,
                    ink: peniko::kurbo::Rect::new(
                        r.0 as f64,
                        r.1 as f64,
                        (r.0 + r.2) as f64,
                        (r.1 + r.3) as f64,
                    ),
                    base: None,
                }],
            };
            let t = std::time::Instant::now();
            frame
                .apply(&gpu.device, &gpu.queue, &mut vello, change)
                .unwrap();
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if i > 0 {
                total += t.elapsed();
            }
        }
        // The same two scenes into an R-sized target, translated.
        let target = texture((r.2, r.3), TextureUsages::STORAGE_BINDING);
        let view = target.create_view(&Default::default());
        let mut total_r = std::time::Duration::ZERO;
        for i in 0..n {
            let at = Affine::translate((-(r.0 as f64), -(r.1 as f64)));
            let (mask, above) = scenes(r, at);
            let t = std::time::Instant::now();
            for s in [&mask, &above] {
                vello
                    .render_to_texture(
                        &gpu.device,
                        &gpu.queue,
                        s,
                        &view,
                        &vello::RenderParams {
                            base_color: Color::TRANSPARENT,
                            width: r.2,
                            height: r.3,
                            antialiasing_method: vello::AaConfig::Msaa16,
                        },
                    )
                    .unwrap();
            }
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if i > 0 {
                total_r += t.elapsed();
            }
        }
        eprintln!(
            "[probe] scratch cost, {what}: apply (scratch) {:.2} ms, R-sized {:.2} ms",
            total.as_secs_f64() * 1000.0 / (n - 1) as f64,
            total_r.as_secs_f64() * 1000.0 / (n - 1) as f64
        );
    }
}

/// After a taint's suspension runs out, a surface left in the scene is split
/// again by a partial frame whose damage covers only part of its region.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_after_suspension() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let gray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "grayscale(1)");
    };
    let ungray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "none");
    };
    let count = Rc::new(Cell::new(0u32));
    let c2 = count.clone();
    let elsewhere = move |p: &Page| {
        let n = c2.get();
        c2.set(n + 1);
        p.boxes.borrow()[3].set_style(
            "background",
            if (n / 2).is_multiple_of(2) {
                "rgb(128, 0, 128)"
            } else {
                "rgb(0, 0, 128)"
            },
        );
    };
    let badge2 = |p: &Page| {
        p.boxes.borrow()[2].set_style("background", "rgb(255, 0, 255)");
    };
    let mut steps: Vec<&dyn Fn(&Page)> = vec![&gray, &ungray];
    for _ in 0..125 {
        steps.push(&elsewhere);
    }
    steps.push(&badge2);
    let failures = probe(
        "after suspension",
        vec![
            (SURFACE, Some(red())),
            (
                "position: absolute; left: 120px; top: 90px; width: 30px; height: 14px; \
                 background: rgb(250, 100, 100)",
                None,
            ),
            (BADGE, None),
            (SIDEBAR, None),
        ],
        true,
        &steps,
        &[],
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// An idle surface in a window that never splits, a change elsewhere: is the
/// surface's box damaged each frame?
#[test]
#[ignore = "a measurement; needs a GPU adapter"]
fn split_runtime_in_scene_idle_probe() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let count = Rc::new(Cell::new(0u32));
    let c2 = count.clone();
    let elsewhere = move |p: &Page| {
        let n = c2.get();
        c2.set(n + 1);
        p.boxes.borrow()[2].set_style(
            "background",
            if (n / 2).is_multiple_of(2) {
                "rgb(128, 0, 128)"
            } else {
                "rgb(0, 0, 128)"
            },
        );
    };
    let gray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "grayscale(1)");
    };
    let ungray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "none");
    };
    let steps: Vec<&dyn Fn(&Page)> = if std::env::var("PROBE_NOGRAY").is_ok() {
        vec![
            &elsewhere, &elsewhere, &elsewhere, &elsewhere, &elsewhere, &elsewhere,
        ]
    } else {
        vec![
            &elsewhere, &elsewhere, &gray, &ungray, &elsewhere, &elsewhere,
        ]
    };
    let failures = probe(
        "in-scene idle",
        vec![
            (SURFACE, Some(red())),
            (
                "position: absolute; left: 60px; top: 48px; width: 20px; height: 12px; \
                 background: rgb(250, 100, 100)",
                None,
            ),
            (SIDEBAR, None),
        ],
        true,
        &steps,
        &if std::env::var("PROBE_LATE").is_ok() {
            vec![false, false, false, false, true, true, true]
        } else {
            vec![false, false, false, true, true, true, true]
        },
    );
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Through `WgpuRenderer::present`'s last step (`present_step`), into a
/// `Bgra8Unorm` surface: the composited texture in the surface's format, the
/// alpha path of an opaque window and of a transparent one (premultiplied
/// and post-multiplied surfaces), and a 1.5 scale factor (#1899 review).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_presented_to_a_surface() {
    use wgpu::CompositeAlphaMode as A;
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let bgra = TextureFormat::Bgra8Unorm;
    let mut failures = Vec::new();
    for (config, what) in [
        ((1.0, Some((bgra, A::Opaque)), false), "opaque, Bgra8"),
        (
            (1.5, Some((bgra, A::Opaque)), false),
            "opaque, Bgra8, scale 1.5",
        ),
        (
            (1.0, Some((bgra, A::PreMultiplied)), true),
            "transparent, premultiplied surface",
        ),
        (
            (1.0, Some((bgra, A::PostMultiplied)), true),
            "transparent, post-multiplied surface",
        ),
        (
            (1.5, Some((TextureFormat::Rgba8Unorm, A::Opaque)), false),
            "opaque, Rgba8, scale 1.5",
        ),
    ] {
        WINDOW.with(|w| w.set(config));
        for mut f in [
            check(
                "a surface, a sidebar",
                vec![(SURFACE, Some(red())), (SIDEBAR, None)],
                EXACT,
                &[true],
                &[],
                &["full"],
                Fault::None,
            ),
            check(
                "own radius, an opaque bar across it",
                vec![
                    (
                        "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                         border-radius: 14px",
                        Some(red()),
                    ),
                    (OPAQUE_BAR, None),
                ],
                EXACT,
                &[true],
                &[],
                &["full"],
                Fault::None,
            ),
        ] {
            for line in &mut f {
                *line = format!("{what}: {line}");
            }
            failures.extend(f);
        }
    }
    WINDOW.with(|w| w.set((1.0, None, false)));
    run(vec![failures]);
}

/// Video on the split path (#1865 PR 4): the frame a CPU-decoded video
/// delivers, uploaded and composited, against the same frame drawn in the
/// scene; its letterbox black stays in the scene beneath it.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_video_matches_in_scene() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let v = |st: &'static str| -> Part { (st, Some(red())) };
    let color = red();
    let c2 = color.clone();
    let redraw = move |p: &Page| {
        c2.set([30, 160, 90, 255]);
        for v in &p.videos {
            v.redraw();
        }
    };
    let recolor = |p: &Page| {
        p.boxes.borrow()[1].set_style("background", "rgb(240, 200, 0)");
    };
    let noop = |_: &Page| {};
    let resize = |w: u32, h: u32| {
        move |p: &Page| {
            let v = &p.videos[0];
            v.surface
                .writer()
                .submit_frame(&video_frame([10, 120, 200, 255], (w, h)), w, h);
        }
    };
    run(vec![
        check(
            "a video, frame the size of its box",
            vec![
                v(
                    "video:120x80|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px",
                ),
                (SIDEBAR, None),
            ],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "a video letterboxed (scaled)",
            vec![v(
                "video:100x60|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px",
            )],
            RESAMPLED,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "a video with rounded corners",
            vec![v(
                "video:120x80|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; border-radius: 12px",
            )],
            EXACT,
            &[true],
            &[],
            &["full"],
            Fault::None,
        ),
        check(
            "a video under its controls, a new frame, the controls recoloured",
            vec![
                (
                    "video:120x80|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px",
                    Some(color.clone()),
                ),
                (OPAQUE_BAR, None),
            ],
            EXACT,
            &[true],
            &[&redraw, &recolor],
            // A new video frame alone: no repaint.
            &["full", "unchanged", "partial"],
            Fault::None,
        ),
        // A new frame of another size (adaptive streaming), then no more
        // (paused): the slice is made again for it, at once (#1906 review).
        check(
            "a new frame of another aspect, then paused",
            vec![v(
                "video:120x80|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px",
            )],
            RESAMPLED,
            &[true],
            &[&resize(100, 40), &noop, &noop],
            &["full", "partial", "unchanged", "unchanged"],
            Fault::None,
        ),
        check(
            "a new frame of the same aspect at another size, then paused",
            vec![
                (
                    "video:120x80|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px",
                    Some(red()),
                ),
                (OPAQUE_BAR, None),
            ],
            RESAMPLED,
            &[true],
            &[&resize(60, 40), &noop, &resize(240, 160), &noop],
            &["full", "partial", "unchanged", "partial", "unchanged"],
            Fault::None,
        ),
        check(
            "a video at half opacity: in the scene",
            vec![v(
                "video:120x80|position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; opacity: 0.5",
            )],
            EXACT,
            &[false],
            &[],
            &["full"],
            Fault::None,
        ),
    ]);
}

/// Only a video the window composites split uploads its frame, after paint
/// (#1906 review: a hidden, culled, refused or overlapped one uploaded every
/// new frame too, about 0.9 ms a frame at 1080p and 3.3 at 4K, and a hidden
/// one held its texture). `split_frame_uploads` counts them.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_video_uploads() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    const BOX: &str = "position: absolute; top: 40px; width: 100px; height: 80px";
    let parts: Vec<Part> = vec![
        // Split.
        (
            Box::leak(format!("video:100x80|{BOX}; left: 10px").into_boxed_str()),
            Some(red()),
        ),
        // Hidden.
        (
            Box::leak(format!("video:100x80|{BOX}; left: 130px; display: none").into_boxed_str()),
            Some(red()),
        ),
        // In an opacity group: in the scene.
        (
            Box::leak(format!("video:100x80|{BOX}; left: 250px; opacity: 0.5").into_boxed_str()),
            Some(red()),
        ),
        // Overlapping the first: in the scene (D7a).
        (
            Box::leak(format!("video:100x80|{BOX}; left: 60px; top: 60px").into_boxed_str()),
            Some(red()),
        ),
        // Off-screen: culled.
        (
            Box::leak(format!("video:100x80|{BOX}; left: 600px").into_boxed_str()),
            Some(red()),
        ),
    ];
    let mut p = page(&parts);
    let first = p.surfaces[0].id();
    let mut win = Window::new(true);
    let redraw = |p: &Page| {
        for v in &p.videos {
            v.redraw();
        }
    };
    let got = win.frame(&mut p.app, true, Fault::None);
    assert_eq!(got.split_ids, vec![first]);
    assert_eq!(got.uploads, 1, "the first frame: the split one's texture");
    for n in 0..3 {
        redraw(&p);
        let got = win.frame(&mut p.app, false, Fault::None);
        assert_eq!(got.split_ids, vec![first], "frame {n}");
        assert_eq!(got.uploads, 1, "frame {n}: one new frame of the split one");
    }
    let got = win.frame(&mut p.app, false, Fault::None);
    assert_eq!((got.kind, got.uploads), ("unchanged", 0), "no new frame");
    assert_eq!(
        win.split.pixel_textures(),
        1,
        "only the split one holds one"
    );
    for s in &p.surfaces {
        unregister_render_surface(s.id());
    }
    crate::shell::renderer::set_gpu_presenting(false);
}

// The #1906 review's probes, adopted: frame size changes, straight alpha on
// opaque and transparent windows, fractional scales, hide/show/move/unmount,
// several videos, and taint over 125 frames. They count pixel failures (a
// case's split expectation is not checked: some of them are in the scene).

fn rv_frame(rgba: [u8; 4], (w, h): (u32, u32)) -> Vec<u8> {
    (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| {
                [
                    rgba[0].wrapping_add((x * 7 + y * 3) as u8),
                    rgba[1].wrapping_add((x * 2 + y * 5) as u8),
                    rgba[2].wrapping_add((x + y * 11) as u8),
                    rgba[3],
                ]
            })
        })
        .collect()
}

fn rv_submit(p: &Page, i: usize, rgba: [u8; 4], size: (u32, u32)) {
    p.videos[i]
        .surface
        .writer()
        .submit_frame(&rv_frame(rgba, size), size.0, size.1);
}

const RV_BOX: &str = "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px";

fn rv_print(name: &str, f: Vec<String>) -> usize {
    for l in &f {
        eprintln!("[rv] {name}: {l}");
    }
    if f.is_empty() {
        eprintln!("[rv] {name}: clean");
    }
    f.iter().filter(|l| l.contains("allowance")).count()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_video_probes() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let v = |st: &'static str| -> Part { (st, Some(red())) };
    let noop = |_: &Page| {};
    let mut bad = 0;
    let only = std::env::var("RV_ONLY").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| n.contains(o));

    // 1. frame size changes mid-stream
    for (name, size) in [
        (
            "size change: aspect change 120x80 -> 100x40",
            (100u32, 40u32),
        ),
        ("size change: same aspect 120x80 -> 60x40", (60, 40)),
        ("size change: 120x80 -> 240x160", (240, 160)),
        ("size change: odd 120x80 -> 97x61", (97, 61)),
    ] {
        if !want(name) {
            continue;
        }
        let step = move |p: &Page| rv_submit(p, 0, [10, 120, 200, 255], size);
        let redraw_same = move |p: &Page| rv_submit(p, 0, [90, 20, 200, 255], size);
        bad += rv_print(
            name,
            check(
                name,
                vec![v(&*Box::leak(
                    format!("video:120x80|{RV_BOX}").into_boxed_str(),
                ))],
                RESAMPLED,
                &[true],
                &[&step, &noop, &noop, &redraw_same, &noop],
                &[],
                Fault::None,
            ),
        );
    }

    // 2. straight alpha, opaque and transparent windows
    for transparent in [false, true] {
        let name = format!("alpha frame, transparent window {transparent}");
        if !want(&name) {
            continue;
        }
        WINDOW.with(|w| w.set((1.0, None, transparent)));
        let a = |p: &Page| rv_submit(p, 0, [200, 100, 40, 128], (120, 80));
        let a2 = |p: &Page| rv_submit(p, 0, [20, 200, 140, 7], (120, 80));
        let a3 = |p: &Page| rv_submit(p, 0, [250, 250, 250, 0], (120, 80));
        bad += rv_print(
            &name,
            check(
                &name,
                vec![
                    v(Box::leak(format!("video:120x80|{RV_BOX}").into_boxed_str())),
                    (OPAQUE_BAR, None),
                ],
                EXACT,
                &[true],
                &[&a, &a2, &a3],
                &[],
                Fault::None,
            ),
        );
        // letterboxed (resampled) alpha
        let name = format!("alpha frame letterboxed, transparent window {transparent}");
        let a = |p: &Page| rv_submit(p, 0, [200, 100, 40, 128], (100, 60));
        bad += rv_print(
            &name,
            check(
                &name,
                vec![v(Box::leak(
                    format!("video:100x60|{RV_BOX}").into_boxed_str(),
                ))],
                RESAMPLED,
                &[true],
                &[&a],
                &[],
                Fault::None,
            ),
        );
    }
    WINDOW.with(|w| w.set((1.0, None, false)));

    // 3. fractional DPR, odd frame and fractional box
    for scale in [1.25f64, 1.5] {
        for (frame, style) in [
            (
                "97x61",
                "position: absolute; left: 50.5px; top: 40.25px; width: 121px; height: 79px",
            ),
            ("120x80", RV_BOX),
            (
                "61x97",
                "position: absolute; left: 33.3px; top: 21.7px; width: 151.3px; height: 77.1px",
            ),
            (
                "120x80",
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; border-radius: 11px",
            ),
        ] {
            let name = format!("scale {scale}, frame {frame}, {style}");
            if !want(&name) {
                continue;
            }
            WINDOW.with(|w| w.set((scale, None, false)));
            let (fw, fh) = frame.split_once('x').unwrap();
            let size: (u32, u32) = (fw.parse().unwrap(), fh.parse().unwrap());
            let redraw = move |p: &Page| rv_submit(p, 0, [5, 220, 30, 255], size);
            bad += rv_print(
                &name,
                check(
                    &name,
                    vec![
                        v(Box::leak(format!("video:{frame}|{style}").into_boxed_str())),
                        (OPAQUE_BAR, None),
                    ],
                    RESAMPLED,
                    &[true],
                    &[&redraw],
                    &[],
                    Fault::None,
                ),
            );
        }
    }
    WINDOW.with(|w| w.set((1.0, None, false)));

    // 4. hidden / shown / moved / unmounted
    let hide = |p: &Page| {
        p.boxes.borrow()[0].set_style("display", "none");
        rv_submit(p, 0, [10, 120, 200, 255], (120, 80));
    };
    let show = |p: &Page| {
        p.boxes.borrow()[0].set_style("display", "block");
    };
    let vis_hidden = |p: &Page| {
        p.boxes.borrow()[0].set_style("visibility", "hidden");
        rv_submit(p, 0, [10, 120, 200, 255], (120, 80));
    };
    let vis_visible = |p: &Page| {
        p.boxes.borrow()[0].set_style("visibility", "visible");
    };
    let redraw = |p: &Page| rv_submit(p, 0, [77, 33, 150, 255], (120, 80));
    let move_and_frame = |p: &Page| {
        p.boxes.borrow()[0].set_style("left", "90px");
        rv_submit(p, 0, [100, 10, 10, 255], (120, 80));
    };
    let bar_and_frame = |p: &Page| {
        p.boxes.borrow()[1].set_style("background", "rgb(240, 200, 0)");
        rv_submit(p, 0, [100, 100, 10, 255], (120, 80));
    };
    let bar_move_and_frame = |p: &Page| {
        p.boxes.borrow()[1].set_style("top", "60px");
        rv_submit(p, 0, [30, 100, 210, 255], (120, 80));
    };
    let remove = |p: &Page| {
        p.boxes.borrow()[0].remove();
        rv_submit(p, 0, [10, 120, 200, 255], (120, 80));
    };
    let unregister = |p: &Page| {
        unregister_render_surface(p.videos[0].surface.id());
    };
    let opacity_and_frame = |p: &Page| {
        p.boxes.borrow()[0].set_style("opacity", "0.5");
        rv_submit(p, 0, [30, 100, 210, 255], (120, 80));
    };
    let opaque_again = |p: &Page| {
        p.boxes.borrow()[0].set_style("opacity", "1");
        rv_submit(p, 0, [130, 10, 210, 255], (120, 80));
    };
    type Steps<'a> = Vec<&'a dyn Fn(&Page)>;
    let cases: Vec<(&str, Steps<'_>)> = vec![
        (
            "display none / redraw / block",
            vec![&hide, &redraw, &show, &redraw],
        ),
        (
            "visibility hidden / redraw / visible",
            vec![&vis_hidden, &redraw, &vis_visible, &redraw],
        ),
        (
            "move + new frame same frame",
            vec![&move_and_frame, &redraw],
        ),
        (
            "controls recolour + new frame same frame",
            vec![&bar_and_frame, &redraw],
        ),
        (
            "controls move + new frame same frame",
            vec![&bar_move_and_frame, &redraw],
        ),
        ("node removed + new frame", vec![&remove, &redraw, &noop]),
        (
            "surface unregistered, node stays",
            vec![&unregister, &noop, &redraw],
        ),
        (
            "opacity 0.5 + frame, then 1 + frame",
            vec![&opacity_and_frame, &redraw, &opaque_again, &redraw],
        ),
    ];
    for (name, steps) in cases {
        if !want(name) {
            continue;
        }
        bad += rv_print(
            name,
            check(
                name,
                vec![
                    v(Box::leak(format!("video:120x80|{RV_BOX}").into_boxed_str())),
                    (OPAQUE_BAR, None),
                ],
                EXACT,
                &[true],
                &steps,
                &[],
                Fault::None,
            ),
        );
    }

    // 5. several videos
    let all4 = |p: &Page| {
        for i in 0..p.videos.len() {
            let s = p.videos[i].size;
            rv_submit(p, i, [(i * 50) as u8, 90, 200, 255], s);
        }
    };
    for (name, specs) in [
        (
            "four videos side by side",
            vec![
                "video:120x80|position: absolute; left: 10px; top: 10px; width: 120px; height: 80px",
                "video:100x60|position: absolute; left: 140px; top: 10px; width: 120px; height: 80px",
                "video:120x80|position: absolute; left: 10px; top: 150px; width: 120px; height: 80px",
                "video:64x48|position: absolute; left: 140px; top: 150px; width: 120px; height: 80px",
            ],
        ),
        (
            "two videos overlapping",
            vec![
                "video:120x80|position: absolute; left: 10px; top: 10px; width: 120px; height: 80px",
                "video:120x80|position: absolute; left: 70px; top: 50px; width: 120px; height: 80px",
            ],
        ),
    ] {
        if !want(name) {
            continue;
        }
        let parts: Vec<Part> = specs.into_iter().map(v).collect();
        bad += rv_print(
            name,
            check(
                name,
                parts,
                RESAMPLED,
                &[true],
                &[&all4, &all4, &noop],
                &[],
                Fault::None,
            ),
        );
    }
    crate::shell::renderer::set_gpu_presenting(false);
    assert_eq!(bad, 0, "pixel failures (see [rv] lines)");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_video_taint() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let gray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "grayscale(1)");
        p.boxes.borrow()[1].set_style("background", "rgb(250, 100, 100)");
        rv_submit(p, 0, [1, 2, 3, 255], (120, 80));
    };
    let n = Rc::new(Cell::new(0u32));
    let n2 = n.clone();
    let frame = move |p: &Page| {
        let k = n2.get();
        n2.set(k + 1);
        rv_submit(p, 0, [(k / 2) as u8, 90, 30, 255], (120, 80));
    };
    let ungray = |p: &Page| {
        p.boxes.borrow()[1].set_style("filter", "none");
        rv_submit(p, 0, [9, 9, 9, 255], (120, 80));
    };
    let mut steps: Vec<&dyn Fn(&Page)> = vec![&gray];
    for _ in 0..125 {
        steps.push(&frame);
    }
    steps.push(&ungray);
    for _ in 0..3 {
        steps.push(&frame);
    }
    let f = check(
        "video taint",
        vec![
            (
                Box::leak(format!("video:120x80|{RV_BOX}").into_boxed_str()),
                Some(red()),
            ),
            (BADGE, None),
        ],
        EXACT,
        &[true],
        &steps,
        &[],
        Fault::None,
    );
    let bad = rv_print("video taint", f);
    crate::shell::renderer::set_gpu_presenting(false);
    assert_eq!(bad, 0);
}

/// The memory cap (D6), under a cap the test sets (`CAP_OVERRIDE`): each
/// 100x80 video carries a badge over it, so one above slice (32,000 bytes)
/// beside the shared scratch target. Uploaded frames are not counted (the
/// second amendment). Surfaces are split in paint order, after those kept
/// from earlier frames, until one does not fit; it and the rest stay in the
/// scene, pixel for pixel what in-scene draws. `check_split` also holds every
/// frame's allocation of slices and scratch to the cap.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_memory_cap() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    // A and B with their slices: 2 x 32,000 and a 256x128 scratch target
    // (195,072 in all); B and C: 2 x 32,000 and 384x128 (260,608); all three
    // 3 x 32,000 and 384x128 (292,608).
    const CAP: u64 = 270_000;
    crate::shell::split_frame::CAP_OVERRIDE.with(|c| c.set(Some(CAP)));
    let v = |st: &'static str| -> Part { (st, Some(red())) };
    const A: &str =
        "video:100x80|position: absolute; left: 10px; top: 40px; width: 100px; height: 80px";
    const B: &str =
        "video:100x80|position: absolute; left: 140px; top: 40px; width: 100px; height: 80px";
    const C: &str =
        "video:100x80|position: absolute; left: 270px; top: 40px; width: 100px; height: 80px";
    const BIG: &str =
        "video:100x80|position: absolute; left: 10px; top: 20px; width: 360px; height: 250px";
    const BADGE_A: &str = "position: absolute; left: 20px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    const BADGE_B: &str = "position: absolute; left: 150px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    const BADGE_C: &str = "position: absolute; left: 280px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    const BADGE_BIG: &str = "position: absolute; left: 20px; top: 30px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    const SMALL: &str =
        "video:100x80|position: absolute; left: 10px; top: 272px; width: 100px; height: 26px";
    const BADGE_SMALL: &str = "position: absolute; left: 20px; top: 278px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    let redraw = |p: &Page| {
        for v in &p.videos {
            v.redraw();
        }
    };
    let hide_first = |p: &Page| {
        p.boxes.borrow()[0].set_style("display", "none");
        p.boxes.borrow()[1].set_style("display", "none");
    };
    // The visible ones: a hidden viewport's new frame has no box to damage.
    let redraw_shown = |p: &Page| {
        for v in &p.videos[1..] {
            v.redraw();
        }
    };
    let three = || {
        vec![
            v(A),
            (BADGE_A, None),
            v(B),
            (BADGE_B, None),
            v(C),
            (BADGE_C, None),
        ]
    };
    run(vec![
        check_split(
            "three with slices, the third past the cap",
            three(),
            RESAMPLED,
            &|_, s| s[..2].to_vec(),
            // The refused one's new frame is damage; the split ones' are not.
            &[&redraw],
            &["full", "partial"],
            Fault::None,
        ),
        check_split(
            "one past the cap alone, then one that fits: neither splits",
            vec![v(BIG), (BADGE_BIG, None), v(SMALL), (BADGE_SMALL, None)],
            RESAMPLED,
            &|_, _| Vec::new(),
            &[],
            &["full"],
            Fault::None,
        ),
        check_split(
            "a split surface gone frees its memory: the refused one splits at its next frame",
            three(),
            RESAMPLED,
            &|n, s| match n {
                0 => s[..2].to_vec(),
                1 => s[1..2].to_vec(),
                _ => s[1..].to_vec(),
            },
            &[&hide_first, &redraw_shown],
            &["full", "partial", "partial"],
            Fault::None,
        ),
    ]);
    crate::shell::split_frame::CAP_OVERRIDE.with(|c| c.set(None));
}

/// A 4K-size frame under its controls splits at the real cap: the frame is
/// not counted (D6, second amendment), only the slice and the scratch target.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_memory_cap_large_frame_splits() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    // 4 times the window's frame bytes, more than the 4x cap alone.
    const VIDEO: &str =
        "video:800x600|position: absolute; left: 0; top: 0; width: 400px; height: 300px";
    const CONTROLS: &str = "position: absolute; left: 0; top: 270px; width: 400px; height: 30px; background: rgba(0, 0, 0, 0.6)";
    run(vec![check_split(
        "a frame of four windows' bytes under its controls",
        vec![(VIDEO, Some(red())), (CONTROLS, None)],
        RESAMPLED,
        &|_, s| s.to_vec(),
        &[],
        &["full"],
        Fault::None,
    )]);
}

/// A split surface that grows past the cap goes to the scene once, and
/// stays there: its leaving the split set is not memory freed for a retry
/// (which would paint every frame twice, refusing it again each time).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_memory_cap_is_not_retried_every_frame() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    crate::shell::split_frame::CAP_OVERRIDE.with(|c| c.set(Some(250_000)));
    let parts: Vec<Part> = vec![
        (
            "video:100x80|position: absolute; left: 10px; top: 40px; width: 100px; height: 80px",
            Some(red()),
        ),
        (
            "position: absolute; left: 20px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)",
            None,
        ),
        (
            "video:100x80|position: absolute; left: 140px; top: 40px; width: 100px; height: 80px",
            Some(red()),
        ),
        (
            "position: absolute; left: 150px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)",
            None,
        ),
    ];
    let mut p = page(&parts);
    let ids: Vec<usize> = p.surfaces.iter().map(|s| s.id()).collect();
    let mut win = Window::with(true, 1.0, None, false);
    let sorted = |mut v: Vec<usize>| {
        v.sort_unstable();
        v
    };
    let got = win.frame(&mut p.app, true, Fault::None);
    assert_eq!(sorted(got.split_ids), sorted(ids.clone()));
    assert_eq!(p.app.split_cap_repaints(), 0);
    // The second grows to 250x250: its slice alone is 250,000 bytes.
    p.boxes.borrow()[2].set_style("width", "250px");
    p.boxes.borrow()[2].set_style("height", "250px");
    let got = win.frame(&mut p.app, false, Fault::None);
    assert_eq!(
        got.split_ids,
        vec![ids[0]],
        "the grown one goes to the scene"
    );
    assert_eq!(p.app.split_cap_repaints(), 1);
    for n in 0..4 {
        for v in &p.videos {
            v.redraw();
        }
        let got = win.frame(&mut p.app, false, Fault::None);
        assert_eq!(got.split_ids, vec![ids[0]], "frame {n}");
        assert_eq!(p.app.split_cap_repaints(), 1, "frame {n}: painted again");
        assert!(win.split.resident_bytes() - win.split.upload_bytes() <= 250_000);
    }
    for s in &p.surfaces {
        unregister_render_surface(s.id());
    }
    crate::shell::split_frame::CAP_OVERRIDE.with(|c| c.set(None));
    crate::shell::renderer::set_gpu_presenting(false);
}

// The #1899 second review's probes, adopted: carets blinking in a surface's
// shadow and over its border, surfaces moving, ink and transforms reaching
// into R, adjacent surfaces, R at the window's edges, fixed boxes over a
// scrolled surface, the left-edge spill, and `above_out`'s replay of an outer
// clip. Each frame against in-scene, and painted once: the fallback that
// paints a frame again is a safety net, never the path (`split_paint_passes`).
#[cfg(test)]
// Adopted as the review wrote them.
#[allow(clippy::type_complexity, clippy::vec_init_then_push)]
mod review_b {
    use super::*;

    /// A page built by `build` (root children), with surfaces from `colors`.
    pub(super) fn custom_page(
        colors: &[Rc<Cell<[u8; 4]>>],
        build: Rc<
            dyn Fn(
                &mut RenderScope,
                &NodeHandle,
                &[RenderSurfaceHandle],
                &RefCell<Vec<NodeHandle>>,
            ),
        >,
    ) -> Page {
        let surfaces: Vec<RenderSurfaceHandle> = colors.iter().cloned().map(surface).collect();
        let boxes: Rc<RefCell<Vec<NodeHandle>>> = Default::default();
        let kept = boxes.clone();
        let ss = surfaces.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
                 background: rgb(60, 70, 90)",
            );
            build(scope, &root, &ss, &kept);
            root
        });
        app.mount_component(W as f32, H as f32);
        app.resolve_and_repaint(W as f32, H as f32);
        Page {
            app,
            boxes,
            surfaces,
            videos: Vec::new(),
        }
    }

    /// Every frame against in-scene; prints paint passes per frame.
    pub(super) fn probe_b(
        name: &str,
        make: &dyn Fn() -> Page,
        edges: bool,
        steps: &[&dyn Fn(&Page)],
    ) -> (Vec<String>, Vec<usize>) {
        let mut failures = Vec::new();
        let mut per_frame = Vec::new();
        let mut split_page = make();
        let mut scene_page = make();
        let mut win = Window::new(true);
        let mut reference = Window::new(false);
        let noop = |_: &Page| {};
        let all: Vec<&dyn Fn(&Page)> = std::iter::once(&noop as &dyn Fn(&Page))
            .chain(steps.iter().copied())
            .collect();
        for (n, step) in all.into_iter().enumerate() {
            step(&split_page);
            step(&scene_page);
            let p0 = split_page.app.split_paint_passes() as usize;
            let got = win.frame(&mut split_page.app, n == 0, Fault::None);
            let p = split_page.app.split_paint_passes() as usize - p0;
            if p > 1 {
                failures.push(format!("{name}, frame {n}: painted {p} times"));
            }
            per_frame.push(p);
            let want = reference.frame(&mut scene_page.app, true, Fault::None);
            let wrong = compare(
                &got.pixels,
                &want.pixels,
                &win.allowances(if edges { EDGES } else { EXACT }),
                win.dev.0,
            );
            eprintln!(
                "[rb] {name}, frame {n}: {} frame, passes {p}, split {:?}, {} wrong",
                got.kind,
                got.split_ids,
                wrong.len(),
            );
            if !wrong.is_empty() {
                failures.push(format!(
                    "{name}, frame {n} ({}, split {:?}): {} pixels past allowance: {:?}",
                    got.kind,
                    got.split_ids,
                    wrong.len(),
                    &wrong[..wrong.len().min(6)]
                ));
            }
        }
        for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
            unregister_render_surface(s.id());
        }
        eprintln!("[rb] {name}: passes per frame {per_frame:?}");
        (failures, per_frame)
    }

    fn simple(parts: Vec<Part>) -> impl Fn() -> Page {
        move || page(&parts)
    }

    fn report(all: Vec<(Vec<String>, Vec<usize>)>) {
        let failures: Vec<String> = all.into_iter().flat_map(|f| f.0).collect();
        crate::shell::renderer::set_gpu_presenting(false);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    const CARET_IN: &str = "position: absolute; left: 90px; top: 100px; width: 2px; height: 14px; \
                            background: rgb(255, 255, 255)";

    /// Q1: caret blinking over / near a surface; a moving surface.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_caret_and_motion() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let blink_on = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgb(255, 255, 255)");
        let blink_off = |p: &Page| p.boxes.borrow()[1].set_style("background", "transparent");
        let blink: Vec<&dyn Fn(&Page)> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    &blink_off as &dyn Fn(&Page)
                } else {
                    &blink_on
                }
            })
            .collect();
        let mut out = Vec::new();
        // a) caret inside R (an input over the viewport).
        out.push(probe_b(
            "caret inside R",
            &simple(vec![(SURFACE, Some(red())), (CARET_IN, None)]),
            false,
            &blink,
        ));
        // b) caret beside R, the surface plain.
        out.push(probe_b(
            "caret beside plain surface",
            &simple(vec![
                (SURFACE, Some(red())),
                (
                    "position: absolute; left: 176px; top: 60px; width: 2px; height: 14px; \
                     background: rgb(255, 255, 255)",
                    None,
                ),
            ]),
            false,
            &blink,
        ));
        // c) caret beside R inside the surface's box-shadow.
        out.push(probe_b(
            "caret in surface's shadow",
            &simple(vec![
                (
                    "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                     box-shadow: 0 0 10px rgb(0, 0, 0)",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 174px; top: 60px; width: 2px; height: 14px; \
                     background: rgb(255, 255, 255)",
                    None,
                ),
            ]),
            true,
            &blink,
        ));
        // c2) caret right of R inside the surface's 2px border.
        out.push(probe_b(
            "caret over surface border",
            &simple(vec![
                (
                    "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                     border: 4px solid rgb(0, 0, 200)",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 172px; top: 60px; width: 2px; height: 14px; \
                     background: rgb(255, 255, 255)",
                    None,
                ),
            ]),
            false,
            &blink,
        ));
        // d) the surface moving 3 px a frame (a dragged panel).
        let moves: Vec<Box<dyn Fn(&Page)>> = (1..=8)
            .map(|i| {
                Box::new(move |p: &Page| {
                    p.boxes.borrow()[0].set_style("left", &format!("{}px", 50 + 3 * i));
                }) as Box<dyn Fn(&Page)>
            })
            .collect();
        let refs: Vec<&dyn Fn(&Page)> = moves.iter().map(|b| b.as_ref()).collect();
        out.push(probe_b(
            "surface moving",
            &simple(vec![(SURFACE, Some(red())), (SIDEBAR, None)]),
            false,
            &refs,
        ));
        // e) moving at fractional positions.
        let fmoves: Vec<Box<dyn Fn(&Page)>> = (1..=8)
            .map(|i| {
                Box::new(move |p: &Page| {
                    p.boxes.borrow()[0].set_style("left", &format!("{}px", 50.0 + 2.3 * i as f64));
                    p.boxes.borrow()[0].set_style("top", &format!("{}px", 40.0 + 1.7 * i as f64));
                }) as Box<dyn Fn(&Page)>
            })
            .collect();
        let refs: Vec<&dyn Fn(&Page)> = fmoves.iter().map(|b| b.as_ref()).collect();
        out.push(probe_b(
            "surface moving fractional, badge over",
            &simple(vec![(SURFACE, Some(red())), (BADGE, None)]),
            false,
            &refs,
        ));
        report(out);
    }

    /// Q1: ink overflow of nodes over R; transforms.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_ink_overflow_and_transforms() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let recolor = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgb(255, 0, 255)");
        let recolor2 = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgb(0, 200, 255)");
        let outline_col =
            |p: &Page| p.boxes.borrow()[1].set_style("outline-color", "rgb(255, 255, 0)");
        let shadow_col =
            |p: &Page| p.boxes.borrow()[1].set_style("box-shadow", "0 0 0 8px rgb(255, 255, 0)");
        let tick = |p: &Page| p.boxes.borrow()[2].set_style("background", "rgb(255, 0, 255)");
        let mut out = Vec::new();
        // a) a sibling outside R whose outline reaches into R.
        out.push(probe_b(
            "outline into R",
            &simple(vec![
                (SURFACE, Some(red())),
                (
                    "position: absolute; left: 180px; top: 60px; width: 30px; height: 20px; \
                     background: rgb(0, 255, 0); outline: 14px solid rgb(250, 250, 250)",
                    None,
                ),
            ]),
            false,
            &[&recolor, &outline_col, &recolor2],
        ));
        // b) a sibling outside R whose spread shadow reaches into R.
        out.push(probe_b(
            "spread shadow into R",
            &simple(vec![
                (SURFACE, Some(red())),
                (
                    "position: absolute; left: 176px; top: 60px; width: 30px; height: 20px; \
                     background: rgb(0, 255, 0); box-shadow: 0 0 0 10px rgb(250, 250, 250)",
                    None,
                ),
            ]),
            false,
            &[&recolor, &shadow_col, &recolor2],
        ));
        // c) a box over R whose shadow spills outside R; a tick changes in the
        //    spill only (outside R), then the box itself.
        out.push(probe_b(
            "box over R, tick in its spill",
            &simple(vec![
                (SURFACE, Some(red())),
                (
                    "position: absolute; left: 150px; top: 60px; width: 18px; height: 20px; \
                     background: rgb(0, 255, 0); box-shadow: 0 0 0 12px rgb(250, 250, 250)",
                    None,
                ),
                (
                    "position: absolute; left: 172px; top: 62px; width: 4px; height: 4px; \
                     background: rgb(0, 0, 0)",
                    None,
                ),
            ]),
            false,
            &[&tick, &recolor],
        ));
        // d) a rotated sibling across R's edge, recoloured.
        out.push(probe_b(
            "rotated across edge",
            &simple(vec![
                (SURFACE, Some(red())),
                (
                    "position: absolute; left: 150px; top: 60px; width: 50px; height: 20px; \
                     background: rgb(0, 255, 0); transform: rotate(20deg)",
                    None,
                ),
            ]),
            true,
            &[&recolor, &recolor2],
        ));
        // e) a scaled sibling outside R whose transform carries it into R.
        out.push(probe_b(
            "scaled into R",
            &simple(vec![
                (SURFACE, Some(red())),
                (
                    "position: absolute; left: 182px; top: 60px; width: 10px; height: 10px; \
                     background: rgb(0, 255, 0); transform: scale(4)",
                    None,
                ),
            ]),
            false,
            &[&recolor, &recolor2],
        ));
        // f) the surface at a fractional translate.
        out.push(probe_b(
            "surface translate(0.5, 0.25)",
            &simple(vec![
                (
                    "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                     transform: translate(0.5px, 0.25px)",
                    Some(red()),
                ),
                (BADGE, None),
            ]),
            false,
            &[&recolor, &recolor2],
        ));
        report(out);
    }

    /// Q1/Q2: adjacent surfaces; R at the window's edges; window-sized clips.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_adjacent_and_edges() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let green = || Rc::new(Cell::new([30, 160, 90, 255]));
        let mut out = Vec::new();
        let draw2 = |p: &Page| p.surfaces[1].request_frame();
        let draw1 = |p: &Page| p.surfaces[0].request_frame();
        let badge = |p: &Page| p.boxes.borrow()[2].set_style("background", "rgb(255, 0, 255)");
        // a) two surfaces side by side, touching at x = 150.
        out.push(probe_b(
            "adjacent surfaces",
            &simple(vec![
                (
                    "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 150px; top: 40px; width: 100px; height: 80px",
                    Some(green()),
                ),
                (
                    "position: absolute; left: 60px; top: 50px; width: 20px; height: 12px; \
                     background: rgb(250, 250, 250)",
                    None,
                ),
            ]),
            false,
            &[&draw2, &draw2, &draw1, &badge, &draw2],
        ));
        // b) stacked vertically, touching at y = 120 (no left growth there).
        out.push(probe_b(
            "stacked surfaces",
            &simple(vec![
                (
                    "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 50px; top: 120px; width: 100px; height: 80px",
                    Some(green()),
                ),
                (BADGE, None),
            ]),
            false,
            &[&draw2, &draw1, &badge],
        ));
        let recolor = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgb(255, 0, 255)");
        let recolor2 = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgba(0, 0, 0, 0.4)");
        // c) R at x = 0 and y = 0, a bar across its right edge.
        out.push(probe_b(
            "R at the window's left",
            &simple(vec![
                (
                    "position: absolute; left: 0px; top: 0px; width: 120px; height: 80px",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 0px; top: 30px; width: 200px; height: 14px; \
                     background: rgba(0, 0, 0, 0.55)",
                    None,
                ),
            ]),
            true,
            &[&recolor, &recolor2],
        ));
        // d) R at the window's right and bottom edges.
        out.push(probe_b(
            "R at the window's right",
            &simple(vec![
                (
                    "position: absolute; left: 280px; top: 220px; width: 120px; height: 80px",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 250px; top: 250px; width: 150px; height: 14px; \
                     background: rgba(0, 0, 0, 0.55)",
                    None,
                ),
            ]),
            true,
            &[&recolor, &recolor2],
        ));
        // e) R past the window's right (fractional left).
        out.push(probe_b(
            "R past the right, fractional",
            &simple(vec![
                (
                    "position: absolute; left: 300.4px; top: 100px; width: 160px; height: 80px",
                    Some(red()),
                ),
                (
                    "position: absolute; left: 280px; top: 130px; width: 150px; height: 14px; \
                     background: rgba(0, 0, 0, 0.55)",
                    None,
                ),
            ]),
            true,
            &[&recolor, &recolor2],
        ));
        // f) a window-sized overflow clip (whole pixels), the surface in it,
        //    a translucent bar from R to the window's edge.
        out.push(probe_b(
            "window-sized clip",
            &simple(vec![
                (
                    "wrap:position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
                     overflow: hidden|position: absolute; left: 50px; top: 40px; width: 120px; \
                     height: 80px",
                    Some(red()),
                ),
                (
                    "wrap:position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
                     overflow: hidden|position: absolute; left: 0px; top: 100px; width: 400px; \
                     height: 14px; background: rgba(0, 0, 0, 0.55)",
                    None,
                ),
            ]),
            true,
            &[&recolor, &recolor2],
        ));
        // g) a window-sized rounded clip around both.
        out.push(probe_b(
            "window-sized rounded clip",
            &move || {
                custom_page(
                    &[red()],
                    Rc::new(|scope, root, ss, kept| {
                        let clip = scope.create_element("div");
                        clip.set_attribute(
                            "style",
                            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
                             overflow: hidden; border-radius: 16px; background: rgb(90, 60, 70)",
                        );
                        let n = RenderSurface {
                            surface: Some(ss[0].clone()),
                        }
                        .render(scope, &[]);
                        n.set_attribute(
                            "style",
                            "position: absolute; left: 0px; top: 0px; width: 120px; height: 80px",
                        );
                        let bar = scope.create_element("div");
                        bar.set_attribute(
                            "style",
                            "position: absolute; left: 0px; top: 30px; width: 400px; height: 14px; \
                             background: rgba(0, 0, 0, 0.55)",
                        );
                        clip.append_child(&n);
                        clip.append_child(&bar);
                        root.append_child(&clip);
                        kept.borrow_mut().push(n);
                        kept.borrow_mut().push(bar);
                    }),
                )
            },
            true,
            &[&recolor, &recolor2],
        ));
        report(out);
    }

    /// Q1: position: fixed over a scrolled surface.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_fixed_over_scrolled() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let make = || {
            custom_page(
                &[red()],
                Rc::new(|scope, root, ss, kept| {
                    let sc = scope.create_element("div");
                    sc.set_attribute(
                        "style",
                        "position: absolute; left: 20px; top: 20px; width: 300px; height: 200px; \
                         overflow: auto; background: rgb(40, 40, 40)",
                    );
                    let inner = scope.create_element("div");
                    inner.set_attribute("style", "position: relative; width: 280px; height: 800px");
                    let n = RenderSurface {
                        surface: Some(ss[0].clone()),
                    }
                    .render(scope, &[]);
                    n.set_attribute(
                        "style",
                        "position: absolute; left: 30px; top: 60px; width: 160px; height: 100px",
                    );
                    inner.append_child(&n);
                    sc.append_child(&inner);
                    root.append_child(&sc);
                    let fixed = scope.create_element("div");
                    fixed.set_attribute(
                        "style",
                        "position: fixed; left: 70px; top: 90px; width: 40px; height: 16px; \
                         background: rgba(255, 255, 255, 0.7)",
                    );
                    root.append_child(&fixed);
                    kept.borrow_mut().push(sc);
                    kept.borrow_mut().push(fixed);
                }),
            )
        };
        let scrolls: Vec<Box<dyn Fn(&Page)>> = [7.0, 15.0, 33.5, 60.0, 61.0, 140.0, 20.0]
            .into_iter()
            .map(|y| {
                Box::new(move |p: &Page| p.boxes.borrow()[0].set_scroll_top(y))
                    as Box<dyn Fn(&Page)>
            })
            .collect();
        let refs: Vec<&dyn Fn(&Page)> = scrolls.iter().map(|b| b.as_ref()).collect();
        let out = vec![probe_b("fixed over scrolled", &make, true, &refs)];
        report(out);
    }
}

#[cfg(test)]
// Adopted as the review wrote them.
#[allow(clippy::type_complexity, clippy::vec_init_then_push)]
mod review_b_idle {
    use super::review_b::probe_b;
    use super::*;

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_idle_in_scene() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let noop = |_: &Page| {};
        let green = || Rc::new(Cell::new([30, 160, 90, 255]));
        let mut f = Vec::new();
        f.extend(
            probe_b(
                "adjacent idle",
                &|| {
                    page(&[
                        (
                            "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px",
                            Some(red()),
                        ),
                        (
                            "position: absolute; left: 150px; top: 40px; width: 100px; height: 80px",
                            Some(green()),
                        ),
                    ])
                },
                false,
                &[&noop, &noop, &noop],
            )
            .0,
        );
        f.extend(
            probe_b(
                "opacity idle",
                &|| {
                    page(&[(
                        "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px; opacity: 0.9",
                        Some(red()),
                    )])
                },
                false,
                &[&noop, &noop, &noop],
            )
            .0,
        );
        f.extend(
            probe_b(
                "gap of 1px",
                &|| {
                    page(&[
                        (
                            "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px",
                            Some(red()),
                        ),
                        (
                            "position: absolute; left: 151px; top: 40px; width: 100px; height: 80px",
                            Some(green()),
                        ),
                    ])
                },
                false,
                &[&noop],
            )
            .0,
        );
        crate::shell::renderer::set_gpu_presenting(false);
        assert!(f.is_empty(), "{}", f.join("\n"));
    }
}

#[cfg(test)]
// Adopted as the review wrote them.
#[allow(clippy::type_complexity, clippy::vec_init_then_push)]
mod review_b_edges {
    use super::review_b::{custom_page, probe_b};
    use super::*;

    /// A box whose left edge is R's right edge, fractional top (the AA spill).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_touching_right_edge() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let recolor = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgb(255, 0, 255)");
        let mut f = Vec::new();
        for (sl, bl, bt) in [
            ("50px", "170px", "60.5px"),
            ("50px", "170px", "60.25px"),
            ("50.5px", "170.5px", "60.5px"),
            ("50px", "170.3px", "60px"),
            ("50px", "169.7px", "60.5px"),
        ] {
            let s1 =
                format!("position: absolute; left: {sl}; top: 40px; width: 120px; height: 80px");
            let s2 = format!(
                "position: absolute; left: {bl}; top: {bt}; width: 30px; height: 20.5px; \
                 background: rgb(250, 250, 250)"
            );
            let s1: &'static str = Box::leak(s1.into_boxed_str());
            let s2: &'static str = Box::leak(s2.into_boxed_str());
            f.extend(
                probe_b(
                    &format!("touching right {sl} {bl} {bt}"),
                    &move || page(&[(s1, Some(red())), (s2, None)]),
                    false,
                    &[&recolor],
                )
                .0,
            );
        }
        crate::shell::renderer::set_gpu_presenting(false);
        assert!(f.is_empty(), "{}", f.join("\n"));
    }

    /// Nested whole-pixel clips around the surface (an outer scroller, an
    /// inner wrapper), the surface reaching past the scroller's bottom; a
    /// sibling after the inner wrapper closes, inside the scroller, over R
    /// and past the scroller's edge: `above_out` must replay the scroller.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_above_out_replays_outer_clip() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let make = || {
            custom_page(
                &[red()],
                Rc::new(|scope, root, ss, kept| {
                    let sc = scope.create_element("div");
                    sc.set_attribute(
                        "style",
                        "position: absolute; left: 20px; top: 20px; width: 300px; height: 150px; \
                         overflow: hidden; background: rgb(40, 40, 40)",
                    );
                    let wrap = scope.create_element("div");
                    wrap.set_attribute(
                        "style",
                        "position: absolute; left: 10px; top: 10px; width: 260px; height: 220px; \
                         overflow: hidden",
                    );
                    let n = RenderSurface {
                        surface: Some(ss[0].clone()),
                    }
                    .render(scope, &[]);
                    n.set_attribute(
                        "style",
                        "position: absolute; left: 20px; top: 60px; width: 160px; height: 140px",
                    );
                    wrap.append_child(&n);
                    sc.append_child(&wrap);
                    let bar = scope.create_element("div");
                    bar.set_attribute(
                        "style",
                        "position: absolute; left: 60px; top: 120px; width: 60px; height: 60px; \
                         background: rgb(250, 250, 250)",
                    );
                    sc.append_child(&bar);
                    root.append_child(&sc);
                    kept.borrow_mut().push(bar);
                }),
            )
        };
        let recolor = |p: &Page| p.boxes.borrow()[0].set_style("background", "rgb(0, 255, 255)");
        let f = probe_b("above_out replay", &make, false, &[&recolor]).0;
        crate::shell::renderer::set_gpu_presenting(false);
        assert!(f.is_empty(), "{}", f.join("\n"));
    }
}

#[cfg(test)]
// Adopted as the review wrote them.
#[allow(clippy::type_complexity, clippy::vec_init_then_push)]
mod review_b_spill {
    use super::review_b::probe_b;
    use super::*;

    /// The #1894 review's 9-level case at runtime scale: a black box whose
    /// left edge is R's right edge, its top fractional.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_review2_spill_exact() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let recolor = |p: &Page| p.boxes.borrow()[1].set_style("background", "rgb(255, 255, 255)");
        let mut f = Vec::new();
        for top in ["60.4px", "60.1px", "60.7px", "41.3px"] {
            let s2 = format!(
                "position: absolute; left: 170px; top: {top}; width: 20px; height: 10px; \
                 background: rgb(0, 0, 0)"
            );
            let s2: &'static str = Box::leak(s2.into_boxed_str());
            f.extend(
                probe_b(
                    &format!("spill exact top {top}"),
                    &move || page(&[(SURFACE, Some(red())), (s2, None)]),
                    false,
                    &[&recolor],
                )
                .0,
            );
        }
        crate::shell::renderer::set_gpu_presenting(false);
        assert!(f.is_empty(), "{}", f.join("\n"));
    }
}

/// Two surfaces side by side at whole pixels, touching: both split, and a
/// frame in which only they draw repaints nothing (#1899 review: a region
/// grown a pixel left overlapped its neighbour, the second was refused, and
/// every one of its frames became a partial repaint).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_adjacent_surfaces_both_split() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let green = Rc::new(Cell::new([30, 160, 90, 255]));
    let p = page(&[
        (
            "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px",
            Some(red()),
        ),
        (
            "position: absolute; left: 150px; top: 40px; width: 100px; height: 80px",
            Some(green),
        ),
    ]);
    let mut p = p;
    let mut ids: Vec<usize> = p.surfaces.iter().map(|s| s.id()).collect();
    ids.sort_unstable();
    let mut win = Window::new(true);
    let mut reference = Window::new(false);
    let mut scene = page(&[
        (
            "position: absolute; left: 50px; top: 40px; width: 100px; height: 80px",
            Some(red()),
        ),
        (
            "position: absolute; left: 150px; top: 40px; width: 100px; height: 80px",
            Some(Rc::new(Cell::new([30, 160, 90, 255]))),
        ),
    ]);
    for n in 0..4 {
        if n > 0 {
            for s in p.surfaces.iter().chain(&scene.surfaces) {
                s.request_frame();
            }
        }
        let got = win.frame(&mut p.app, n == 0, Fault::None);
        let want = reference.frame(&mut scene.app, true, Fault::None);
        let mut split = got.split_ids.clone();
        split.sort_unstable();
        assert_eq!(split, ids, "frame {n}: both split");
        if n > 0 {
            assert_eq!(
                got.kind, "unchanged",
                "frame {n}: new frames alone repaint nothing"
            );
        }
        let wrong = compare(&got.pixels, &want.pixels, &win.allowances(EXACT), win.dev.0);
        assert!(
            wrong.is_empty(),
            "frame {n}: {:?}",
            &wrong[..wrong.len().min(6)]
        );
    }
    for s in p.surfaces.iter().chain(&scene.surfaces) {
        unregister_render_surface(s.id());
    }
    crate::shell::renderer::set_gpu_presenting(false);
}

/// A surface refused (for the cap, or for overlap) and drawn in the scene
/// beside split ones, at fractional scales (#1911 review). In the scene,
/// Vello's output for a resampled image depends on which other images share
/// its atlas, and the reference holds the split ones' frames there too: D10's
/// atlas note, one level inside the refused frame's box (measured: 145
/// pixels at 1.05 between two in-scene windows, one with its neighbours'
/// frames and one without). Everything else as in-scene, and the split set
/// as expected.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_refused_at_fractional_scale() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let v = |st: &'static str| -> Part { (st, Some(red())) };
    const A: &str =
        "video:500x400|position: absolute; left: 10px; top: 40px; width: 100px; height: 80px";
    const B: &str =
        "video:500x400|position: absolute; left: 140px; top: 40px; width: 100px; height: 80px";
    const BADGE_A: &str = "position: absolute; left: 20px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    const BADGE_B: &str = "position: absolute; left: 150px; top: 50px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    // Far from A and B: its slice moves the scratch target to the window's
    // corner, past the cap.
    const C: &str =
        "video:500x400|position: absolute; left: 260px; top: 180px; width: 130px; height: 100px";
    const BADGE_C: &str = "position: absolute; left: 270px; top: 190px; width: 20px; height: 12px; background: rgb(250, 250, 250)";
    // Over B: refused for overlap.
    const D: &str =
        "video:500x400|position: absolute; left: 200px; top: 70px; width: 100px; height: 80px";
    let redraw = |p: &Page| {
        for v in &p.videos {
            v.redraw();
        }
    };
    let mut failures = Vec::new();
    for (scale, cap) in [(1.05, 500_000u64), (1.5, 900_000)] {
        WINDOW.with(|w| w.set((scale, None, false)));
        crate::shell::split_frame::CAP_OVERRIDE.with(|c| c.set(Some(cap)));
        ATLAS.with(|a| *a.borrow_mut() = vec![(260.0, 180.0, 130.0, 100.0)]);
        failures.extend(check_split(
            &format!("scale {scale}: the third refused by the cap"),
            vec![
                v(A),
                (BADGE_A, None),
                v(B),
                (BADGE_B, None),
                v(C),
                (BADGE_C, None),
            ],
            RESAMPLED,
            &|_, s| s[..2].to_vec(),
            &[&redraw, &redraw],
            &[],
            Fault::None,
        ));
        crate::shell::split_frame::CAP_OVERRIDE.with(|c| c.set(None));
        ATLAS.with(|a| *a.borrow_mut() = vec![(200.0, 70.0, 100.0, 80.0)]);
        failures.extend(check_split(
            &format!("scale {scale}: the third refused for overlap"),
            vec![v(A), v(B), v(D)],
            RESAMPLED,
            &|_, s| s[..2].to_vec(),
            &[&redraw, &redraw],
            &[],
            Fault::None,
        ));
    }
    ATLAS.with(|a| a.borrow_mut().clear());
    WINDOW.with(|w| w.set((1.0, None, false)));
    run(vec![failures]);
}

/// S4: a change inside a kept region painted after its surface goes to the
/// region's above slices, and below is not re-rendered over the region; any
/// other change there re-renders it. Each case against in-scene pixels, the
/// frames' `split_below_kept` pinned.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_above_only_changes() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    // The surface rounded, so below shows at its corners inside R.
    const ROUNDED: &str = "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                           box-sizing: border-box; border-radius: 14px";
    // In flow, before the surface: its removal is a partial frame.
    const UNDER_FLOW: &str = "margin-left: 40px; margin-top: 30px; width: 140px; height: 100px; \
                              background: rgb(200, 60, 160)";
    const UNDER: &str = "position: absolute; left: 40px; top: 30px; width: 140px; height: 100px; \
                         background: rgb(200, 60, 160)";
    const LABEL: &str = "position: absolute; left: 70px; top: 60px; width: 60px; height: 16px; \
                         color: white; font: 12px sans-serif";
    // Later in the tree, painted before the surface (a lower z-index).
    const LOW_Z: &str = "position: absolute; left: 44px; top: 34px; width: 132px; height: 92px; \
                         background: rgb(40, 180, 90); z-index: 1";
    const HIGH_Z: &str = "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                          box-sizing: border-box; border-radius: 14px; z-index: 2";
    struct Case {
        name: &'static str,
        parts: Vec<Part>,
        target: usize,
        step: &'static dyn Fn(&NodeHandle),
        kind: &'static str,
        below_kept: u64,
    }
    let recolor: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| {
        n.set_style("background", "rgb(240, 200, 0)");
    };
    let remove: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| n.remove();
    let raise: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| {
        n.set_style("z-index", "3");
    };
    let span_colour: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| {
        n.children()[1].set_style("color", "rgb(255, 255, 0)");
    };
    // Over the corner, beneath a z-index 2 surface until raised.
    const UNDER_Z: &str = "position: absolute; left: 150px; top: 100px; width: 40px; height: 40px; \
                           background: rgba(0, 0, 255, 0.5); z-index: 1";
    let shift: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| {
        n.set_style("left", "90px");
    };
    let hide: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| {
        n.set_style("display", "none");
    };
    static TICK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let retext: &'static dyn Fn(&NodeHandle) = &|n: &NodeHandle| {
        // The same text on both pages: each tick is set twice in a row.
        let t = TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) / 2;
        n.set_text(&format!("12:{t:02}"));
    };
    let cases = vec![
        Case {
            name: "controls across the region recoloured: below kept inside it",
            parts: vec![(ROUNDED, Some(red())), (OPAQUE_BAR, None)],
            target: 1,
            step: recolor,
            kind: "partial",
            below_kept: 1,
        },
        Case {
            name: "a badge inside the region recoloured: no below render at all",
            parts: vec![(ROUNDED, Some(red())), (BADGE, None)],
            target: 1,
            step: recolor,
            kind: "unchanged",
            below_kept: 1,
        },
        Case {
            name: "a label's text over the surface changes",
            parts: vec![(ROUNDED, Some(red())), (LABEL, None)],
            target: 1,
            step: retext,
            kind: "",
            below_kept: 1,
        },
        Case {
            name: "a badge over the surface moved within it: below kept",
            parts: vec![(ROUNDED, Some(red())), (BADGE, None)],
            target: 1,
            step: shift,
            kind: "unchanged",
            below_kept: 1,
        },
        Case {
            name: "a span's colour in a label over the surface: below kept",
            parts: vec![(ROUNDED, Some(red())), (LABEL, None)],
            target: 1,
            step: span_colour,
            kind: "",
            below_kept: 1,
        },
        Case {
            // Its old pixels are in below, its new start after the split: a
            // reordered box is never placed.
            name: "a box beneath the surface raised over it by z-index: below re-rendered",
            parts: vec![(HIGH_Z, Some(red())), (UNDER_Z, None)],
            target: 1,
            step: raise,
            kind: "partial",
            below_kept: 0,
        },
        Case {
            // A removal restyles the whole document today, so the frame is
            // full; S4's removed-rect entry (never placed) is for the day it
            // is not.
            name: "the box under the surface removed: below re-rendered",
            parts: vec![(UNDER_FLOW, None), (ROUNDED, Some(red())), (BADGE, None)],
            target: 0,
            step: remove,
            kind: "",
            below_kept: 0,
        },
        Case {
            name: "the box under the surface recoloured: below re-rendered",
            parts: vec![(UNDER, None), (ROUNDED, Some(red())), (BADGE, None)],
            target: 0,
            step: recolor,
            kind: "partial",
            below_kept: 0,
        },
        Case {
            name: "a box later in the tree but painted first recoloured: below re-rendered",
            parts: vec![(HIGH_Z, Some(red())), (LOW_Z, None)],
            target: 1,
            step: recolor,
            kind: "partial",
            below_kept: 0,
        },
        Case {
            name: "a badge over the surface hidden: below re-rendered (it paints nowhere now)",
            parts: vec![(ROUNDED, Some(red())), (BADGE, None)],
            target: 1,
            step: hide,
            kind: "",
            below_kept: 0,
        },
    ];
    let mut failures = Vec::new();
    for case in cases {
        let target = case.target;
        let step = case.step;
        let act = move |p: &Page| step(&p.boxes.borrow()[target]);
        let mut split_page = page(&case.parts);
        let mut scene_page = page(&case.parts);
        if case.name.contains("label's text") {
            split_page.boxes.borrow()[1].set_text("12:59");
            scene_page.boxes.borrow()[1].set_text("12:59");
        }
        if case.name.contains("span") {
            split_page.boxes.borrow()[1].set_inner_html("ab <span>cdef</span> gh");
            scene_page.boxes.borrow()[1].set_inner_html("ab <span>cdef</span> gh");
        }
        let surfaces: Vec<usize> = split_page.surfaces.iter().map(|s| s.id()).collect();
        let mut win = Window::new(true);
        let mut reference = Window::new(false);
        for n in 0..3 {
            if n > 0 {
                act(&split_page);
                act(&scene_page);
            }
            let got = win.frame(&mut split_page.app, n == 0, Fault::None);
            let want = reference.frame(&mut scene_page.app, true, Fault::None);
            let name = case.name;
            if got.split_ids != surfaces {
                failures.push(format!("{name}, frame {n}: split {:?}", got.split_ids));
            }
            if n == 1 {
                if !case.kind.is_empty() && got.kind != case.kind {
                    failures.push(format!("{name}: a {} frame, want {}", got.kind, case.kind));
                }
                if got.below_kept != case.below_kept {
                    failures.push(format!(
                        "{name}: below kept over {} regions, want {}",
                        got.below_kept, case.below_kept
                    ));
                }
            }
            let wrong = compare(&got.pixels, &want.pixels, &win.allowances(EDGES), win.dev.0);
            if !wrong.is_empty() {
                failures.push(format!(
                    "{name}, frame {n} ({}): {} pixels past their allowance: {:?}",
                    got.kind,
                    wrong.len(),
                    &wrong[..wrong.len().min(6)]
                ));
            }
        }
        for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
            unregister_render_surface(s.id());
        }
    }
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The #1923 review's probes, adopted: DOM moves across the paint order,
// boxes with negatively z-indexed descendants (clipped, scrolled, moved,
// hidden, recoloured, resized), a span recoloured, a removal, z-index flips,
// static to relative, mixed inline content, `display: contents`; and a random
// driver over above-only and mixed changes around a kept surface.

/// Every frame of `steps` against in-scene; each frame's kind and
/// `split_below_kept` printed.
fn s4_probe(name: &str, parts: Vec<Part>, steps: &[&dyn Fn(&Page)]) -> Vec<String> {
    let mut failures = Vec::new();
    let mut split_page = page(&parts);
    let mut scene_page = page(&parts);
    let mut win = Window::new(true);
    let mut reference = Window::new(false);
    let noop = |_: &Page| {};
    let all: Vec<&dyn Fn(&Page)> = std::iter::once(&noop as &dyn Fn(&Page))
        .chain(steps.iter().copied())
        .collect();
    let mut prev: Option<Vec<[u8; 4]>> = None;
    for (n, step) in all.into_iter().enumerate() {
        step(&split_page);
        step(&scene_page);
        let got = win.frame(&mut split_page.app, n == 0, Fault::None);
        let want = reference.frame(&mut scene_page.app, true, Fault::None);
        let wrong = compare(&got.pixels, &want.pixels, &win.allowances(EDGES), win.dev.0);
        let (mut ch, mut ch_r) = (0, 0);
        if let Some(p) = &prev {
            for (i, (a, b)) in p.iter().zip(&want.pixels).enumerate() {
                if a != b {
                    ch += 1;
                    let (x, y) = (i as u32 % W, i as u32 / W);
                    if (50..170).contains(&x) && (40..120).contains(&y) {
                        ch_r += 1;
                    }
                }
            }
        }
        prev = Some(want.pixels.clone());
        eprintln!(
            "[rv1923] {name}, frame {n}: ref changed {ch} px ({ch_r} in R); {} frame, split {:?}, below_kept {}, {} wrong {:?}",
            got.kind,
            got.split_ids,
            got.below_kept,
            wrong.len(),
            &wrong[..wrong.len().min(3)]
        );
        if !wrong.is_empty() {
            failures.push(format!(
                "{name}, frame {n} ({}, below_kept {}): {} pixels past their allowance: {:?}",
                got.kind,
                got.below_kept,
                wrong.len(),
                &wrong[..wrong.len().min(6)]
            ));
        }
    }
    for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
        unregister_render_surface(s.id());
    }
    failures
}

const RV_ROUNDED: &str = "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                          box-sizing: border-box; border-radius: 14px";

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_s4_probes() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let only: Option<String> = std::env::var("RV_ONLY").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| n.contains(o));
    let mut failures = Vec::new();

    // 1. A translucent box before the surface in the tree moved (DOM) after
    // it, same geometry, across the surface's rounded corner.
    let to_end = |i: usize| {
        move |p: &Page| {
            let b = p.boxes.borrow()[i].clone();
            let root = b.parent_node().unwrap();
            root.append_child(&b);
        }
    };
    let corner_translucent = "position: absolute; left: 150px; top: 100px; width: 40px; \
                              height: 40px; background: rgba(0, 0, 255, 0.5)";
    if want("dom-move-after") {
        failures.extend(s4_probe(
            "dom-move-after",
            vec![
                (corner_translucent, None),
                (RV_ROUNDED, Some(red())),
                (BADGE, None),
            ],
            &[&to_end(0)],
        ));
    }
    // 2. The same, opaque, moved away in the same step.
    let corner_opaque = "position: absolute; left: 150px; top: 100px; width: 40px; \
                         height: 40px; background: rgb(0, 200, 255)";
    if want("dom-move-away") {
        let mv = move |p: &Page| {
            to_end(0)(p);
            p.boxes.borrow()[0].set_style("left", "250px");
        };
        failures.extend(s4_probe(
            "dom-move-away",
            vec![
                (corner_opaque, None),
                (RV_ROUNDED, Some(red())),
                (BADGE, None),
            ],
            &[&mv],
        ));
    }
    // 3. Control: after -> before.
    if want("dom-move-before") {
        let mv = |p: &Page| {
            let b = p.boxes.borrow()[1].clone();
            let a = p.boxes.borrow()[0].clone();
            b.parent_node().unwrap().insert_before(&b, &a);
        };
        failures.extend(s4_probe(
            "dom-move-before",
            vec![
                (RV_ROUNDED, Some(red())),
                (corner_translucent, None),
                (BADGE, None),
            ],
            &[&mv],
        ));
    }
    // 4. A positioned box after the surface whose negative-z child paints
    // under the surface's corner; the box starts clipping.
    let neg = "wrap:position: absolute; left: 140px; top: 95px; width: 20px; height: 15px\
               |position: relative; z-index: -1; width: 60px; height: 50px; \
               background: rgb(40, 200, 90)";
    let parent_style = |k: &'static str, v: &'static str| {
        move |p: &Page| {
            let c = p.boxes.borrow()[1].clone();
            c.parent_node().unwrap().set_style(k, v);
        }
    };
    // The page's root a stacking context, so a negative z paints over its
    // background (and under the surface).
    let root_ctx = |p: &Page| {
        let s = p.boxes.borrow()[0].clone();
        s.parent_node().unwrap().set_style("z-index", "0");
    };
    if want("neg-z-inside-clip") {
        let n2 = "wrap:position: absolute; left: 150px; top: 100px; width: 10px; height: 10px\
                  |position: relative; z-index: -1; width: 20px; height: 20px; \
                  background: rgb(40, 200, 90)";
        failures.extend(s4_probe(
            "neg-z-inside-clip",
            vec![(RV_ROUNDED, Some(red())), (n2, None), (BADGE, None)],
            &[&root_ctx, &parent_style("overflow", "hidden")],
        ));
        failures.extend(s4_probe(
            "neg-z-inside-radius",
            vec![(RV_ROUNDED, Some(red())), (n2, None), (BADGE, None)],
            &[
                &root_ctx,
                &parent_style("overflow", "hidden"),
                &parent_style("border-radius", "5px"),
            ],
        ));
    }
    if want("neg-z-move") {
        let n4 = "wrap:position: absolute; left: 150px; top: 100px; width: 10px; height: 10px\
                  |position: relative; z-index: -1; width: 20px; height: 20px; \
                  background: rgb(40, 200, 90)";
        failures.extend(s4_probe(
            "neg-z-move",
            vec![(RV_ROUNDED, Some(red())), (n4, None), (BADGE, None)],
            &[&root_ctx, &parent_style("left", "140px")],
        ));
    }
    if want("span-recolour") {
        let lab = "position: absolute; left: 70px; top: 60px; width: 80px; height: 16px; \
                   font: 12px/16px sans-serif; color: white";
        let init = |p: &Page| {
            p.boxes.borrow()[1].set_inner_html("ab <span>cdef</span> gh");
        };
        let recolour = |p: &Page| {
            let l = p.boxes.borrow()[1].clone();
            l.children()[1].set_style("color", "rgb(255, 255, 0)");
        };
        failures.extend(s4_probe(
            "span-recolour",
            vec![(RV_ROUNDED, Some(red())), (lab, None)],
            &[&init, &recolour],
        ));
    }
    if want("remove-badge") {
        let rm = |p: &Page| {
            p.boxes.borrow()[1].remove();
        };
        failures.extend(s4_probe(
            "remove-badge",
            vec![(RV_ROUNDED, Some(red())), (BADGE, None), (OPAQUE_BAR, None)],
            &[&rm],
        ));
    }
    if want("neg-z-scroll") {
        let n3 = "wrap:position: absolute; left: 150px; top: 95px; width: 20px; height: 25px; \
                  overflow: hidden\
                  |position: relative; z-index: -1; width: 20px; height: 30px; \
                  border-top: 30px solid rgb(40, 200, 90); background: rgb(0, 0, 255)";
        let scroll = |p: &Page| {
            let c = p.boxes.borrow()[1].clone();
            c.parent_node().unwrap().set_scroll_top(20.0);
        };
        failures.extend(s4_probe(
            "neg-z-scroll",
            vec![(RV_ROUNDED, Some(red())), (n3, None), (BADGE, None)],
            &[&root_ctx, &scroll],
        ));
    }
    // #1929: a clip change on the box does not damage its hoisted child's
    // overflow outside it, with nothing split either; the case stays out
    // until that is in (`S4_PROBE_1929=1` runs it).
    if want("neg-z-clip") && std::env::var("S4_PROBE_1929").is_ok() {
        failures.extend(s4_probe(
            "neg-z-clip",
            vec![(RV_ROUNDED, Some(red())), (neg, None), (BADGE, None)],
            &[&root_ctx, &parent_style("overflow", "hidden")],
        ));
    }
    if want("neg-z-visibility") {
        failures.extend(s4_probe(
            "neg-z-visibility",
            vec![(RV_ROUNDED, Some(red())), (neg, None), (BADGE, None)],
            &[&root_ctx, &parent_style("visibility", "hidden")],
        ));
    }
    if want("neg-z-recolour-parent") {
        failures.extend(s4_probe(
            "neg-z-recolour-parent",
            vec![(RV_ROUNDED, Some(red())), (neg, None), (BADGE, None)],
            &[&root_ctx, &parent_style("background", "rgb(250, 250, 0)")],
        ));
    }
    if want("neg-z-parent-resize") {
        // The child sized from its parent: a percentage width.
        let neg_pct = "wrap:position: absolute; left: 140px; top: 95px; width: 20px; height: 15px\
                       |position: relative; z-index: -1; width: 300%; height: 50px; \
                       background: rgb(40, 200, 90)";
        failures.extend(s4_probe(
            "neg-z-parent-resize",
            vec![(RV_ROUNDED, Some(red())), (neg_pct, None), (BADGE, None)],
            &[&root_ctx, &parent_style("width", "8px")],
        ));
    }
    // 5. z-index flips on a box over the corner.
    if want("z-flip") {
        let zb = "position: absolute; left: 150px; top: 100px; width: 40px; height: 40px; \
                  background: rgba(0, 0, 255, 0.5); z-index: 3";
        let s = "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                 box-sizing: border-box; border-radius: 14px; z-index: 2";
        failures.extend(s4_probe(
            "z-flip",
            vec![(s, Some(red())), (zb, None)],
            &[
                &|p: &Page| p.boxes.borrow()[1].set_style("z-index", "1"),
                &|p: &Page| p.boxes.borrow()[1].set_style("z-index", "3"),
            ],
        ));
    }
    // 6. Static -> relative: a later static block paints under an absolute
    // surface until it is positioned.
    if want("static-to-relative") {
        let st = "margin-left: 150px; margin-top: 100px; width: 40px; height: 40px; \
                  background: rgba(0, 0, 255, 0.5)";
        failures.extend(s4_probe(
            "static-to-relative",
            vec![(RV_ROUNDED, Some(red())), (st, None)],
            &[&|p: &Page| p.boxes.borrow()[1].set_style("position", "relative")],
        ));
    }
    // 7. Mixed inline content: the IFC root after the surface, a relative
    // negative-z span inside, text changed.
    if want("ifc-span") {
        let lab = "position: absolute; left: 140px; top: 100px; width: 80px; height: 20px; \
                   font: 14px/16px sans-serif; color: white";
        let step = |p: &Page| {
            let b = p.boxes.borrow()[1].clone();
            b.set_inner_html(
                "a<span style=\"position: relative; z-index: -1; background: rgb(0,180,0)\">WWWW</span>",
            );
        };
        let step2 = |p: &Page| {
            let b = p.boxes.borrow()[1].clone();
            b.set_inner_html(
                "a<span style=\"position: relative; z-index: -1; background: rgb(0,180,0)\">MMMMMM</span>",
            );
        };
        failures.extend(s4_probe(
            "ifc-span",
            vec![(RV_ROUNDED, Some(red())), (lab, None)],
            &[&step, &step2],
        ));
    }
    // 8. display: contents wrapper after the surface, its child over the corner.
    if want("contents") {
        let dc = "wrap:display: contents; color: white|position: absolute; left: 150px; top: 100px; \
                  width: 40px; height: 40px; background: rgba(0, 0, 255, 0.5)";
        failures.extend(s4_probe(
            "contents",
            vec![(RV_ROUNDED, Some(red())), (dc, None)],
            &[
                &parent_style_any(1, "display", "block"),
                &parent_style_any(1, "display", "contents"),
            ],
        ));
    }
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn parent_style_any(i: usize, k: &'static str, v: &'static str) -> impl Fn(&Page) {
    move |p: &Page| {
        let c = p.boxes.borrow()[i].clone();
        c.parent_node().unwrap().set_style(k, v);
    }
}

/// Random above-only and mixed changes around a kept surface, every frame
/// against in-scene (review of #1923). `RV_NEGZ=0` leaves out the
/// negative-z child's container ops.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_runtime_review_s4_random() {
    let _p = presenting_lock();
    test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let seeds: Vec<u64> = std::env::var("PROBE_SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect())
        .unwrap_or_else(|| (1..=10).collect());
    let steps_n: usize = std::env::var("PROBE_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14);
    let negz = std::env::var("RV_NEGZ").map_or(true, |v| v != "0");
    let mut failures = Vec::new();
    let mut kept_total = 0u64;
    for seed in seeds {
        let rng = Rc::new(Cell::new(
            seed.wrapping_mul(6364136223846793005).wrapping_add(7),
        ));
        let next = {
            let rng = rng.clone();
            move |n: u32| -> u32 {
                let x = rng
                    .get()
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                rng.set(x);
                ((x >> 33) as u32) % n
            }
        };
        let color = red();
        let mut ops: Vec<Step> = vec![Box::new(|p: &Page| {
            let s = p.boxes.borrow()[1].clone();
            s.parent_node().unwrap().set_style("z-index", "0");
        })];
        let mut log = Vec::new();
        for _ in 0..steps_n {
            let kind = next(if negz { 11 } else { 9 });
            let x = next(200) as i32 + 20;
            let y = next(120) as i32 + 20;
            let c = [next(256) as u8, next(256) as u8, next(256) as u8];
            log.push(format!("{kind}:{x}:{y}"));
            let color = color.clone();
            ops.push(match kind {
                // Badge (box 2) recoloured, translucent sometimes.
                0 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[2].set_style(
                        "background",
                        &format!(
                            "rgba({}, {}, {}, {})",
                            c[0],
                            c[1],
                            c[2],
                            if c[0] > 128 { 1.0 } else { 0.5 }
                        ),
                    );
                }),
                // Badge moved.
                1 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[2].set_style("left", &format!("{x}px"));
                    p.boxes.borrow()[2].set_style("top", &format!("{y}px"));
                }),
                // Label text.
                2 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[3].set_text(&format!("{}:{:02}", c[0] % 24, c[1] % 60));
                }),
                // Under-box (box 0) recoloured.
                3 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[0]
                        .set_style("background", &format!("rgb({}, {}, {})", c[0], c[1], c[2]));
                }),
                // Badge z-index flips (surface is z auto: 0 above, -1 below).
                4 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[2].set_style("z-index", if c[2] > 128 { "-1" } else { "1" });
                }),
                // Badge visibility.
                5 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[2]
                        .set_style("visibility", if c[1] > 128 { "hidden" } else { "visible" });
                }),
                // Surface redraw.
                6 => Box::new(move |p: &Page| {
                    color.set([c[0], c[1], c[2], 255]);
                    for s in &p.surfaces {
                        s.request_frame();
                    }
                }),
                // Label's colour (inherited by its text).
                7 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[3]
                        .set_style("color", &format!("rgb({}, {}, {})", c[0], c[1], c[2]));
                }),
                // Badge shadow.
                8 => Box::new(move |p: &Page| {
                    p.boxes.borrow()[2]
                        .set_style("box-shadow", &format!("0 0 {}px rgb(0, 0, 0)", c[1] % 12));
                }),
                // The negative-z child's container: clip toggled.
                9 => Box::new(move |p: &Page| {
                    let n = p.boxes.borrow()[4].clone();
                    n.parent_node()
                        .unwrap()
                        .set_style("overflow", if c[0] > 128 { "hidden" } else { "visible" });
                }),
                // The negative-z child's container: recoloured.
                _ => Box::new(move |p: &Page| {
                    let n = p.boxes.borrow()[4].clone();
                    n.parent_node().unwrap().set_style(
                        "background",
                        &format!("rgba({}, {}, {}, 0.5)", c[0], c[1], c[2]),
                    );
                }),
            });
        }
        eprintln!("[rv1923r] seed {seed}: {log:?}");
        let parts: Vec<Part> = vec![
            (
                "position: absolute; left: 40px; top: 30px; width: 60px; height: 40px; \
                 background: rgb(200, 60, 160)",
                None,
            ),
            (RV_ROUNDED, Some(color.clone())),
            (BADGE, None),
            (
                "position: absolute; left: 70px; top: 60px; width: 60px; height: 16px; \
                 color: white; font: 12px/16px sans-serif",
                None,
            ),
            (
                "wrap:position: absolute; left: 150px; top: 100px; width: 10px; height: 10px\
                 |position: relative; z-index: -1; width: 20px; height: 20px; \
                 background: rgb(40, 200, 90)",
                None,
            ),
        ];
        let mut split_page = page(&parts);
        let mut scene_page = page(&parts);
        split_page.boxes.borrow()[3].set_text("12:00");
        scene_page.boxes.borrow()[3].set_text("12:00");
        let mut win = Window::new(true);
        let mut reference = Window::new(false);
        let noop: Step = Box::new(|_: &Page| {});
        for (n, step) in std::iter::once(&noop).chain(ops.iter()).enumerate() {
            step(&split_page);
            step(&scene_page);
            let got = win.frame(&mut split_page.app, n == 0, Fault::None);
            let want = reference.frame(&mut scene_page.app, true, Fault::None);
            kept_total += got.below_kept;
            let wrong = compare(&got.pixels, &want.pixels, &win.allowances(EDGES), win.dev.0);
            eprintln!(
                "[rv1923r] seed {seed}, frame {n} ({}): {} frame, below_kept {}, {} wrong",
                if n >= 2 { &log[n - 2] } else { "-" },
                got.kind,
                got.below_kept,
                wrong.len()
            );
            if !wrong.is_empty() {
                failures.push(format!(
                    "seed {seed}, frame {n} ({}, below_kept {}): {} wrong: {:?}",
                    got.kind,
                    got.below_kept,
                    wrong.len(),
                    &wrong[..wrong.len().min(4)]
                ));
            }
        }
        for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
            unregister_render_surface(s.id());
        }
    }
    eprintln!("[rv1923r] below kept over {kept_total} regions in all");
    crate::shell::renderer::set_gpu_presenting(false);
    assert!(kept_total > 0, "no frame kept below: S4 never ran");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// #549 on the split path: a `position: fixed` box escapes the clips above the
// stacking context that owns it, and a GPU surface beside it or inside it is
// still split. Paint pops those clips around the fixed entry and pushes them
// back after it; the split painter mirrors every push and pop, so the clips
// the split recorded as open around the surface are the ones that are.
#[cfg(test)]
mod fixed_escape_549 {
    use super::review_b::custom_page;
    use super::*;

    const GREEN: [u8; 4] = [0, 200, 0, 255];

    /// `clipper > owner(z-index: 1) > …`, the clipper rounded and cutting
    /// both the surface's region and the fixed box.
    fn make(surface_in_fixed: bool) -> Page {
        custom_page(
            &[red()],
            Rc::new(move |scope, root, ss, kept| {
                let clipper = scope.create_element("div");
                clipper.set_attribute(
                    "style",
                    "position: absolute; left: 20px; top: 20px; width: 200px; height: 150px; \
                     overflow: hidden; border-radius: 12px",
                );
                let owner = scope.create_element("div");
                owner.set_attribute(
                    "style",
                    "position: relative; z-index: 1; width: 180px; height: 130px",
                );
                let n = RenderSurface {
                    surface: Some(ss[0].clone()),
                }
                .render(scope, &[]);
                let fixed = scope.create_element("div");
                if surface_in_fixed {
                    // The surface is inside the fixed box, and reaches past
                    // the clipper with it.
                    fixed.set_attribute(
                        "style",
                        "position: fixed; left: 120px; top: 80px; width: 200px; height: 150px; \
                         background: rgb(0, 200, 0)",
                    );
                    n.set_attribute(
                        "style",
                        "position: absolute; left: 20px; top: 20px; width: 140px; height: 90px",
                    );
                    fixed.append_child(&n);
                    owner.append_child(&fixed);
                } else {
                    // The fixed box is painted over the surface's region and
                    // reaches past the clipper.
                    n.set_attribute(
                        "style",
                        "position: absolute; left: 10px; top: 10px; width: 120px; height: 80px",
                    );
                    fixed.set_attribute(
                        "style",
                        "position: fixed; left: 100px; top: 60px; width: 200px; height: 60px; \
                         background: rgb(0, 200, 0)",
                    );
                    owner.append_child(&n);
                    owner.append_child(&fixed);
                }
                clipper.append_child(&owner);
                root.append_child(&clipper);
                kept.borrow_mut().push(fixed);
            }),
        )
    }

    /// Both shapes, split, against in-scene, over a full frame and a partial
    /// one that recolours the fixed box; the fixed box (and in the second
    /// shape the surface in it) drawn past the clipper.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_split_runtime_a_fixed_box_escapes_a_clip_around_a_split_surface() {
        let _p = presenting_lock();
        test_gpu();
        crate::shell::renderer::set_gpu_presenting(true);
        let mut failures = Vec::new();
        for (surface_in_fixed, past) in [(false, (280, 90)), (true, (250, 150))] {
            let name = if surface_in_fixed {
                "a surface in the fixed box"
            } else {
                "a fixed box over the surface"
            };
            let (mut split_page, mut scene_page) = (make(surface_in_fixed), make(surface_in_fixed));
            let surfaces: Vec<usize> = split_page.surfaces.iter().map(|s| s.id()).collect();
            let mut win = Window::new(true);
            let mut reference = Window::new(false);
            let recolor = |p: &Page| p.boxes.borrow()[0].set_style("background", "rgb(0, 160, 0)");
            for n in 0..2 {
                if n == 1 {
                    recolor(&split_page);
                    recolor(&scene_page);
                }
                let got = win.frame(&mut split_page.app, n == 0, Fault::None);
                let want = reference.frame(&mut scene_page.app, true, Fault::None);
                if got.split_ids != surfaces {
                    failures.push(format!(
                        "{name}, frame {n}: split {:?}, want {surfaces:?}",
                        got.split_ids
                    ));
                }
                let at = |px: &[[u8; 4]], (x, y): (u32, u32)| px[(y * W + x) as usize];
                let drawn = at(&want.pixels, past);
                let expect = if surface_in_fixed {
                    red().get()
                } else if n == 0 {
                    GREEN
                } else {
                    [0, 160, 0, 255]
                };
                if drawn != expect {
                    failures.push(format!(
                        "{name}, frame {n}: in-scene {drawn:?} at {past:?} past the clipper, \
                         want {expect:?}"
                    ));
                }
                let wrong = compare(&got.pixels, &want.pixels, &win.allowances(EDGES), win.dev.0);
                if !wrong.is_empty() {
                    failures.push(format!(
                        "{name}, frame {n} ({}): {} pixels past their allowance: {:?}",
                        got.kind,
                        wrong.len(),
                        &wrong[..wrong.len().min(6)]
                    ));
                }
            }
            for s in split_page.surfaces.iter().chain(&scene_page.surfaces) {
                unregister_render_surface(s.id());
            }
        }
        crate::shell::renderer::set_gpu_presenting(false);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
