//! #2688 on a real adapter: the wgpu-hal command encoders a GPU window's
//! frames leave alive.
//!
//! wgpu-core keeps every hal command encoder it has made in a free list (a
//! stack) and never trims it, and a Vulkan driver keeps each encoder's command
//! pool at the longest command stream it has held. So the most encoders ever
//! alive at once, and which of them ever held Vello's stream, are what the
//! process keeps (on RADV 1.3 MiB of GTT for a short stream, 3 to 4 MiB for a
//! Vello frame's). The `gpu_` tests count hal encoders (`wgpu`'s `counters`
//! feature, a dev-dependency), which every adapter reports. The `_probe_`s
//! are run by hand, not in CI (`--skip probe_2688`): the `gtt_` ones read the
//! driver's memory (`drm-total-gtt`) and fail where it is not reported, the
//! others only print.
//!
//! Frames are presented to the renderer's test stand-in swapchain.

use super::*;
use crate::shell::desktop::WgpuRenderer;
use rinch_dom::perf::FullRepaintReason;
use std::sync::Arc;
use std::time::Duration;

const PANEL: (u32, u32) = (1280, 40);
const WINDOW: (u32, u32) = (1280, 720);

/// An app with a line of text and a cell that turns red under the pointer, so
/// moving the pointer in and out of it makes partial frames.
fn app_at(size: (u32, u32)) -> RinchApp {
    let mut app = super::perf_expect::new_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: fixed; inset: 0; background: rgb(0, 0, 255); color: white; \
             font: 16px/20px sans-serif",
        );
        let cell = scope.create_element("div");
        cell.set_attribute("class", "cell");
        root.append_child(&cell);
        root.append_child(&scope.create_text("A panel-like window, presenting."));
        root
    });
    app.mount_component(size.0 as f32, size.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(
            ".cell { position: absolute; left: 0; top: 0; width: 30px; height: 20px; \
             background: green } .cell:hover { background: red }",
        );
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(size.0 as f32, size.1 as f32);
    app
}

fn gpu() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    eprintln!("#2688 adapter: {:?}", adapter.get_info().name);
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    (Arc::new(device), Arc::new(queue))
}

/// The hal command encoders the device has made and not destroyed: the ones
/// in flight and wgpu-core's free list.
fn hal_encoders(device: &wgpu::Device) -> isize {
    device.get_internal_counters().hal.command_encoders.read()
}

/// This process's GTT in KiB (`drm-total-gtt` over its DRM clients), or
/// `None` where no DRM client reports it (lavapipe, another OS).
fn gtt_kib() -> Option<u64> {
    let mut total = None;
    let mut clients = std::collections::HashSet::new();
    for entry in std::fs::read_dir("/proc/self/fdinfo").ok()?.flatten() {
        let Ok(info) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(client) = info.lines().find(|l| l.starts_with("drm-client-id:")) else {
            continue;
        };
        if !clients.insert(client.to_owned()) {
            continue;
        }
        for v in info
            .lines()
            .filter_map(|l| l.strip_prefix("drm-total-gtt:"))
        {
            let mut words = v.split_whitespace();
            let n: u64 = words.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let kib = match words.next() {
                Some("MiB") => n * 1024,
                Some("GiB") => n * 1024 * 1024,
                _ => n,
            };
            *total.get_or_insert(0) += kib;
        }
    }
    total
}

struct Window {
    device: Arc<wgpu::Device>,
    r: WgpuRenderer,
    app: RinchApp,
    size: (u32, u32),
    hovered: bool,
}

impl Window {
    /// A window of `size` whose renderer lets `limit` frames be in flight.
    fn new(size: (u32, u32), limit: usize) -> Self {
        let (device, queue) = gpu();
        let mut r = WgpuRenderer::offscreen(
            device.clone(),
            queue,
            size.0,
            size.1,
            wgpu::TextureFormat::Bgra8Unorm,
        );
        r.stand_in = Some(Vec::new());
        r.in_flight.limit = Some(limit);
        let mut w = Self {
            device,
            r,
            app: app_at(size),
            size,
            hovered: false,
        };
        let first = w
            .app
            .build_gpu_frame(1.0, size, Some(FullRepaintReason::FirstFrame));
        w.r.paint_frame(first, false).unwrap();
        w
    }

