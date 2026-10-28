//! #1718: the desktop GPU path repaints only a frame's damage.
//!
//! The first half needs no GPU: what [`RinchApp::build_gpu_frame`] hands the
//! renderer. The second half is `#[ignore]`d, since it needs an adapter:
//! `cargo test -p rinch --features gpu --release --lib gpu_partial_frame -- --ignored --nocapture`
//! — the pixel identity of a partial frame with a full one, and the
//! measurement the issue's numbers came from (`gpu_hover_probe`).
use super::*;
use rinch_components::{Tree, TreeNodeData, UseTreeOptions, UseTreeReturn};
use rinch_core::Component;
use std::time::Instant;

const W: u32 = 1200;
const H: u32 = 800;

fn app() -> RinchApp {
    let mut data = Vec::new();
    let mut open = std::collections::HashSet::new();
    for i in 0..17 {
        let id = format!("p{i}");
        open.insert(id.clone());
        data.push(
            TreeNodeData::new(&id, format!("Folder {i}")).with_children(
                (0..9)
                    .map(|j| TreeNodeData::new(format!("c{i}-{j}"), format!("Item {i}.{j}")))
                    .collect(),
            ),
        );
    }
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: open,
        ..Default::default()
    });
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let tree = Tree {
            data,
            tree: Some(state),
            ..Default::default()
        }
        .render(scope, &[]);
        // The tree in its scroller, and beside it a long document: what the
        // issue's app shows, and what a hover over the tree must not encode.
        let list = scope.create_element("div");
        list.set_attribute(
            "style",
            "width: 400px; height: 800px; overflow: auto; flex: none",
        );
        list.append_child(&tree);
        let doc = scope.create_element("div");
        doc.set_attribute(
            "style",
            "flex: 1; height: 800px; overflow: auto; font: 16px/24px sans-serif",
        );
        for i in 0..200 {
            let p = scope.create_element("p");
            p.append_child(&scope.create_text(&format!(
                "Paragraph {i}. The quick brown fox jumps over the lazy dog, \
                 and the document beside the tree goes on for a while longer \
                 than the window shows."
            )));
            doc.append_child(&p);
        }
        let root = scope.create_element("div");
        root.set_attribute("style", "display: flex; height: 800px");
        root.append_child(&list);
        root.append_child(&doc);
        root
    });
    app.mount_component(W as f32, H as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

fn hover(app: &mut RinchApp, y: f32) {
    app.handle_event(PlatformEvent::MouseMove { x: 50.0, y }, (W, H), 1.0);
    app.handle_event(PlatformEvent::AboutToWait, (W, H), 1.0);
    if app.has_pending_layout() {
        app.resolve_and_repaint(W as f32, H as f32);
    }
}

/// The rects a partial frame copies, or a panic naming what it got.
/// How much a scene asks Vello to draw: its draw objects and its glyphs.
fn weight(scene: &vello::Scene) -> usize {
    let e = scene.encoding();
    e.draw_tags.len() + e.resources.glyphs.len()
}

type Rects = Vec<(u32, u32, u32, u32)>;

fn partial(frame: GpuFrame<'_>) -> (usize, (u32, u32), Rects) {
    match frame {
        GpuFrame::Partial { scene, size, rects } => (weight(scene), size, rects),
        GpuFrame::Full(_) => panic!("a full frame"),
        GpuFrame::Unchanged => panic!("an unchanged frame"),
    }
}

/// Mount, render the first frame in full, and hover the row at `y = 100`.
fn settled() -> RinchApp {
    let mut app = app();
    assert!(matches!(
        app.build_gpu_frame(
            1.0,
            (W, H),
            Some(rinch_dom::perf::FullRepaintReason::FirstFrame)
        ),
        GpuFrame::Full(_)
    ));
    hover(&mut app, 100.0);
    let _ = app.build_gpu_frame(1.0, (W, H), None);
    app.end_perf_frame();
    app
}

#[test]
fn a_hover_repaints_the_two_rows_it_moved_between_not_the_window() {
    let mut app = settled();
    let full_paths = {
        let mut fresh = self::app();
        weight(fresh.build_scene(1.0, (W, H)))
    };
    hover(&mut app, 300.0);
    let (paths, size, rects) = partial(app.build_gpu_frame(1.0, (W, H), None));
    let stats = app.end_perf_frame().unwrap();
    assert_eq!(stats.get(rinch_dom::perf::Counter::RepaintPartial), 1);
    assert_eq!(stats.get(rinch_dom::perf::Counter::RepaintFullGpu), 0);
    // Every rect is one of the two rows: inside the scroller's 400px, and
    // around y = 100 or y = 300.
    assert!(!rects.is_empty());
    for &(x, y, w, h) in &rects {
        assert!(x + w <= 400 && h <= 60, "{rects:?}");
        let mid = y + h / 2;
        assert!(
            mid.abs_diff(100) < 30 || mid.abs_diff(300) < 30,
            "{rects:?}"
        );
        // …and inside the target it is rendered into.
        assert!(x + w <= size.0 && y + h <= size.1, "{rects:?} {size:?}");
    }
    let px: u32 = rects.iter().map(|r| r.2 * r.3).sum();
    assert_eq!(
        stats.get(rinch_dom::perf::Counter::RepaintedPx),
        px as u64,
        "the copied rects are the damage"
    );
    // Only what the damage reaches is encoded.
    assert!(
        paths * 4 < full_paths,
        "a partial frame encoded {paths} draws and glyphs of the full frame's {full_paths}"
    );
}

#[test]
fn a_frame_with_nothing_changed_renders_nothing() {
    let mut app = settled();
    app.request_repaint();
    assert!(matches!(
        app.build_gpu_frame(1.0, (W, H), None),
        GpuFrame::Unchanged
    ));
    let stats = app.end_perf_frame().unwrap();
    assert_eq!(stats.get(rinch_dom::perf::Counter::RepaintNone), 1);
    assert!(matches!(
        app.build_gpu_frame(1.0, (W, H), None),
        GpuFrame::Unchanged
    ));
    let stats = app.end_perf_frame().unwrap();
    assert_eq!(stats.get(rinch_dom::perf::Counter::PaintCachedFrames), 1);
    assert_eq!(stats.get(rinch_dom::perf::Counter::PaintFrames), 0);
}

#[test]
fn a_target_that_lost_its_frame_is_repainted_in_full() {
    use rinch_dom::perf::{Counter, FullRepaintReason};
    let mut app = settled();
    // Nothing changed in the document, but the UI texture was resized.
    assert!(matches!(
        app.build_gpu_frame(1.0, (W, H), Some(FullRepaintReason::Resize)),
        GpuFrame::Full(_)
    ));
    assert_eq!(
        app.end_perf_frame()
            .unwrap()
            .get(Counter::RepaintFullResize),
        1
    );
    // A hover over a texture rendered over another base colour.
    hover(&mut app, 300.0);
    assert!(matches!(
        app.build_gpu_frame(1.0, (W, H), Some(FullRepaintReason::Invalidated)),
        GpuFrame::Full(_)
    ));
    assert_eq!(
        app.end_perf_frame()
            .unwrap()
            .get(Counter::RepaintFullInvalidated),
        1
    );
}

#[test]
fn build_scene_after_a_partial_frame_encodes_the_whole_document() {
    let mut app = settled();
    hover(&mut app, 300.0);
    let _ = partial(app.build_gpu_frame(1.0, (W, H), None));
    let full_paths = {
        let mut fresh = self::app();
        hover(&mut fresh, 300.0);
        weight(fresh.build_scene(1.0, (W, H)))
    };
    // Nothing is dirty, but the painter holds a pruned frame: the embed
    // path's cached scene must not be it.
    assert_eq!(weight(app.build_scene(1.0, (W, H))), full_paths);
}

#[test]
#[ignore]
fn gpu_hover_probe() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("adapter");
    eprintln!("adapter: {:?}", adapter.get_info().name);
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    let tex = device.create_texture(&wgpu::TextureDescriptor {
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
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let mut app = app();
    let rows = {
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        d.tree.nodes.len()
    };
    eprintln!("nodes: {rows}");
    let mut encode = Vec::new();
    let mut render = Vec::new();
    let mut submit = Vec::new();
    // Before #1718: every hover re-encodes and renders the whole window.
    for i in 0..200 {
        hover(&mut app, 10.0 + (i % 30) as f32 * 26.0);
        let t = Instant::now();
        let scene = app.build_scene(1.0, (W, H)).clone();
        let te = t.elapsed();
        let t = Instant::now();
        renderer
            .render_to_texture(
                &device,
                &queue,
                &scene,
                &view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::WHITE,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .unwrap();
        let ts = t.elapsed();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let tr = t.elapsed();
        let stats = app.end_perf_frame().unwrap();
        if i >= 20 {
            submit.push(ts.as_secs_f64() * 1e3);
            encode.push(te.as_secs_f64() * 1e3);
            render.push(tr.as_secs_f64() * 1e3);
        }
        if i == 20 {
            eprintln!("full frame: {}", super::perf_expect::baseline_text(&stats));
        }
    }
    // After: `build_gpu_frame` + `render_partial`, as the desktop shell does.
    let (mut p_encode, mut p_render) = (Vec::new(), Vec::new());
    let mut target = None;
    let mut u_render = Vec::new();
    let tex2 = device.create_texture(&wgpu::TextureDescriptor {
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
    let view2 = tex2.create_view(&Default::default());
    let _ = app.build_gpu_frame(
        1.0,
        (W, H),
        Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
    );
    app.end_perf_frame();
    for i in 0..200 {
        hover(&mut app, 10.0 + (i % 30) as f32 * 26.0);
        let t = Instant::now();
        let frame = app.build_gpu_frame(1.0, (W, H), None);
        let te = t.elapsed();
        let t = Instant::now();
        match frame {
            GpuFrame::Partial { scene, size, rects } => {
                crate::shell::desktop::render_partial(
                    &device,
                    &queue,
                    &mut renderer,
                    &tex,
                    &mut target,
                    crate::shell::desktop::PartialFrame {
                        scene,
                        size,
                        rects: &rects,
                        base: vello::peniko::Color::WHITE,
                    },
                )
                .unwrap();
            }
            GpuFrame::Full(_) => panic!("a full frame at hover {i}"),
            GpuFrame::Unchanged => {}
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let tr = t.elapsed();
        // The same pruned scene, untranslated, into a whole-window target.
        let t = Instant::now();
        renderer
            .render_to_texture(
                &device,
                &queue,
                app.painter.scene(),
                &view2,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::WHITE,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .unwrap();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let tu = t.elapsed();
        let stats = app.end_perf_frame().unwrap();
        if i >= 20 {
            p_encode.push(te.as_secs_f64() * 1e3);
            p_render.push(tr.as_secs_f64() * 1e3);
            u_render.push(tu.as_secs_f64() * 1e3);
        }
        if i == 20 {
            eprintln!(
                "partial frame: {}",
                super::perf_expect::baseline_text(&stats)
            );
        }
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    eprintln!(
        "full: encode median {:.3} ms, render(submit) median {:.3} ms, render+gpu wait median {:.3} ms",
        med(&mut encode),
        med(&mut submit),
        med(&mut render)
    );
    eprintln!(
        "pruned, untranslated, whole window: {:.3} ms",
        med(&mut u_render)
    );
    eprintln!(
        "partial: encode median {:.3} ms, render+copy+gpu wait median {:.3} ms",
        med(&mut p_encode),
        med(&mut p_render)
    );
    // A static repaint of the same scene, to see Vello's GPU time with no encode.
    let scene = app.build_scene(1.0, (W, H)).clone();
    let mut v = Vec::new();
    for aa in [vello::AaConfig::Msaa16, vello::AaConfig::Area] {
        v.clear();
        for _ in 0..30 {
            let t = Instant::now();
            renderer
                .render_to_texture(
                    &device,
                    &queue,
                    &scene,
                    &view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::WHITE,
                        width: W,
                        height: H,
                        antialiasing_method: aa,
                    },
                )
                .unwrap();
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            v.push(t.elapsed().as_secs_f64() * 1e3);
        }
        eprintln!("same scene {:?}: {:.3} ms", aa, med(&mut v));
    }
    // An empty scene: Vello's fixed per-frame cost at this size.
    let empty = vello::Scene::new();
    v.clear();
    for _ in 0..30 {
        let t = Instant::now();
        renderer
            .render_to_texture(
                &device,
                &queue,
                &empty,
                &view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::WHITE,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .unwrap();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        v.push(t.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!("empty scene: {:.3} ms", med(&mut v));
    for (sw, sh) in [(400u32, 64u32), (1200, 64), (64, 64)] {
        let small = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: sw,
                height: sh,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sv = small.create_view(&Default::default());
        let mut sub = vello::Scene::new();
        sub.append(&scene, Some(vello::kurbo::Affine::translate((0.0, -300.0))));
        for (name, sc) in [("empty", &empty), ("full scene, translated", &sub)] {
            v.clear();
            for _ in 0..30 {
                let t = Instant::now();
                renderer
                    .render_to_texture(
                        &device,
                        &queue,
                        sc,
                        &sv,
                        &vello::RenderParams {
                            base_color: vello::peniko::Color::WHITE,
                            width: sw,
                            height: sh,
                            antialiasing_method: vello::AaConfig::Msaa16,
                        },
                    )
                    .unwrap();
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                v.push(t.elapsed().as_secs_f64() * 1e3);
            }
            eprintln!("{sw}x{sh} {name}: {:.3} ms", med(&mut v));
        }
    }
    eprintln!(
        "scene encoding size: {} path segs",
        scene.encoding().n_path_segments
    );
}

fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let bpr = (w * 4).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bpr),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buf
        .slice(..)
        .get_mapped_range()
        .expect("the buffer is mapped");
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        out.extend_from_slice(&data[(y * bpr) as usize..(y * bpr + w * 4) as usize]);
    }
    out
}

/// A partial frame leaves the UI texture exactly as a full render of the same
/// state does, over a sweep of hovers that scrolls the list too (#1718).
#[test]
#[ignore]
fn gpu_partial_frames_match_a_full_render() {
    use crate::shell::desktop::{PartialFrame, render_partial};
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let new_renderer = || {
        vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .unwrap()
    };
    let mut renderer = new_renderer();
    let mk = |w, h| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let params = |w, h| vello::RenderParams {
        base_color: vello::peniko::Color::WHITE,
        width: w,
        height: h,
        antialiasing_method: vello::AaConfig::Msaa16,
    };
    let ui = mk(W, H);
    let reference = mk(W, H);
    let mut target = None;
    let mut app = app();
    let GpuFrame::Full(scene) = app.build_gpu_frame(
        1.0,
        (W, H),
        Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
    ) else {
        panic!("the first frame is full")
    };
    renderer
        .render_to_texture(
            &device,
            &queue,
            scene,
            &ui.create_view(&Default::default()),
            &params(W, H),
        )
        .unwrap();
    let mut partials = 0;
    for (i, y) in [100.0, 130.0, 300.0, 310.0, 620.0, 40.0, 780.0]
        .into_iter()
        .enumerate()
    {
        hover(&mut app, y);
        if i == 4 {
            // A wheel notch: the frame that scrolls is the scroller's box.
            app.handle_event(
                PlatformEvent::MouseWheel {
                    x: 50.0,
                    y,
                    delta_x: 0.0,
                    delta_y: -90.0,
                },
                (W, H),
                1.0,
            );
            app.handle_event(PlatformEvent::AboutToWait, (W, H), 1.0);
            if app.has_pending_layout() {
                app.resolve_and_repaint(W as f32, H as f32);
            }
        }
        match app.build_gpu_frame(1.0, (W, H), None) {
            GpuFrame::Partial { scene, size, rects } => {
                partials += 1;
                render_partial(
                    &device,
                    &queue,
                    &mut renderer,
                    &ui,
                    &mut target,
                    PartialFrame {
                        scene,
                        size,
                        rects: &rects,
                        base: vello::peniko::Color::WHITE,
                    },
                )
                .unwrap();
            }
            GpuFrame::Full(scene) => {
                renderer
                    .render_to_texture(
                        &device,
                        &queue,
                        scene,
                        &ui.create_view(&Default::default()),
                        &params(W, H),
                    )
                    .unwrap();
            }
            GpuFrame::Unchanged => {}
        }
        let scene = app.build_scene(1.0, (W, H)).clone();
        renderer
            .render_to_texture(
                &device,
                &queue,
                &scene,
                &reference.create_view(&Default::default()),
                &params(W, H),
            )
            .unwrap();
        let a = readback(&device, &queue, &ui, W, H);
        let b = readback(&device, &queue, &reference, W, H);
        let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        if differ != 0 {
            for (k, (x, y)) in a
                .iter()
                .zip(&b)
                .enumerate()
                .filter(|(_, (x, y))| x != y)
                .take(12)
            {
                if x != y {
                    let p = k / 4;
                    eprintln!(
                        "  px ({}, {}) ch {}: {} vs full {}",
                        p as u32 % W,
                        p as u32 / W,
                        k % 4,
                        x,
                        y
                    );
                }
            }
        }
        assert_eq!(
            differ, 0,
            "step {i} (hover at {y}): {differ} channels differ from a full render"
        );
    }
    assert!(partials >= 5, "only {partials} partial frames");
}
