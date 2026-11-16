//! #2781: the desktop loop's look at the frames the windows gave Vello
//! (#2071) asks every window that has a GPU renderer, not only the main one
//! and DevTools. A frame of a second window that overflowed Vello's buffers
//! is redrawn, in full and at the grown sizes, and the loop's timed wake stays
//! armed while that window alone has frames out.
//!
//! The first tests need no adapter: each window's renderer is [`FakeVello`],
//! which answers `check_frames` the way Vello does (counters back a few looks
//! after the submit; a frame that needed more than the buffers hold went out
//! undrawn, and the buffers grow to what it needed) and keeps the real
//! `UiTextureState` the way `WgpuRenderer` does, so whether the redraw is full
//! is the app's own answer. The last is the same through a real Vello
//! renderer, `#[ignore]`d for an adapter:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_bump_overflow_second_window --test-threads=1`.

use super::*;
use crate::app::GpuFrame;
use crate::shell::desktop::UiTextureState;
use crate::window_handle::WindowHandle;
use rinch_core::element::WindowProps;
use std::collections::{HashMap, VecDeque};
use vello::FrameCheck;
use vello::peniko::Color;

/// The paths the buffers of a new [`FakeVello`] hold.
const FIRST_CAPACITY: u32 = 64;
/// Looks between a frame's submit and its counters' arrival.
const LATENCY: u32 = 2;

/// A window's Vello renderer as the desktop loop sees it, without a GPU.
struct FakeVello {
    /// Paths a frame may have before it overflows the buffers.
    capacity: u32,
    /// The frames out: the paths each needed, and looks until its counters
    /// are back.
    out: VecDeque<(u32, u32)>,
    /// A frame came back overflowed and nobody has been told yet.
    lost: bool,
    /// What `WgpuRenderer` keeps of its UI texture.
    ui: UiTextureState,
}

impl FakeVello {
    fn new() -> Self {
        Self {
            capacity: FIRST_CAPACITY,
            out: VecDeque::new(),
            lost: false,
            ui: UiTextureState::default(),
        }
    }

    /// Draw `w`'s frame as `paint_gpu` does: full when the UI texture holds
    /// no good frame. Answers what kind it was, and the paths it needed.
    fn draw(&mut self, w: &mut ShellWindow) -> (&'static str, u32) {
        let base = Color::WHITE;
        let size = (w.width, w.height);
        let target = self.ui.invalid_reason(base);
        let (kind, need) = match w.app.build_gpu_frame(1.0, size, target) {
            GpuFrame::Full(scene) => ("full", scene.encoding().n_paths),
            GpuFrame::Partial { scene, .. } => ("partial", scene.encoding().n_paths),
            GpuFrame::Unchanged => return ("unchanged", 0),
        };
        if kind == "full" {
            self.ui.begin_render();
        }
        self.ui.rendered(base);
        self.out.push_back((need, LATENCY));
        (kind, need)
    }

    /// `WgpuRenderer::check_frames`.
    fn check(&mut self) -> FrameCheck {
        for (_, looks) in &mut self.out {
            *looks = looks.saturating_sub(1);
        }
        while let Some(&(need, 0)) = self.out.front() {
            self.out.pop_front();
            if need > self.capacity {
                self.capacity = need;
                self.lost = true;
            }
        }
        let check = if std::mem::take(&mut self.lost) {
            FrameCheck::Lost
        } else if self.out.is_empty() {
            FrameCheck::Settled
        } else {
            FrameCheck::Pending
        };
        self.ui.note_check(check);
        check
    }
}

/// The loop's look, each window's renderer the fake kept for its document.
fn look(rt: &mut RinchRuntime, vello: &mut HashMap<u64, FakeVello>) -> FramesLook {
    rt.look_at_frames(|w| vello.get_mut(&w.app.doc_key()).map(FakeVello::check))
}

fn light(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute(
        "style",
        "width: 100px; height: 40px; background: rgb(0, 0, 200)",
    );
    root
}

/// More boxes than [`FIRST_CAPACITY`] paths: a bar with many buttons.
fn heavy(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("style", "display: flex; flex-wrap: wrap; width: 400px");
    for _ in 0..200 {
        let button = scope.create_element("div");
        button.set_attribute(
            "style",
            "width: 18px; height: 18px; margin: 1px; background: rgb(200, 30, 40)",
        );
        root.append_child(&button);
    }
    root
}

fn props() -> WindowProps {
    WindowProps {
        width: 400,
        height: 300,
        ..WindowProps::default()
    }
}

/// What `open_pending_window` does, but mounted without an OS window, and
/// laid out for its first frame.
fn make(rt: &mut RinchRuntime, handle: WindowHandle) -> u64 {
    let id = handle.id();
    let pending = crate::window_handle::take_pending(id).expect("waiting to be made");
    let mut window = RinchRuntime::new_other_window(pending, &rt.fonts);
    crate::window_handle::mounting(id, || window.mount_before_window(1.0));
    crate::window_handle::note_mounted(id, window.app.doc_key());
    window
        .app
        .resolve_and_repaint(window.width as f32, window.height as f32);
    rt.others.push(window);
    id
}

