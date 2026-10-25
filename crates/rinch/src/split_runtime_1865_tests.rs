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
        format: TextureFormat::Rgba8Unorm,
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
    let data = buffer.slice(..).get_mapped_range();
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
}

/// A frame: what it presented, which surfaces it composited split, and the
/// frame's kind (`"full"`, `"partial"`, `"unchanged"`).
struct Shown {
    pixels: Vec<[u8; 4]>,
    split_ids: Vec<usize>,
    kind: &'static str,
}

impl Window {
    fn new(splits: bool) -> Self {
        Self {
            vello: vello_renderer(),
            registry: SurfaceImages::default(),
            ui: texture(
                (W, H),
                TextureUsages::STORAGE_BINDING
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC
                    | TextureUsages::COPY_DST,
            ),
            partial: None,
            split: SplitFrame::default(),
            compositor: SplitCompositor::new(&test_gpu().device, TextureFormat::Rgba8Unorm),
            target: texture(
                (W, H),
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            ),
            splits,
        }
    }

    fn frame(&mut self, app: &mut RinchApp, full: bool, fault: Fault) -> Shown {
        let gpu = test_gpu();
        paint_like_paint_gpu(app, 1.0);
        let frames = collect_gpu_surface_frames();
        let split_ids: Vec<usize> = self.split.surface_ids().collect();
        let fresh: Vec<usize> = frames
            .iter()
            .filter(|(id, _, fresh)| *fresh && !split_ids.contains(id))
            .map(|(id, ..)| *id)
            .collect();
        let images = self.registry.sync(
            &mut self.vello,
            frames.into_iter().map(|(id, texture, _)| (id, texture)),
        );
        if !fresh.is_empty() {
            app.request_repaint();
            app.mark_surface_nodes_paint_dirty(&fresh);
        }
        rinch_dom::paint::set_surface_gpu_images(Some(images));
        app.install_gpu_viewport_frames(
            collect_viewport_frames_by_name(),
            Some(Default::default()),
        );
        app.set_split_enabled(self.splits);
        let target = full.then_some(rinch_dom::perf::FullRepaintReason::FirstFrame);
        let (frame, change) = app.build_gpu_frame_split(1.0, (W, H), target);
        let base = vello::peniko::Color::WHITE;
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
                    .render_to_texture(&gpu.device, &gpu.queue, scene, &ui_view, &params(W, H))
                    .expect("render");
                "full"
            }
            GpuFrame::Partial { scene, size, rects } => {
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
        if let Some(change) = change
            && fault != Fault::StaleSlices
        {
            self.split
                .apply(&gpu.device, &gpu.queue, &mut self.vello, change)
                .expect("apply");
        }
        let split_ids: Vec<usize> = self.split.surface_ids().collect();
        let pixels = if self.split.is_empty() || fault == Fault::NoComposite {
            read(&self.ui)
        } else {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            self.split.composite(
                &self.compositor,
                &gpu.device,
                &mut encoder,
                &ui_view,
                &self.target.create_view(&Default::default()),
                (W, H),
                &self.registry.textures(),
            );
            gpu.queue.submit(Some(encoder.finish()));
            read(&self.target)
        };
        Shown {
            pixels,
            split_ids,
            kind,
        }
    }

    /// The allowance at each pixel (D10, from this window's own slices).
    fn allowances(&self, edges: bool) -> Vec<i32> {
        let mut allow = vec![0; (W * H) as usize];
        let fractional = |a: u8| a != 0 && a != 255;
        for ((rx, ry, rw, rh), textures) in self.split.slices() {
            let read: Vec<Option<Vec<[u8; 4]>>> = textures.iter().map(|t| t.map(read)).collect();
            for y in 0..H {
                for x in 0..W {
                    let i = (y * W + x) as usize;
                    let inside = x >= rx && x < rx + rw && y >= ry && y < ry + rh;
                    if inside {
                        let local = ((y - ry) * rw + (x - rx)) as usize;
                        if read.iter().flatten().any(|px| fractional(px[local][3])) {
                            allow[i] = allow[i].max(1);
                        }
                    } else if edges
                        && x + 16 >= rx
                        && x < rx + rw + 16
                        && y + 16 >= ry
                        && y < ry + rh + 16
                    {
                        allow[i] = allow[i].max(1);
                    }
                }
            }
        }
        allow
    }
}

/// A pixel past its allowance: `(x, y, split, in-scene, allowance)`.
type Violation = (u32, u32, [u8; 4], [u8; 4], i32);

