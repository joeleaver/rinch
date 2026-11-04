//! #2137 on the GPU painter: a `visibility: hidden` subtree with nothing
//! visible in it is skipped by paint and the stacking collector, and Vello's
//! picture is the one it paints when the subtree is walked
//! (`rinch_dom::visibility::set_hidden_subtree_cull`). The software half is
//! `crates/rinch-dom/tests/hidden_subtree_cull_2137_tests.rs`. They need an
//! adapter: `cargo test -p rinch --features gpu --lib -- --ignored gpu_hidden
//! --test-threads=1`.
use super::gpu_partial_frame_1718_tests::readback;
use super::*;

const W: u32 = 320;
const H: u32 = 240;

/// One device, one renderer and one target, for a run of scenes.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    target: wgpu::Texture,
}

impl Gpu {
    fn new() -> Self {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .unwrap();
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
        Self {
            device,
            queue,
            renderer,
            target,
        }
    }

    /// A full frame of `app`, with the hidden-subtree skip on or off: its
    /// pixels, and how many clips and layers its scene encodes.
    fn render(&mut self, app: &mut RinchApp, cull: bool) -> (Vec<u8>, u32) {
        rinch_dom::visibility::set_hidden_subtree_cull(cull);
        let frame = app.build_gpu_frame(
            1.0,
            (W, H),
            Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
        );
        rinch_dom::visibility::set_hidden_subtree_cull(true);
        let GpuFrame::Full(scene) = frame else {
            panic!("a first frame is full")
        };
        let clips = scene.encoding().n_clips;
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &self.target.create_view(&Default::default()),
                &vello::RenderParams {
                    base_color: vello::peniko::Color::WHITE,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .unwrap();
        (
            readback(&self.device, &self.queue, &self.target, W, H),
            clips,
        )
    }
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p != q).count()
}

struct Rng(u64);
impl Rng {
    fn pick(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
    fn of<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.pick(xs.len() as u64) as usize]
    }
}

fn color(r: &mut Rng) -> String {
    format!(
        "rgba({}, {}, {}, 0.{})",
        r.pick(256),
        r.pick(256),
        r.pick(256),
        4 + r.pick(6)
    )
}

fn visibility(r: &mut Rng) -> &'static str {
    r.of(&[
        "",
        "",
        "visibility: hidden;",
        "visibility: hidden;",
        "visibility: visible;",
    ])
}

fn div(scope: &mut RenderScope, parent: &NodeHandle, style: &str) -> NodeHandle {
    let d = scope.create_element("div");
    d.set_attribute("style", style);
    parent.append_child(&d);
    d
}