    /// A partial frame (the cell changes colour), presented as the desktop
    /// loop presents one.
    fn partial_frame(&mut self) {
        self.hovered = !self.hovered;
        let x = if self.hovered { 10.0 } else { 600.0 };
        let size = self.size;
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y: 10.0 }, size, 1.0);
        self.app.handle_event(PlatformEvent::AboutToWait, size, 1.0);
        self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        let frame = self.app.build_gpu_frame(1.0, size, None);
        assert!(
            matches!(frame, GpuFrame::Partial { .. }),
            "the cell's change makes a partial frame"
        );
        self.r.paint_frame(frame, false).unwrap();
        // The stand-in keeps every texture presented; a swapchain does not.
        self.r.stand_in.as_mut().unwrap().clear();
    }

    /// A full frame, presented.
    fn full_frame(&mut self) {
        let frame = self
            .app
            .build_gpu_frame(1.0, self.size, Some(FullRepaintReason::Invalidated));
        self.r.paint_frame(frame, false).unwrap();
        self.r.stand_in.as_mut().unwrap().clear();
    }

    /// Time for the GPU to finish what it was given, with nothing polling wgpu
    /// meanwhile (a window presenting once a second does nothing between).
    fn idle(&self) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A window presenting now and then (deskrinch's panel: a clock, once a
/// second): its frames never overlap, so one frame's encoders are all it ever
/// needs. A partial frame's copies and the composite are one submission after
/// Vello's, so that is four: the frame's own, Vello's, and wgpu's
/// pending-writes encoder for Vello's uploads and the one that replaces it.
/// Before #2688 the copies were a submission of their own, and five.
#[test]
#[ignore]
fn gpu_an_idle_presenter_keeps_four_encoders_2688() {
    // One frame in flight: each frame waits for the last, however slow the
    // adapter, so nothing here depends on how long the GPU takes.
    let mut w = Window::new(PANEL, 1);
    w.idle();
    let mut counts = Vec::new();
    for _ in 0..12 {
        w.partial_frame();
        w.idle();
        counts.push(hal_encoders(&w.device));
    }
    eprintln!("#2688 idle partial frames, hal encoders after each: {counts:?}");
    assert!(
        counts.iter().all(|&n| n <= 4),
        "an idle presenter's hal encoders: {counts:?}"
    );
}

/// Frames rendered back to back, faster than the GPU finishes them (a window
/// animating at its display's rate on a slow GPU, or one whose presents are
/// held while it keeps rendering): with at most `limit` frames in flight, at
/// most `limit` frames' encoders are ever alive. One frame's are four (above);
/// each frame more in flight adds its three in flight.
#[test]
#[ignore]
fn gpu_frames_in_flight_are_limited_2688() {
    for (limit, most) in [(1, 4), (2, 7)] {
        let mut w = Window::new(WINDOW, limit);
        let mut peak = 0;
        for _ in 0..6 {
            for _ in 0..8 {
                w.full_frame();
                peak = peak.max(hal_encoders(&w.device));
                assert!(
                    w.r.in_flight.pending() <= limit,
                    "{} frames in flight at a limit of {limit}",
                    w.r.in_flight.pending()
                );
            }
            w.idle();
        }
        eprintln!("#2688 limit {limit}: at most {peak} hal encoders");
        assert!(
            peak <= most,
            "{peak} hal encoders alive at once with {limit} frames in flight"
        );
    }
}

/// No limit: what the frames do without one, for comparison (not asserted:
/// how far frames overlap depends on the GPU and the build).
#[test]
#[ignore]
fn no_frame_limit_probe_2688() {
    let mut w = Window::new(WINDOW, 0);
    let mut peak = 0;
    for _ in 0..6 {
        for _ in 0..8 {
            w.full_frame();
            peak = peak.max(hal_encoders(&w.device));
        }
        w.idle();
    }
    eprintln!("#2688 no limit: at most {peak} hal encoders");
}

/// The driver's memory for an idle presenter (RADV, run by hand): each frame
/// takes its encoders from the free list in the order the last frame put them
/// back, so Vello's command stream stays in one encoder and the others keep
/// their short ones. When Vello's went to a different encoder each frame, the
/// driver kept every encoder at Vello's size: deskrinch's panel climbed from
/// 13 to 32 MiB over its first presents, and an earlier form of this probe
/// by 11.8 MiB over its first five full frames at 1280x40.
#[test]
#[ignore]
fn gtt_an_idle_presenter_does_not_climb_probe_2688() {
    let mut w = Window::new(PANEL, 2);
    w.idle();
    w.full_frame();
    w.idle();
    let start = gtt_kib().expect("this driver reports no drm-total-gtt: run on hardware");
    let mut levels = Vec::new();
    for i in 0..16 {
        if i % 2 == 0 {
            w.full_frame();
        } else {
            w.partial_frame();
        }
        w.idle();
        levels.push(gtt_kib().unwrap());
    }
    eprintln!("#2688 idle presenter GTT KiB, from {start}: {levels:?}");
    let grew = levels.iter().max().unwrap() - start;
    assert!(
        grew <= 2048,
        "an idle presenter's GTT grew {grew} KiB after its first frame: {levels:?}"
    );
}

/// The driver's memory for frames back to back with at most two in flight
/// (RADV, run by hand): bounded at two frames' encoders.
#[test]
#[ignore]
fn gtt_back_to_back_frames_stop_climbing_probe_2688() {
    let mut w = Window::new(WINDOW, 2);
    let mut levels = Vec::new();
    for _ in 0..30 {
        for _ in 0..8 {
            w.full_frame();
        }
        w.idle();
        levels.push(gtt_kib().expect("this driver reports no drm-total-gtt: run on hardware"));
    }
    eprintln!("#2688 back to back, limit 2, GTT KiB per burst: {levels:?}");
    let grew = levels.last().unwrap() - levels[0];
    assert!(
        grew <= 8 * 1024,
        "GTT grew {grew} KiB over 30 bursts of 8: {levels:?}"
    );
}