/// The pixels past their allowance.
fn compare(got: &[[u8; 4]], want: &[[u8; 4]], allow: &[i32]) -> Vec<Violation> {
    let mut out = Vec::new();
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let off = (0..4)
            .map(|c| (g[c] as i32 - w[c] as i32).abs())
            .max()
            .unwrap();
        if off > allow[i] {
            out.push((i as u32 % W, i as u32 / W, *g, *w, allow[i]));
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
}

/// A part of a page: its style, and the colour a surface draws (`None`: a
/// plain box).
type Part = (&'static str, Option<Rc<Cell<[u8; 4]>>>);

fn page(spec: &[Part]) -> Page {
    let parts: Vec<(&'static str, Option<RenderSurfaceHandle>)> = spec
        .iter()
        .map(|(style, color)| (*style, color.clone().map(surface)))
        .collect();
    let surfaces: Vec<RenderSurfaceHandle> = parts.iter().filter_map(|(_, s)| s.clone()).collect();
    let boxes: Rc<RefCell<Vec<NodeHandle>>> = Default::default();
    let kept = boxes.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
             background: rgb(60, 70, 90)",
        );
        for (style, surface) in &parts {
            let (wrap, style) = match style.strip_prefix("wrap:") {
                Some(rest) => {
                    let (w, s) = rest.split_once('|').unwrap();
                    (Some(w), s)
                }
                None => (None, *style),
            };
            let node = match surface {
                Some(s) => {
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
                None => {
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
    }
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
const SIDEBAR: &str = "position: absolute; left: 320px; top: 0; width: 80px; height: 300px; \
                       background: rgb(0, 0, 128)";

/// One page, split against in-scene, over the frames `steps` makes. `split`
/// says whether the surfaces must end up split (or must not).
fn check(
    name: &str,
    parts: Vec<Part>,
    edges: bool,
    split: bool,
    steps: &[&dyn Fn(&Page)],
    fault: Fault,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut split_page = page(&parts);
    let mut scene_page = page(&parts);
    let surfaces: Vec<usize> = split_page.surfaces.iter().map(|s| s.id()).collect();
    let mut win = Window::new(true);
    let mut reference = Window::new(false);
    let noop = |_: &Page| {};
    let all: Vec<&dyn Fn(&Page)> = std::iter::once(&noop as &dyn Fn(&Page))
        .chain(steps.iter().copied())
        .collect();
    for (n, step) in all.into_iter().enumerate() {
        step(&split_page);
        step(&scene_page);
        let got = win.frame(&mut split_page.app, n == 0, fault);
        let want = reference.frame(&mut scene_page.app, true, Fault::None);
        if split && got.split_ids.len() != surfaces.len() {
            failures.push(format!(
                "{name}, frame {n}: split {:?}, want all of {surfaces:?}",
                got.split_ids
            ));
        }
        if !split && !got.split_ids.is_empty() {
            failures.push(format!("{name}, frame {n}: split {:?}", got.split_ids));
        }
        let wrong = compare(&got.pixels, &want.pixels, &win.allowances(edges));
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
            false,
            true,
            &[],
            Fault::None,
        ),
        check(
            "border, padding, own radius",
            vec![s(
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                    box-sizing: border-box; border: 5px solid rgb(0, 128, 0); padding: 10px; \
                    border-radius: 20px; background: rgb(0, 0, 255)",
            )],
            false,
            true,
            &[],
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
            false,
            true,
            &[],
            Fault::None,
        ),
        check(
            "an opaque bar across it",
            vec![s(SURFACE), (OPAQUE_BAR, None)],
            false,
            true,
            &[],
            Fault::None,
        ),
        check(
            "a translucent bar across it",
            vec![s(SURFACE), (TRANSLUCENT_BAR, None)],
            true,
            true,
            &[],
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
            false,
            true,
            &[],
            Fault::None,
        ),
        check(
            "two surfaces apart",
            vec![
                s(SURFACE),
                s("position: absolute; left: 200px; top: 150px; width: 90px; height: 60px"),
            ],
            false,
            true,
            &[],
            Fault::None,
        ),
        check(
            "rotated: in the scene",
            vec![s(
                "position: absolute; left: 50px; top: 40px; width: 120px; height: 80px; \
                    transform: rotate(7deg)",
            )],
            false,
            false,
            &[],
            Fault::None,
        ),
        check(
            "in an opacity group: in the scene",
            vec![(
                "wrap:position: absolute; left: 0; top: 0; opacity: 0.6|position: absolute; \
                 left: 50px; top: 40px; width: 120px; height: 80px",
                Some(red()),
            )],
            false,
            false,
            &[],
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
            ],
            false,
            true,
            &[&recolor, &redraw, &elsewhere],
            Fault::None,
        ),
        check(
            "the surface hidden",
            vec![(SURFACE, Some(red())), (OPAQUE_BAR, None), (SIDEBAR, None)],
            false,
            false,
            &[&hide],
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
            false,
            true,
            &[&recolor],
            fault,
        );
        assert!(
            !failures.is_empty(),
            "control ({what}): no failure; the oracle cannot see it"
        );
    }
    crate::shell::renderer::set_gpu_presenting(false);
}
