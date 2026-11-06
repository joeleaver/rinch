//! Smooth wheel scrolling (#2287): a mouse wheel's notch glides to where it
//! scrolls, on Chromium's curve, instead of jumping 40px in one frame.
//!
//! Every fixture drives the desktop loop's own path: the winit delta goes
//! through `shell::rinch_runtime::wheel_platform_event` (what both windows of
//! the shell call) into `handle_event`, and the frames are `AboutToWait`
//! turns with the document's animation clock (#2267) pinned and stepped 10 ms
//! a frame — Joe's 100 Hz display. Every scroller starts part-way down, off
//! offset 0, and every sample is taken mid-curve, so a fixture cannot pass on
//! a fixed point.

use super::*;
use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};
use winit::event::MouseScrollDelta;

const VP: (u32, u32) = (800, 600);
/// One frame at 100 Hz.
const FRAME_MS: f64 = 10.0;
const T0: f64 = 1_000_000.0;
/// Where every scroller starts: part-way down, so neither end is near.
const START: f64 = 333.0;

struct Fixture {
    app: RinchApp,
    scroller: u64,
    /// `scrollTop` at each `onscroll`, in order.
    fired: Rc<RefCell<Vec<f64>>>,
}

/// A 200x300 scroller at the origin, 3000px of content, with an `onscroll`
/// that records each offset it is told.
fn mount() -> Fixture {
    let fired: Rc<RefCell<Vec<f64>>> = Rc::default();
    let fired_in = fired.clone();
    let handler = register_scroll_handler(ScrollCallback::from(move |e: ScrollEvent| {
        fired_in.borrow_mut().push(e.scroll_top);
    }));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 200px; height: 300px; \
             overflow-y: auto",
        );
        scroller.set_attribute("id", "scroller");
        scroller.set_attribute("data-onscroll", &handler.0.to_string());
        let inner = scope.create_element("div");
        inner.set_attribute("style", "height: 3000px");
        scroller.append_child(&inner);
        root.append_child(&scroller);
        // Somewhere else for a `lock_scroll` to be taken from.
        let overlay = scope.create_element("div");
        overlay.set_attribute("id", "overlay");
        overlay.set_attribute(
            "style",
            "position: absolute; left: 400px; top: 0px; width: 10px; height: 10px",
        );
        root.append_child(&overlay);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let scroller = by_id(&app, "scroller");
    set_top(&mut app, scroller, START);
    let mut f = Fixture {
        app,
        scroller,
        fired,
    };
    f.app
        .doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_animation_clock_ms(Some(T0));
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    f.fired.borrow_mut().clear();
    f
}

fn by_id(app: &RinchApp, id: &str) -> u64 {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(n, _)| n)
        .unwrap_or_else(|| panic!("#{id}"))
}

fn top(app: &RinchApp, id: u64) -> f64 {
    app.doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(id)
        .unwrap()
        .scroll_offset
        .1
}

/// A programmatic `scroll_top` write, as `NodeHandle::set_scroll_top` makes it.
fn set_top(app: &mut RinchApp, id: u64, v: f64) {
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_scroll_top(rinch_core::dom::NodeId(id), v);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

/// What the desktop shell sends for `delta` with the pointer at `at`.
fn wheel(app: &mut RinchApp, at: (f32, f32), delta: MouseScrollDelta) -> Vec<AppAction> {
    let (x, y) = at;
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
    let event = crate::shell::rinch_runtime::wheel_platform_event(delta, at, 1.0)
        .expect("a line or pixel delta is an event");
    app.handle_event(event, VP, 1.0)
}

/// One notch toward the end of the content (winit's sign: negative `y`).
fn notch(app: &mut RinchApp) -> Vec<AppAction> {
    wheel(app, (100.0, 150.0), MouseScrollDelta::LineDelta(0.0, -1.0))
}

/// One turn of the loop, `FRAME_MS` after the last: answers whether it asked
/// for a redraw.
fn frame(app: &mut RinchApp) -> bool {
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .advance_animation_clock_ms(FRAME_MS);
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0)
        .contains(&AppAction::RequestRedraw)
}