/// The rinch-dom differential's shapes, as a component: paragraphs with spans
/// of their own visibility, decorated leaves, positioned and `z-index`ed boxes
/// (translucent ones among them), scrollers, `display: contents` wrappers and
/// transformed or translucent containers, hidden often and overridden now and
/// then.
fn random_box(scope: &mut RenderScope, r: &mut Rng, parent: &NodeHandle, depth: u32) {
    let kind = r.pick(if depth >= 3 { 3 } else { 8 });
    let vis = visibility(r);
    let bg = color(r);
    match kind {
        0 => {
            let p = div(scope, parent, &format!("{vis} background: {bg}"));
            let t = scope.create_text("Lorem ipsum ");
            p.append_child(&t);
            let span = scope.create_element("span");
            span.set_attribute(
                "style",
                &format!("{} background: {}", visibility(r), color(r)),
            );
            let t = scope.create_text("dolor sit");
            span.append_child(&t);
            p.append_child(&span);
        }
        1 | 2 => {
            let deco = r.of(&[
                "",
                "box-shadow: 3px 3px 4px rgb(0, 80, 80);",
                "border: 2px solid rgb(0, 0, 0); border-radius: 5px;",
            ]);
            div(
                scope,
                parent,
                &format!(
                    "{vis} width: {}px; height: {}px; background: {bg}; {deco}",
                    10 + r.pick(80),
                    4 + r.pick(30)
                ),
            );
        }
        3 | 4 => {
            let position = r.of(&["relative", "absolute", "absolute", "fixed"]);
            let z = r.of(&["", "z-index: -1;", "z-index: 0;", "z-index: 2;"]);
            let opacity = r.of(&["", "", "opacity: 0.6;"]);
            let b = div(
                scope,
                parent,
                &format!(
                    "{vis} position: {position}; left: {}px; top: {}px; width: {}px; \
                     height: {}px; background: {bg}; {z} {opacity}",
                    r.pick(200) as i64 - 20,
                    r.pick(160) as i64 - 20,
                    10 + r.pick(90),
                    10 + r.pick(60)
                ),
            );
            for _ in 0..r.pick(3) {
                random_box(scope, r, &b, depth + 1);
            }
        }
        5 => {
            let s = div(
                scope,
                parent,
                &format!(
                    "{vis} width: {}px; height: {}px; overflow: {}; background: {bg}",
                    40 + r.pick(120),
                    30 + r.pick(60),
                    r.of(&["auto", "scroll", "hidden"])
                ),
            );
            div(scope, &s, "height: 300px; width: 10px");
            for _ in 0..1 + r.pick(3) {
                random_box(scope, r, &s, depth + 1);
            }
        }
        6 => {
            let c = div(scope, parent, &format!("{vis} display: contents"));
            for _ in 0..1 + r.pick(3) {
                random_box(scope, r, &c, depth + 1);
            }
        }
        _ => {
            let extra = r.of(&[
                "",
                "transform: translate(3px, 2px);",
                "opacity: 0.8;",
                "padding: 3px;",
            ]);
            let c = div(scope, parent, &format!("{vis} background: {bg}; {extra}"));
            for _ in 0..1 + r.pick(4) {
                random_box(scope, r, &c, depth + 1);
            }
        }
    }
}

fn random_app(seed: u64) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x2137);
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "font-family: sans-serif; font-size: 14px; line-height: 18px",
        );
        for _ in 0..2 + r.pick(4) {
            random_box(scope, &mut r, &root, 0);
        }
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

/// Random documents of hidden and visible subtrees, on Vello: the picture with
/// the skip is the picture without it, pixel for pixel. Positive control: the
/// skip fired (fewer paint visits) on many of them.
#[test]
#[ignore]
fn gpu_hidden_subtree_cull_paints_the_same_on_vello() {
    let mut gpu = Gpu::new();
    let mut skipped = 0;
    for seed in 0..60u64 {
        let mut app = random_app(seed);
        app.reset_perf();
        let (on, _) = gpu.render(&mut app, true);
        let on_visits = app
            .end_perf_frame()
            .unwrap()
            .get(rinch_dom::perf::Counter::PaintNodesVisited);
        let (off, _) = gpu.render(&mut app, false);
        let off_visits = app
            .end_perf_frame()
            .unwrap()
            .get(rinch_dom::perf::Counter::PaintNodesVisited);
        assert_eq!(differing(&on, &off), 0, "seed {seed}");
        if on_visits < off_visits {
            skipped += 1;
        }
    }
    assert!(skipped >= 20, "the skip fired on {skipped} of 60");
}