fn open(rt: &mut RinchRuntime, content: fn(&mut RenderScope) -> NodeHandle) -> u64 {
    let handle = rinch_core::reactive::unowned(|| crate::open_window(props(), content));
    make(rt, handle)
}

fn doc(rt: &mut RinchRuntime, role: WindowRole) -> u64 {
    rt.w(role).app.doc_key()
}

/// A second window's frame that overflows: the look keeps the wake armed
/// while its counters are out (the main window's long back), finds it lost,
/// asks that window to draw again, and grows its buffers; the redraw is full,
/// fits, and the wake disarms.
#[test]
fn a_second_windows_lost_frame_is_redrawn_in_full_and_its_buffers_grow() {
    let mut rt = RinchRuntime::new("t", 800, 600, light, &[]);
    rt.main.app.mount_component(800.0, 600.0);
    rt.main.app.resolve_and_repaint(800.0, 600.0);
    let id = open(&mut rt, heavy);
    let other = WindowRole::Other(id);
    let mut vello = HashMap::new();
    vello.insert(doc(&mut rt, WindowRole::Main), FakeVello::new());
    vello.insert(doc(&mut rt, other), FakeVello::new());
    let main_doc = doc(&mut rt, WindowRole::Main);
    let other_doc = doc(&mut rt, other);

    // The main window draws, and its counters come back: it fits.
    let (_, need) = vello.get_mut(&main_doc).unwrap().draw(&mut rt.main);
    assert!(need <= FIRST_CAPACITY, "the main window fits: {need}");
    while look(&mut rt, &mut vello).pending {}

    // The second window draws its first frame, too heavy for its buffers.
    let (kind, need) = vello.get_mut(&other_doc).unwrap().draw(rt.w(other));
    assert_eq!(kind, "full");
    assert!(need > FIRST_CAPACITY, "or this test proves nothing: {need}");

    let first = look(&mut rt, &mut vello);
    assert!(
        first.pending && first.lost.is_empty(),
        "only the second window has frames out, and the wake stays armed for it: {first:?}"
    );
    let second = look(&mut rt, &mut vello);
    assert_eq!(second.lost, [other], "its lost frame is redrawn");
    let fake = &vello[&other_doc];
    assert_eq!(
        fake.capacity, need,
        "its buffers grew to what the frame needed"
    );
    assert_eq!(
        fake.ui.invalid_reason(Color::WHITE),
        Some(rinch_dom::perf::FullRepaintReason::Invalidated),
        "and its next frame is a full one"
    );

    // The redraw it asked for: full, at the grown sizes, so it fits.
    let (kind, _) = vello.get_mut(&other_doc).unwrap().draw(rt.w(other));
    assert_eq!(kind, "full");
    let mut looks = 0;
    let settled = loop {
        let look = look(&mut rt, &mut vello);
        looks += 1;
        if !look.pending {
            break look;
        }
    };
    assert!(settled.lost.is_empty(), "the redraw fitted: {settled:?}");
    assert_eq!(
        looks, LATENCY,
        "armed until its counters were back, no longer"
    );
    assert_eq!(
        vello[&main_doc].capacity, FIRST_CAPACITY,
        "the main window's untouched"
    );
}

fn root_opening_two_bars(scope: &mut RenderScope) -> NodeHandle {
    crate::open_window(props(), heavy);
    crate::open_window(props(), light);
    scope.create_element("div")
}

/// A windowless app has no main window: every window is a second one, and
/// each is asked. Only the heavy one lost a frame.
#[test]
fn a_windowless_apps_windows_are_each_asked() {
    crate::window_handle::set_windowless(true);
    crate::window_handle::set_exit_when(crate::ExitWhen::Explicitly);
    let mut rt = RinchRuntime::new("t", 800, 600, root_opening_two_bars, &[]);
    crate::window_handle::set_windowless(false);
    crate::window_handle::set_exit_when(crate::ExitWhen::MainWindowCloses);
    rt.mount_windowless_root();
    let opened = crate::windows();
    assert_eq!(opened.len(), 2);
    let ids: Vec<u64> = opened.into_iter().map(|h| make(&mut rt, h)).collect();
    let mut vello = HashMap::new();
    for &id in &ids {
        let role = WindowRole::Other(id);
        let doc = doc(&mut rt, role);
        let mut fake = FakeVello::new();
        assert_eq!(fake.draw(rt.w(role)).0, "full");
        vello.insert(doc, fake);
    }
    assert!(look(&mut rt, &mut vello).pending, "their counters are out");
    let lost = look(&mut rt, &mut vello);
    assert_eq!(
        lost.lost,
        [WindowRole::Other(ids[0])],
        "the heavy bar's frame"
    );
    assert!(!lost.pending);
    assert!(vello[&doc(&mut rt, WindowRole::Other(ids[0]))].capacity > FIRST_CAPACITY);
    assert_eq!(
        vello[&doc(&mut rt, WindowRole::Other(ids[1]))].capacity,
        FIRST_CAPACITY
    );
}