/// Frames until the clock stops asking, at most `max`: the offset after each.
fn run(f: &mut Fixture, max: usize) -> Vec<f64> {
    let mut out = Vec::new();
    for _ in 0..max {
        let asked = frame(&mut f.app);
        out.push(top(&f.app, f.scroller) - START);
        if !asked {
            break;
        }
    }
    out
}

/// One notch: nothing moves in the frame it arrives, then 20 frames (200 ms,
/// Chromium's `kInverseDelta` duration for 40px) on `ease-in-out`, each a
/// whole device pixel (the offset is floored to the grid, #1543), and the
/// clock goes idle on the frame after the last move.
///
/// Mutants it kills: a linear ease (`[2, 4, 6, …]`), an ease-out (front-loaded:
/// `[5, 10, …]`), a 100 ms duration (40 at frame 10), and a tick that never
/// stops asking for frames.
#[test]
fn a_notch_glides_on_chromes_curve_then_goes_idle() {
    let mut f = mount();
    let actions = notch(&mut f.app);
    assert!(actions.contains(&AppAction::RequestRedraw));
    assert_eq!(
        top(&f.app, f.scroller),
        START,
        "the notch itself moves nothing: its frames do"
    );
    assert!(f.app.wheel_scroll_animating());
    let samples = run(&mut f, 40);
    assert_eq!(
        samples,
        [
            0.0, 0.0, 1.0, 3.0, 5.0, 7.0, 10.0, 13.0, 16.0, 20.0, 23.0, 26.0, 29.0, 32.0, 34.0,
            36.0, 38.0, 39.0, 39.0, 40.0, 40.0
        ],
        "offsets after each 10 ms frame"
    );
    assert!(!f.app.wheel_scroll_animating());
    // Idle after: the loop asks for nothing more.
    for _ in 0..3 {
        assert!(
            !frame(&mut f.app),
            "an idle loop after the glide asked for a frame"
        );
    }
    assert_eq!(top(&f.app, f.scroller), START + 40.0);
}

/// `onscroll` fires on every frame the offset moves, with that frame's
/// offset, as Chrome's does — not once for the notch.
#[test]
fn onscroll_fires_on_every_frame_that_moves() {
    let mut f = mount();
    notch(&mut f.app);
    let samples = run(&mut f, 40);
    let mut moved: Vec<f64> = Vec::new();
    let mut last = 0.0;
    for s in samples {
        if s != last {
            moved.push(START + s);
            last = s;
        }
    }
    assert!(
        moved.len() > 10,
        "positive control: it moved over many frames"
    );
    assert_eq!(*f.fired.borrow(), moved);
}

/// A second notch retargets the running glide: the offset carries on from
/// where it is at the speed it has — the frame after the second notch moves
/// at least as far as the frame before it — and ends 80px down. A glide
/// restarted from rest would crawl (`ease-in-out` starts flat).
#[test]
fn a_second_notch_retargets_without_restarting() {
    let mut f = mount();
    notch(&mut f.app);
    let first = run(&mut f, 8);
    assert_eq!(first.len(), 8);
    let before = first[7] - first[6];
    assert!(before >= 2.0, "positive control: it is moving ({first:?})");
    notch(&mut f.app);
    let rest = run(&mut f, 60);
    let after = rest[0] - first[7];
    assert!(
        after >= before,
        "the frame after the retarget moved {after}px, the one before {before}px: \
         it restarted from rest ({first:?} then {rest:?})"
    );
    // Monotonic, and it lands on the sum of the two notches.
    let all: Vec<f64> = first.iter().chain(rest.iter()).copied().collect();
    assert!(all.windows(2).all(|w| w[1] >= w[0]), "{all:?}");
    assert_eq!(*all.last().unwrap(), 80.0);
    assert!(!f.app.wheel_scroll_animating());
}

/// A trackpad's (or a high-resolution wheel's) pixel delta scrolls directly.
#[test]
fn a_pixel_delta_stays_direct() {
    let mut f = mount();
    wheel(
        &mut f.app,
        (100.0, 150.0),
        MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, -37.0)),
    );
    assert_eq!(top(&f.app, f.scroller), START + 37.0);
    assert!(!f.app.wheel_scroll_animating());
    assert_eq!(*f.fired.borrow(), [START + 37.0]);
}

