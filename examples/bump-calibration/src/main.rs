//! Measures what rinch scenes need of Vello's bump-allocated buffers against
//! the rules that give their first sizes (`BumpSizes::initial`, #2466), so
//! that the rules can be recalibrated: on another GPU, for a new kind of
//! scene, or after a change to Vello's shaders.
//!
//! Each scene is rendered offscreen at each size through the embed API
//! (`RinchContext`, `RinchOverlayRenderer`), on a new renderer, with Vello's
//! bump reports on (`record_bump_reports`). For every frame the table takes
//! the smallest first size over need per buffer: the margin the rule leaves,
//! under 1 where a first frame would run out. It also prints the units the
//! rules are written in (lines and segments per path tag, tiles and ptcl
//! words per target tile) and the deepest nesting, for writing new ones.
//!
//! ```text
//! cargo run --release -p bump-calibration [-- --size 1920x1080 --scene curves]
//! ```
//!
//! With no arguments: both scenes (`curves`, `ui-zoo`) at 1280x720,
//! 1920x1080 and 3840x2160. Run it in an isolated session
//! (`scripts/isolated-session.sh`): it needs only the GPU, and ui-zoo's
//! sections may look for session services.

mod report;
mod scenes;

use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};
use rinch::prelude::*;
use rinch::wgpu;
use scenes::Scene;

/// Reports kept per scene and size: more than its frames.
const REPORTS: usize = 1024;

fn main() {
    rinch::init_logging();
    let (sizes, scenes) = arguments();
    let (device, queue) = gpu();
    println!("{}", report::header());
    for &scene in &scenes {
        for &(width, height) in &sizes {
            let summary = measure(&device, &queue, scene, width, height);
            println!("{}", report::row(scene.name(), width, height, &summary));
        }
    }
}

/// `--size WxH` and `--scene NAME`, each repeatable; the defaults otherwise.
fn arguments() -> (Vec<(u32, u32)>, Vec<Scene>) {
    let mut sizes = Vec::new();
    let mut scenes = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .unwrap_or_else(|| usage(&format!("{arg} needs a value")));
        match arg.as_str() {
            "--size" => sizes.push(size(&value)),
            "--scene" => scenes
                .push(Scene::named(&value).unwrap_or_else(|| usage(&format!("no scene {value}")))),
            other => usage(&format!("unknown argument {other}")),
        }
    }
    if sizes.is_empty() {
        sizes = vec![(1280, 720), (1920, 1080), (3840, 2160)];
    }
    if scenes.is_empty() {
        scenes = Scene::ALL.to_vec();
    }
    (sizes, scenes)
}

fn size(value: &str) -> (u32, u32) {
    value
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or_else(|| usage(&format!("not a size: {value}")))
}

fn usage(problem: &str) -> ! {
    eprintln!(
        "bump-calibration: {problem}\n\
         usage: bump-calibration [--size WxH]... [--scene curves|ui-zoo]..."
    );
    std::process::exit(2)
}

/// The high-performance adapter, with its own limits.
fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("a GPU adapter");
    let info = adapter.get_info();
    eprintln!(
        "adapter: {} ({}, {})",
        info.name, info.driver, info.driver_info
    );
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("a device")
}

/// Every frame of `scene` at `width` x `height`, on a new renderer.
fn measure(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: Scene,
    width: u32,
    height: u32,
) -> report::Summary {
    let config = RinchContextConfig {
        width,
        height,
        scale_factor: 1.0,
        theme: Some(ThemeProviderProps::default()),
        fonts: Vec::new(),
    };
    let mut ctx = RinchContext::new(config, move |scope: &mut RenderScope| {
        scene.build(scope, width, height)
    });
    let mut overlay =
        RinchOverlayRenderer::new(device, width, height, wgpu::TextureFormat::Rgba8Unorm);
    overlay.record_bump_reports(REPORTS);
    for step in 0..scene.steps() {
        scene.show_step(step);
        // Twice: the second frame is the one a resolved layout draws.
        for _ in 0..2 {
            ctx.update(&[]);
            let _ = overlay.render(device, queue, &mut ctx);
            settle(device, queue, &mut overlay, &mut ctx);
        }
    }
    scene.finish();
    report::Summary::of(&overlay.take_bump_reports())
}

/// Until every frame's counters are back, drawing again any that was lost.
fn settle(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    overlay: &mut RinchOverlayRenderer,
    ctx: &mut RinchContext,
) {
    loop {
        let check = overlay.check_frames(device);
        if check.needs_render {
            let _ = overlay.render(device, queue, ctx);
        } else if check.check_again_at.is_some() {
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        } else {
            return;
        }
    }
}