/// The first test through a real Vello renderer: #2071's 400 window-sized
/// layers, in a second window, overflow its first frame. The loop's looks
/// alone (never a blocking poll) find it lost, ask that window to draw again,
/// and the redraws, full and at the grown sizes, draw it; a frame after that
/// fits at once.
#[cfg(software_shell)]
#[test]
#[ignore]
fn gpu_bump_overflow_second_window_is_redrawn_by_the_loop() {
    use crate::app::gpu_bump_overflow_2071_tests::{Gpu, assert_drawn, layered};
    use rinch_dom::perf::FullRepaintReason;
    use std::time::{Duration, Instant};

    let mut rt = RinchRuntime::new("t", 800, 600, light, &[]);
    rt.main.app.mount_component(800.0, 600.0);
    let props = WindowProps {
        width: 1024,
        height: 768,
        ..WindowProps::default()
    };
    let handle = rinch_core::reactive::unowned(|| crate::open_window(props, layered));
    let id = make(&mut rt, handle);
    let other = WindowRole::Other(id);
    let other_doc = doc(&mut rt, other);
    let mut gpu = Gpu::new();

    // The loop's looks until every frame is in, each lost one redrawn the way
    // its redraw would: `WgpuRenderer::check_frames` is Vello's answer noted
    // by the UI texture's state.
    let settle = |rt: &mut RinchRuntime, gpu: &mut Gpu| -> u32 {
        let start = Instant::now();
        let mut redraws = 0;
        loop {
            let look = rt.look_at_frames(|w| {
                (w.app.doc_key() == other_doc).then(|| {
                    let check = gpu.renderer.check_frames(&gpu.device);
                    gpu.state.note_check(check);
                    check
                })
            });
            for role in &look.lost {
                assert_eq!(*role, other);
                redraws += 1;
                assert_eq!(gpu.render(&mut rt.w(other).app, None), "full");
            }
            if !look.pending && look.lost.is_empty() {
                return redraws;
            }
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "the counters never came back"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    };

    assert_eq!(
        gpu.render(&mut rt.w(other).app, Some(FullRepaintReason::FirstFrame)),
        "full"
    );
    let redraws = settle(&mut rt, &mut gpu);
    assert!(
        (1..=4).contains(&redraws),
        "the first frame overflowed and was redrawn, one bad frame per buffer at the most: {redraws}"
    );
    assert_drawn(&gpu);
    // At the grown sizes it now fits at once.
    gpu.render(&mut rt.w(other).app, Some(FullRepaintReason::Invalidated));
    assert_eq!(settle(&mut rt, &mut gpu), 0);
    assert_drawn(&gpu);
}

/// A hidden window prepared on the GPU (#2633) gives its frame to the
/// renderer parked for its show: its overflow is found there too, by the
/// loop's own look ([`RinchRuntime::check_vello_frames`], never a blocking
/// poll), and the window is prepared again, so the show presents a drawn
/// frame.
#[cfg(software_shell)]
#[test]
#[ignore]
fn gpu_bump_overflow_second_window_hidden_and_prepared_is_prepared_again() {
    use crate::app::gpu_bump_overflow_2071_tests::layered;
    use crate::app::gpu_partial_frame_1718_tests::readback;
    use std::time::{Duration, Instant};

    let mut rt = RinchRuntime::new("t", 800, 600, light, &[]);
    rt.main.app.mount_component(800.0, 600.0);
    let props = WindowProps {
        width: 1024,
        height: 768,
        visible: false,
        ..WindowProps::default()
    };
    let handle = rinch_core::reactive::unowned(|| crate::open_window(props, layered));
    let id = make(&mut rt, handle);
    let other = WindowRole::Other(id);
    rt.w(other)
        .prepare_hidden_as(crate::shell::renderer::Renderer::Gpu, Some(1.0));
    assert!(rt.w(other).prepared.is_some(), "prepared");
    assert!(rt.w(other).parked_renderer.is_some(), "on the GPU");

    let start = Instant::now();
    let mut looks = 0;
    while rt.check_vello_frames().is_some() {
        looks += 1;
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "the counters never came back"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(looks > 0, "the preparation's frames were out");
    let r = rt.w(other).parked_renderer.as_ref().unwrap();
    let px = readback(&r.device, &r.queue, &r.render_texture, 1024, 768);
    let at = ((210 * 1024 + 310) * 4) as usize;
    assert_eq!(px[at..at + 4], [30, 200, 90, 255], "the box on top");
}