/// The app's escape hatch: off, a notch scrolls its 40px in the frame it
/// arrives, as before #2287.
#[test]
fn turned_off_a_notch_jumps() {
    let mut f = mount();
    f.app.set_smooth_wheel_scroll(false);
    notch(&mut f.app);
    assert_eq!(top(&f.app, f.scroller), START + 40.0);
    assert!(!f.app.wheel_scroll_animating());
}

/// Turned off mid-glide, the glide ends at its target on the next frame
/// rather than losing the rest of the notch.
#[test]
fn turned_off_mid_glide_lands_on_the_target() {
    let mut f = mount();
    notch(&mut f.app);
    run(&mut f, 5);
    let mid = top(&f.app, f.scroller);
    assert!(mid > START && mid < START + 40.0, "mid-glide at {mid}");
    f.app.set_smooth_wheel_scroll(false);
    frame(&mut f.app);
    assert_eq!(top(&f.app, f.scroller), START + 40.0);
    assert!(!f.app.wheel_scroll_animating());
}

/// A programmatic `scroll_top` write mid-glide wins: the glide stops, and the
/// offset stays where the write put it.
#[test]
fn a_scroll_top_write_cancels_the_glide() {
    let mut f = mount();
    notch(&mut f.app);
    run(&mut f, 6);
    set_top(&mut f.app, f.scroller, 1200.0);
    let after = run(&mut f, 30);
    assert!(
        after.iter().all(|&s| s == 1200.0 - START),
        "the glide went on after the write: {after:?}"
    );
    assert!(!f.app.wheel_scroll_animating());
}

/// A press on the animating scroller's scrollbar stops the glide before the
/// thumb has moved, so the thumb stays under the pointer.
#[test]
fn a_scrollbar_press_cancels_the_glide() {
    use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
    let mut f = mount();
    notch(&mut f.app);
    run(&mut f, 6);
    let at = top(&f.app, f.scroller);
    let track = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        rinch_dom::paint::scrollbar::scrollbars(&d.tree, f.scroller, 1.0)
            .vertical
            .expect("a vertical bar")
    };
    let along = track.thumb_start(at) + track.thumb_len / 2.0;
    let x = (200.0 - (MARGIN + THICKNESS / 2.0)) as f32;
    f.app
        .handle_event(PlatformEvent::MouseMove { x, y: along as f32 }, VP, 1.0);
    f.app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y: along as f32,
            button: rinch_platform::MouseButton::Left,
        },
        VP,
        1.0,
    );
    assert!(
        f.app.scrollbar_drag.is_some(),
        "positive control: the thumb is held"
    );
    let after = run(&mut f, 30);
    assert!(
        after.iter().all(|&s| s == at - START),
        "the glide went on under a held thumb: {after:?}"
    );
    assert!(!f.app.wheel_scroll_animating());
}

/// A page (400px) holding a 100px list at its top; both scroll vertically.
fn mount_nested() -> (RinchApp, u64, u64) {
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let page = scope.create_element("div");
        page.set_attribute("id", "page");
        page.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 400px; height: 400px; \
             overflow-y: auto",
        );
        let list = scope.create_element("div");
        list.set_attribute("id", "list");
        list.set_attribute("style", "width: 300px; height: 100px; overflow-y: auto");
        let rows = scope.create_element("div");
        rows.set_attribute("style", "height: 400px");
        list.append_child(&rows);
        page.append_child(&list);
        let tail = scope.create_element("div");
        tail.set_attribute("style", "height: 2000px");
        page.append_child(&tail);
        page
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let page = by_id(&app, "page");
    let list = by_id(&app, "list");
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_animation_clock_ms(Some(T0));
    (app, page, list)
}

fn settle_glides(app: &mut RinchApp) {
    for _ in 0..60 {
        if !frame(app) {
            return;
        }
    }
    panic!("a glide never finished");
}