/// A page under a closed `Drawer` full of translucent badges: on Vello, the
/// same picture with the skip, and the badges' 30 empty group layers gone
/// from the scene.
#[test]
#[ignore]
fn gpu_a_closed_drawer_paints_the_same_on_vello() {
    use rinch_components::Drawer;
    use rinch_core::Component;
    let mut app = super::perf_expect::new_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = format!(
            "{} body {{ font-family: sans-serif; font-size: 12px; line-height: 14px; }} \
             .item {{ position: relative; height: 14px; }} \
             .badge {{ position: absolute; right: 4px; top: 3px; width: 8px; height: 8px; \
                       opacity: 0.5; background: rgb(200, 0, 0); }}",
            rinch_components::generate_component_css()
        );
        let t = scope.create_text(&css);
        style.append_child(&t);
        root.append_child(&style);
        for i in 0..10 {
            let row = div(
                scope,
                &root,
                "height: 20px; background: rgb(220, 230, 250); \
                 border-bottom: 1px solid rgb(120, 120, 160)",
            );
            let t = scope.create_text(&format!("page row {i}"));
            row.append_child(&t);
        }
        let content = scope.create_element("div");
        for i in 0..30 {
            let item = div(scope, &content, "");
            item.set_attribute("class", "item");
            let t = scope.create_text(&format!("drawer item {i}"));
            item.append_child(&t);
            let b = div(scope, &item, "");
            b.set_attribute("class", "badge");
        }
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(|| false)),
            position: "left".to_string(),
            ..Default::default()
        }
        .render(scope, &[content]);
        root.append_child(&drawer);
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    let mut gpu = Gpu::new();
    let (on, on_clips) = gpu.render(&mut app, true);
    let (off, off_clips) = gpu.render(&mut app, false);
    assert_eq!(differing(&on, &off), 0);
    assert!(
        on_clips + 30 <= off_clips,
        "walked, the drawer's 30 badges open a layer each ({off_clips} against {on_clips})"
    );
}

/// Two translucent rows under one rounded clip, `z` 1 and 3, with a hidden box
/// at `z` 2 under another clip between them. Walked, the hidden entry's chain
/// was switched to and back, so the rows took a clip layer each; skipped, they
/// share one, as gathered entries do (#1645), and Vello antialiases that
/// clip's edge once. The picture is the one the page without the hidden box
/// paints.
#[test]
#[ignore]
fn gpu_a_hidden_entry_between_two_runs_of_one_clip() {
    let page = |hidden: bool| {
        let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "position: absolute; left: 0; top: 0; width: 320px; height: 240px; \
                 background-color: white",
            );
            let row = "position: absolute; left: 0; top: 0; width: 100%; height: 100%";
            let a = div(
                scope,
                &root,
                "position: absolute; left: 13px; top: 9px; width: 181px; height: 170px; \
                 overflow: hidden; border-radius: 30px",
            );
            let b = div(
                scope,
                &root,
                "position: absolute; left: 150px; top: 9px; width: 90px; height: 170px; \
                 overflow: hidden",
            );
            div(
                scope,
                &a,
                &format!("{row}; z-index: 1; background: rgba(0, 160, 0, 0.6)"),
            );
            if hidden {
                div(
                    scope,
                    &b,
                    &format!("{row}; z-index: 2; visibility: hidden; background: red"),
                );
            }
            div(
                scope,
                &a,
                &format!("{row}; z-index: 3; background: rgba(0, 0, 200, 0.6)"),
            );
            root
        });
        app.mount_component(W as f32, H as f32);
        app.resolve_and_repaint(W as f32, H as f32);
        app
    };
    let mut gpu = Gpu::new();
    let (without, _) = gpu.render(&mut page(false), true);
    let mut app = page(true);
    let (on, on_clips) = gpu.render(&mut app, true);
    let (off, off_clips) = gpu.render(&mut app, false);
    let max_step = on
        .iter()
        .zip(&off)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    // Measured on lavapipe: walked, 194 pixels of the rounded edge differ,
    // by at most 23 levels; the clip layers fall 8 → 4.
    eprintln!(
        "on vs off: {} px, max {max_step}; clips {on_clips} / {off_clips}",
        differing(&on, &off)
    );
    assert!(on_clips < off_clips, "the rows share one clip layer now");
    assert_eq!(
        differing(&on, &without),
        0,
        "skipped, the picture is the page's without the hidden box"
    );
    assert_eq!(differing(&on, &without), 0);
}