/// The notch picks its scroller when it arrives, as the direct wheel does:
/// over the list, the list glides and the page stays; with the list at its
/// end, the next notch chains to the page, which glides.
#[test]
fn nested_scrollers_pick_at_the_notch_and_chain_at_the_end() {
    let (mut app, page, list) = mount_nested();
    set_top(&mut app, list, 250.0);
    let over_list = (150.0, 50.0);
    wheel(&mut app, over_list, MouseScrollDelta::LineDelta(0.0, -1.0));
    settle_glides(&mut app);
    assert_eq!(top(&app, list), 290.0);
    assert_eq!(top(&app, page), 0.0, "the page did not move under the list");

    // 10px of the list left: this notch takes it there, and no further.
    wheel(&mut app, over_list, MouseScrollDelta::LineDelta(0.0, -1.0));
    settle_glides(&mut app);
    assert_eq!(top(&app, list), 300.0, "the list at its end");
    assert_eq!(top(&app, page), 0.0);

    // At its end: the page takes the notch, and glides.
    wheel(&mut app, over_list, MouseScrollDelta::LineDelta(0.0, -1.0));
    assert_eq!(top(&app, page), 0.0, "a glide, not a jump");
    frame(&mut app);
    frame(&mut app);
    frame(&mut app);
    let mid = top(&app, page);
    assert!(mid > 0.0 && mid < 40.0, "the page mid-glide at {mid}");
    settle_glides(&mut app);
    assert_eq!(top(&app, page), 40.0);
    assert_eq!(top(&app, list), 300.0);
}

/// The editor's caret reveal and a glide: typing mid-glide scrolls the pane
/// back to the caret, and the glide stops there rather than dragging the pane
/// away from the line being typed on. The reveal is a scroll write like any
/// other, so it wins. Text in the bundled Inter at a declared 16px / 24px.
#[test]
fn typing_mid_glide_reveals_the_caret_and_ends_the_glide() {
    const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
    let pane_id = Rc::new(std::cell::Cell::new(0));
    let pane_in = pane_id.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..120)
        .map(|i| format!("<p>{i} lorem ipsum dolor sit amet consectetur adipiscing elit</p>"))
        .collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let pane = scope.create_element("div");
        pane.set_attribute("style", "width: 500px; height: 500px; overflow-y: auto");
        let editor = handle_in.mount(scope);
        editor.set_styles(&[
            ("font-family", "sans-serif"),
            ("font-size", "16px"),
            ("line-height", "24px"),
        ]);
        pane.append_child(&editor);
        root.append_child(&pane);
        pane_in.set(pane.node_id().0);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let pane = pane_id.get();
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_animation_clock_ms(Some(T0));
    // The caret at the top of the document, focused; the pane scrolled far
    // below it.
    handle.set_selection(rinch_editor_core::Selection::cursor(
        rinch_editor_core::Pos(1),
    ));
    handle.focus();
    for _ in 0..4 {
        frame(&mut app);
    }
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    set_top(&mut app, pane, 1500.0);
    for _ in 0..2 {
        frame(&mut app);
    }
    assert_eq!(top(&app, pane), 1500.0, "control: nothing revealed yet");

    wheel(
        &mut app,
        (250.0, 250.0),
        MouseScrollDelta::LineDelta(0.0, -1.0),
    );
    for _ in 0..6 {
        frame(&mut app);
    }
    let mid = top(&app, pane);
    assert!(
        mid > 1500.0 && mid < 1540.0,
        "positive control: mid-glide at {mid}"
    );

    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyX,
            logical_key: Some("x".into()),
            text: Some("x".into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    for _ in 0..40 {
        frame(&mut app);
        if app.has_pending_layout() {
            app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        }
    }
    assert!(
        handle
            .doc()
            .child(0)
            .child(0)
            .text()
            .is_some_and(|t| t.starts_with('x')),
        "positive control: the key was typed"
    );
    let after = top(&app, pane);
    assert!(
        after < 100.0,
        "the pane is at {after}: the glide dragged it away from the caret"
    );
    assert!(!app.wheel_scroll_animating());
}

// ── Review of PR #2290 ──────────────────────────────────────────────────────

/// The DevTools window's app is sent no `AboutToWait`; its loop turn is
/// `devtools_turn`. A notch there must glide to its end and go idle, not sit
/// at the start with the glide "running" for ever.
#[test]
fn a_glide_in_the_devtools_window_runs_to_its_end() {
    let mut f = mount();
    notch(&mut f.app);
    let mut asked = 0;
    for _ in 0..40 {
        f.app
            .doc
            .as_ref()
            .unwrap()
            .borrow_mut()
            .advance_animation_clock_ms(FRAME_MS);
        if crate::shell::rinch_runtime::devtools_turn(&mut f.app, VP.0 as f32, VP.1 as f32) {
            asked += 1;
        }
    }
    assert_eq!(
        top(&f.app, f.scroller),
        START + 40.0,
        "the DevTools glide never moved"
    );
    assert!(!f.app.wheel_scroll_animating());
    assert!((19..40).contains(&asked), "redraws asked: {asked}");
}

/// The clock stepping back mid-glide (a wall-clock correction) neither
/// scrolls backwards nor stalls the glide: it carries on from where it was as
/// soon as the clock moves forward again.
#[test]
fn a_clock_step_back_mid_glide_neither_reverses_nor_stalls() {
    let mut f = mount();
    notch(&mut f.app);
    let first = run(&mut f, 10);
    let mid = *first.last().unwrap();
    assert!(
        mid > 0.0 && mid < 40.0,
        "positive control: mid-glide at {mid}"
    );
    f.app
        .doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .advance_animation_clock_ms(-5000.0);
    let rest = run(&mut f, 200);
    assert!(
        rest.iter().all(|&s| s >= mid),
        "it scrolled back past {mid}: {rest:?}"
    );
    assert_eq!(*rest.last().unwrap(), 40.0);
    assert!(
        rest.len() <= 15,
        "{} frames to finish after the step: {rest:?}",
        rest.len()
    );
}

/// A `lock_scroll` (#474) taken mid-glide, by an overlay elsewhere, locks the
/// gliding scroller out: the glide stops where it is.
#[test]
fn a_scroll_lock_mid_glide_stops_the_glide() {
    let mut f = mount();
    notch(&mut f.app);
    run(&mut f, 6);
    let at = top(&f.app, f.scroller);
    assert!(at > START, "positive control: it moved");
    let overlay = by_id(&f.app, "overlay");
    f.app
        .doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_scroll_locked(true, rinch_core::dom::NodeId(overlay));
    let after = run(&mut f, 30);
    assert!(
        after.iter().all(|&s| s == at - START),
        "the glide went on under the lock: {after:?}"
    );
    assert!(!f.app.wheel_scroll_animating());
}

/// At a fractional scale (1.25) a trackpad's physical pixels divide by the
/// scale (#299): 50 physical px are 40 logical, on the device grid.
#[test]
fn a_pixel_delta_divides_by_a_fractional_scale() {
    let mut f = mount();
    f.app.set_device_pixel_ratio(1.25);
    set_top(&mut f.app, f.scroller, 332.8);
    let s0 = top(&f.app, f.scroller);
    assert_eq!(s0, 332.8);
    let (x, y) = (100.0, 150.0);
    f.app
        .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
    let event = crate::shell::rinch_runtime::wheel_platform_event(
        MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, -50.0)),
        (x, y),
        1.25,
    )
    .unwrap();
    f.app.handle_event(event, VP, 1.0);
    assert_eq!(top(&f.app, f.scroller), s0 + 40.0);
}

/// At 1.25 a line glide's every frame is on the device grid (a whole number
/// of device pixels) and it lands on its 40px.
#[test]
fn a_line_glide_at_a_fractional_scale_stays_on_the_grid_and_lands() {
    let mut f = mount();
    f.app.set_device_pixel_ratio(1.25);
    set_top(&mut f.app, f.scroller, 332.8);
    let s0 = top(&f.app, f.scroller);
    notch(&mut f.app);
    let mut tops = Vec::new();
    for _ in 0..40 {
        let asked = frame(&mut f.app);
        tops.push(top(&f.app, f.scroller));
        if !asked {
            break;
        }
    }
    for t in &tops {
        let device = t * 1.25;
        assert!(
            (device - device.round()).abs() < 1e-6,
            "{t} is off the grid: {tops:?}"
        );
    }
    assert!(tops.windows(2).all(|w| w[1] >= w[0]), "{tops:?}");
    assert_eq!(*tops.last().unwrap(), s0 + 40.0, "{tops:?}");
    assert!(tops.len() > 15, "it glided: {tops:?}");
}
