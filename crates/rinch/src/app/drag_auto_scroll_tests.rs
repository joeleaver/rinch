//! Auto-scroll during an element drag (`draggable="true"` + `data-ondrag*`).
//!
//! A drag whose pointer rests in the band along the top or bottom edge of a
//! scroll container under it scrolls that container, a frame at a time from
//! the `AboutToWait` clock, faster nearer the edge, until the pointer leaves
//! the band or the drag ends. The rows scrolling under the still pointer are
//! drop targets, and the one that arrives under it must hear
//! `data-ondragenter` and `data-ondragover` without the pointer moving.
//!
//! Every fixture drives the real event path: a press, moves, `AboutToWait`
//! frames and a release, through `handle_event`.

use super::*;
use std::cell::Cell;
use std::time::Duration;

const WINDOW: (u32, u32) = (800, 600);
const ROWS: usize = 20;
const ROW_HEIGHT: f32 = 40.0;
/// The list's box: (0,100) to (300,300), 200px showing 800px of rows.
const LIST_TOP: f32 = 100.0;
const LIST_BOTTOM: f32 = 300.0;
/// Inside the list, clear of its scrollbar.
const X: f32 = 100.0;

struct Fixture {
    app: RinchApp,
    list: u64,
    /// The row index each `data-ondragover` fired on, in order.
    over: Rc<RefCell<Vec<usize>>>,
    /// The row index each `data-ondragenter` fired on, in order.
    entered: Rc<RefCell<Vec<usize>>>,
    /// `data-onscroll` on the list.
    scrolls: Rc<Cell<u32>>,
}

/// A 300x200 `overflow-y: auto` list at (0,100) of twenty 40px rows, each a
/// drop target, and a draggable source at (500,40)-(600,80) outside it.
/// `list_style` is appended to the list's own style.
fn fixture_with(list_style: &str, page_height: Option<f32>) -> Fixture {
    let over: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let entered: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let scrolls = Rc::new(Cell::new(0u32));
    let (over_sink, enter_sink, scroll_sink) = (over.clone(), entered.clone(), scrolls.clone());
    let list_style = list_style.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let height = page_height.unwrap_or(600.0);
        root.set_attribute(
            "style",
            &format!("position: relative; width: 800px; height: {height}px"),
        );

        let source = scope.create_element("div");
        source.set_attribute(
            "style",
            "position: absolute; left: 500px; top: 40px; width: 100px; height: 40px",
        );
        source.set_attribute("draggable", "true");
        let end = events::register_handler(Rc::new(|| {}));
        source.set_attribute("data-ondragend", &end.0.to_string());
        root.append_child(&source);

        let list = scope.create_element("div");
        list.set_attribute(
            "style",
            &format!(
                "position: absolute; left: 0px; top: {LIST_TOP}px; width: 300px; \
                 height: {}px; overflow-y: auto; {list_style}",
                LIST_BOTTOM - LIST_TOP
            ),
        );
        list.set_attribute("id", "list");
        let sink = scroll_sink.clone();
        let on_scroll = events::register_scroll_handler(
            (move |_: events::ScrollEvent| {
                sink.set(sink.get() + 1);
            })
            .into(),
        );
        list.set_attribute("data-onscroll", &on_scroll.0.to_string());
        for i in 0..ROWS {
            let row = scope.create_element("div");
            row.set_attribute("style", &format!("height: {ROW_HEIGHT}px"));
            let drop = events::register_handler(Rc::new(|| {}));
            row.set_attribute("data-ondrop", &drop.0.to_string());
            let sink = over_sink.clone();
            let over = events::register_handler(Rc::new(move || sink.borrow_mut().push(i)));
            row.set_attribute("data-ondragover", &over.0.to_string());
            let sink = enter_sink.clone();
            let enter = events::register_handler(Rc::new(move || sink.borrow_mut().push(i)));
            row.set_attribute("data-ondragenter", &enter.0.to_string());
            list.append_child(&row);
        }
        root.append_child(&list);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let list = find_by_id(&app, "list");
    Fixture {
        app,
        list,
        over,
        entered,
        scrolls,
    }
}

fn fixture() -> Fixture {
    fixture_with("", None)
}

fn find_by_id(app: &RinchApp, wanted: &str) -> u64 {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(wanted))
        .map(|(id, _)| id)
        .unwrap()
}

fn send(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, WINDOW, 1.0)
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    send(app, PlatformEvent::MouseMove { x, y });
}

fn frame(app: &mut RinchApp) -> Vec<AppAction> {
    send(app, PlatformEvent::AboutToWait)
}

fn asks_for_a_frame(actions: &[AppAction]) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, AppAction::RequestRedraw))
}

fn scroll_top(app: &RinchApp, id: u64) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

fn set_scroll_top(app: &mut RinchApp, id: u64, top: f64) {
    let mut d = app.doc.as_ref().unwrap().borrow_mut();
    d.tree.nodes[id].scroll_offset.1 = top;
    d.tree.mark_scrolled(id);
}

/// Press the source and cross the threshold: the drag is live, over nothing.
fn start_drag(app: &mut RinchApp) {
    send(
        app,
        PlatformEvent::MouseDown {
            x: 520.0,
            y: 55.0,
            button: MouseButton::Left,
        },
    );
    move_to(app, (540.0, 70.0));
    assert!(app.active_dnd.is_some(), "the drag went live");
}

fn release(app: &mut RinchApp, (x, y): (f32, f32)) {
    send(
        app,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    );
}

/// A frame later than the last by at least `ms`, so the tick has time to
/// scroll for.
fn frame_after(app: &mut RinchApp, ms: u64) -> Vec<AppAction> {
    std::thread::sleep(Duration::from_millis(ms));
    frame(app)
}

#[test]
fn a_pointer_resting_in_the_bottom_band_keeps_scrolling() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 8.0));
    assert_eq!(
        scroll_top(&f.app, f.list),
        0.0,
        "a move alone scrolls nothing"
    );

    let actions = frame(&mut f.app);
    let first = scroll_top(&f.app, f.list);
    assert!(first > 0.0, "the first frame in the band scrolls");
    assert!(
        asks_for_a_frame(&actions),
        "and asks for the next frame, so a still pointer keeps scrolling"
    );

    let mut last = first;
    for _ in 0..3 {
        let actions = frame_after(&mut f.app, 20);
        let now = scroll_top(&f.app, f.list);
        assert!(now > last, "each frame scrolls on: {last} -> {now}");
        assert!(asks_for_a_frame(&actions));
        last = now;
    }
    assert!(
        f.scrolls.get() >= 4,
        "every frame that scrolled fired onscroll"
    );
}

#[test]
fn the_top_band_scrolls_up() {
    let mut f = fixture();
    set_scroll_top(&mut f.app, f.list, 300.0);
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_TOP + 8.0));
    frame(&mut f.app);
    assert!(scroll_top(&f.app, f.list) < 300.0);
}

#[test]
fn nearer_the_edge_is_faster() {
    let first_frame = |y: f32| {
        let mut f = fixture();
        start_drag(&mut f.app);
        move_to(&mut f.app, (X, y));
        frame(&mut f.app);
        scroll_top(&f.app, f.list)
    };
    let near = first_frame(LIST_BOTTOM - 2.0);
    let far = first_frame(LIST_BOTTOM - 24.0);
    assert!(far > 0.0, "24px in is still in the band");
    assert!(near > far, "2px from the edge {near} > 24px from it {far}");
}

#[test]
fn the_middle_of_the_list_does_not_scroll() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, (LIST_TOP + LIST_BOTTOM) / 2.0));
    frame(&mut f.app);
    frame_after(&mut f.app, 20);
    assert_eq!(scroll_top(&f.app, f.list), 0.0);
    assert!(f.app.drag_auto_scroll_at.is_none());
}

#[test]
fn leaving_the_band_stops_it() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    frame_after(&mut f.app, 20);
    let scrolled = scroll_top(&f.app, f.list);
    assert!(scrolled > 0.0);

    move_to(&mut f.app, (X, (LIST_TOP + LIST_BOTTOM) / 2.0));
    frame(&mut f.app);
    frame_after(&mut f.app, 20);
    assert_eq!(
        scroll_top(&f.app, f.list),
        scrolled,
        "out of the band, still"
    );
    assert!(f.app.drag_auto_scroll_at.is_none());
}

#[test]
fn ending_the_drag_stops_it() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    release(&mut f.app, (X, LIST_BOTTOM - 4.0));
    assert!(f.app.active_dnd.is_none());
    assert!(
        f.app.drag_auto_scroll_at.is_none(),
        "the drop reset the clock"
    );
    let dropped_at = scroll_top(&f.app, f.list);
    frame_after(&mut f.app, 20);
    assert_eq!(scroll_top(&f.app, f.list), dropped_at);
}

#[test]
fn escape_stops_it_too() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    send(
        &mut f.app,
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
    );
    assert!(f.app.active_dnd.is_none(), "Escape cancelled the drag");
    let cancelled_at = scroll_top(&f.app, f.list);
    frame_after(&mut f.app, 20);
    assert_eq!(scroll_top(&f.app, f.list), cancelled_at);
}

/// The row under a still pointer changes as the list scrolls, and the drag
/// follows it: the arriving row hears `data-ondragenter` and
/// `data-ondragover`, with no pointer move between.
#[test]
fn the_row_scrolled_under_a_still_pointer_hears_dragover() {
    let mut f = fixture();
    start_drag(&mut f.app);
    let y = LIST_BOTTOM - 2.0;
    move_to(&mut f.app, (X, y));
    // 198px into the list: row 4 (160..200).
    assert_eq!(f.entered.borrow().as_slice(), &[4]);
    assert_eq!(f.over.borrow().last(), Some(&4));

    // Scroll until another row is under the pointer, without moving it.
    for _ in 0..40 {
        frame_after(&mut f.app, 20);
        if scroll_top(&f.app, f.list) >= f64::from(ROW_HEIGHT) {
            break;
        }
    }
    let top = scroll_top(&f.app, f.list) as f32;
    assert!(top >= ROW_HEIGHT, "the list scrolled a row or more: {top}");
    let under = ((y - LIST_TOP + top) / ROW_HEIGHT) as usize;
    assert!(under > 4);
    assert_eq!(
        f.over.borrow().last(),
        Some(&under),
        "the row now under the pointer heard dragover"
    );
    assert!(
        f.entered.borrow().contains(&under),
        "and dragenter: {:?}",
        f.entered.borrow()
    );

    // The drop lands on that row, not the one the pointer first pointed at.
    let target = f.app.active_dnd.as_ref().unwrap().over_target;
    let row = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        d.tree.nodes[f.list].children[under]
    };
    assert_eq!(target, Some(row));
}

#[test]
fn a_list_at_its_end_scrolls_no_further_and_lets_the_clock_rest() {
    let mut f = fixture();
    set_scroll_top(&mut f.app, f.list, 600.0); // 800 - 200: the end
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    assert_eq!(scroll_top(&f.app, f.list), 600.0);
    assert!(
        f.app.drag_auto_scroll_at.is_none(),
        "nothing can scroll there, so the clock is not kept turning"
    );
}

/// Nested containers: a list at its end hands the band to the page it sits
/// in, which can still move.
#[test]
fn a_list_at_its_end_lets_the_page_around_it_scroll() {
    // The list at (0,400)-(300,600): its bottom band is the window's, and the
    // page under it is 1200px tall in a 600px window.
    let mut f = fixture_with("top: 400px;", Some(1200.0));
    let body = f.app.doc.as_ref().unwrap().borrow().tree.body_id;
    set_scroll_top(&mut f.app, f.list, 600.0);
    f.app.resolve_and_repaint(800.0, 600.0);
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, 596.0));
    frame(&mut f.app);
    assert_eq!(scroll_top(&f.app, f.list), 600.0, "the list is at its end");
    assert!(scroll_top(&f.app, body) > 0.0, "the page scrolled instead");
}

/// The wheel still scrolls through the helper it now shares with the drag.
#[test]
fn the_wheel_still_scrolls_the_list() {
    let mut f = fixture();
    send(
        &mut f.app,
        PlatformEvent::MouseWheel {
            x: X,
            y: 200.0,
            delta_x: 0.0,
            delta_y: -50.0,
        },
    );
    assert_eq!(scroll_top(&f.app, f.list), 50.0);
    assert_eq!(f.scrolls.get(), 1);
}

// ── Review fixtures (PR #1592 review) ──────────────────────────────────────

fn hit_tests(app: &RinchApp) -> u64 {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.perf.total().get(rinch_dom::perf::Counter::HitTests)
}

/// R1: at a fractional device scale the last fraction of a device pixel of a
/// range off the grid is unreachable (#1543), so `at < max` holds for ever:
/// the clock must still come to rest at the end.
#[test]
fn review_a_list_at_its_snapped_end_lets_the_clock_rest_at_a_fractional_scale() {
    // height 201: max = 800 - 201 = 599, not a multiple of 0.8.
    let mut f = fixture_with("height: 201px;", None);
    f.app.set_device_pixel_ratio(1.25);
    f.app.resolve_and_repaint(800.0, 600.0);
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_TOP + 201.0 - 2.0));
    let mut asked = false;
    for _ in 0..80 {
        asked = asks_for_a_frame(&frame_after(&mut f.app, 20));
        if !asked {
            break;
        }
    }
    let top = scroll_top(&f.app, f.list);
    assert!(top > 590.0, "it reached the end: {top}");
    assert!(
        !asked,
        "at its end ({top}) nothing can scroll, so no frame should be asked for"
    );
}

/// R1b: same, and the page around it must take over.
#[test]
fn review_a_list_at_its_snapped_end_hands_over_to_the_page_at_a_fractional_scale() {
    let mut f = fixture_with("top: 399px; height: 201px;", Some(1200.0));
    let body = f.app.doc.as_ref().unwrap().borrow().tree.body_id;
    f.app.set_device_pixel_ratio(1.25);
    f.app.resolve_and_repaint(800.0, 600.0);
    set_scroll_top(&mut f.app, f.list, 598.4); // the snapped end
    f.app.resolve_and_repaint(800.0, 600.0);
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, 596.0));
    for _ in 0..5 {
        frame_after(&mut f.app, 20);
    }
    assert!(
        scroll_top(&f.app, body) > 0.0,
        "the page should scroll once the list is at its (snapped) end; list at {}",
        scroll_top(&f.app, f.list)
    );
}

/// R2: a drag under a `lock_scroll` overlay must not scroll the page behind it.
#[test]
fn review_a_locked_page_is_not_auto_scrolled() {
    for locked in [false, true] {
        let mut f = fixture_with("", Some(1200.0));
        let body = f.app.doc.as_ref().unwrap().borrow().tree.body_id;
        if locked {
            let mut d = f.app.doc.as_ref().unwrap().borrow_mut();
            rinch_core::dom::DomDocument::set_scroll_locked(
                &mut *d,
                true,
                rinch_core::dom::NodeId(f.list),
            );
        }
        start_drag(&mut f.app);
        move_to(&mut f.app, (500.0, 596.0)); // over the page, not the list
        frame(&mut f.app);
        frame_after(&mut f.app, 20);
        let top = scroll_top(&f.app, body);
        if locked {
            assert_eq!(top, 0.0, "locked page scrolled");
        } else {
            assert!(top > 0.0, "positive control: unlocked page scrolls");
        }
    }
}

/// R3: a pointer-capture `Drag` (a slider) near a scroller's edge is not an
/// element drag and must not auto-scroll.
#[test]
fn review_a_pointer_capture_drag_does_not_auto_scroll() {
    let mut f = fixture();
    events::Drag::absolute().on_move(|_, _| {}).start();
    move_to(&mut f.app, (X, LIST_BOTTOM - 2.0));
    frame(&mut f.app);
    frame_after(&mut f.app, 20);
    events::Drag::cancel();
    assert_eq!(scroll_top(&f.app, f.list), 0.0);
}

/// R4: a drag resting outside every band asks for no frame (idle clock).
#[test]
fn review_a_drag_outside_every_band_asks_for_no_frame() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, 200.0));
    frame(&mut f.app);
    for _ in 0..3 {
        assert!(!asks_for_a_frame(&frame_after(&mut f.app, 5)));
    }
}

/// R5: an idle AboutToWait during a drag outside every band costs no hit test
/// (base: 0).
#[test]
fn review_hit_tests_per_idle_drag_frame() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, 200.0));
    frame(&mut f.app);
    let before = hit_tests(&f.app);
    for _ in 0..10 {
        frame(&mut f.app);
    }
    assert_eq!(
        hit_tests(&f.app) - before,
        0,
        "an idle drag frame reuses the drag's own hit test"
    );
}

/// The reuse is not blind: once a move puts the pointer in a band, the next
/// frame scrolls (and an idle frame after a move re-tests nothing either).
#[test]
fn an_idle_frame_after_a_move_reuses_the_moves_hit_test() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, 200.0));
    let before = hit_tests(&f.app);
    frame(&mut f.app);
    assert_eq!(hit_tests(&f.app) - before, 0);
    move_to(&mut f.app, (X, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    assert!(scroll_top(&f.app, f.list) > 0.0, "the band was seen");
}

/// The tick itself asks for the next frame. With the pointer at the band's
/// inner edge a frame scrolls well under a device pixel (about 0.3 px/s), so
/// nothing moves, nothing is dirtied and no repaint asks for a frame: only the
/// tick's own request keeps the clock turning until the remainder adds up.
#[test]
fn a_band_too_slow_to_move_a_pixel_a_frame_still_keeps_the_clock_turning() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 31.5));
    frame(&mut f.app);
    for _ in 0..3 {
        let actions = frame_after(&mut f.app, 5);
        assert_eq!(scroll_top(&f.app, f.list), 0.0, "a frame moved nothing");
        assert!(
            asks_for_a_frame(&actions),
            "the slow band keeps asking for frames"
        );
    }
}

/// A container partly below the window: its bottom band is measured on the
/// part inside the viewport, so a pointer near the window's bottom edge is in
/// it (unclamped, the band would be 100px below the window, out of reach).
#[test]
fn a_container_running_off_the_window_has_its_band_at_the_window_edge() {
    let mut f = fixture_with("top: 500px;", None);
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, 596.0));
    frame(&mut f.app);
    frame_after(&mut f.app, 20);
    assert!(scroll_top(&f.app, f.list) > 0.0);
}

/// A drag whose release went missing scrolls while it rests in a band, until
/// the missed-release heal ends it: the next left press cancels the drag, and
/// with it the auto-scroll.
#[test]
fn a_stranded_drag_stops_scrolling_when_the_missed_release_heals() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    assert!(
        scroll_top(&f.app, f.list) > 0.0,
        "it scrolls while stranded"
    );
    send(
        &mut f.app,
        PlatformEvent::MouseDown {
            x: X,
            y: LIST_BOTTOM - 4.0,
            button: MouseButton::Left,
        },
    );
    assert!(f.app.active_dnd.is_none(), "the heal cancelled the drag");
    let at = scroll_top(&f.app, f.list);
    assert!(!asks_for_a_frame(&frame_after(&mut f.app, 20)));
    assert_eq!(scroll_top(&f.app, f.list), at);
}

/// R6: a long first frame in the band scrolls one capped step, not the stall.
#[test]
fn review_a_stalled_clock_is_one_capped_step() {
    let mut f = fixture();
    start_drag(&mut f.app);
    move_to(&mut f.app, (X, LIST_BOTTOM - 1.0));
    frame(&mut f.app);
    let a = scroll_top(&f.app, f.list);
    std::thread::sleep(Duration::from_millis(400));
    frame(&mut f.app);
    let step = scroll_top(&f.app, f.list) - a;
    // max speed at 1px from edge ~ 1200*(31/32)^2 ≈ 1126 px/s × 0.05 ≈ 56px.
    assert!(step <= 61.0, "a 400ms stall scrolled {step}px");
    assert!(step > 30.0, "but a full capped step: {step}");
}

// ── Round-2 review fixtures (PR #1592) ─────────────────────────────────────

/// R2-1: a list slides away from a resting pointer (a `transform` transition
/// ending at a non-`none` value, which moves no hit-cache generation). After
/// the transition's last tick, a reused hit from mid-slide still names a row
/// of the list; its band is measured on `y` alone, so the list keeps
/// scrolling though the pointer is no longer over it.
#[test]
fn r2_a_list_that_slid_away_is_not_scrolled_from_a_mid_slide_hit() {
    let mut f = fixture_with("transition: transform 60ms linear;", None);
    start_drag(&mut f.app);
    let px = 250.0;
    move_to(&mut f.app, (px, LIST_BOTTOM - 4.0));
    frame(&mut f.app);
    {
        let mut d = f.app.doc.as_ref().unwrap().borrow_mut();
        rinch_core::dom::DomDocument::set_style(
            &mut *d,
            rinch_core::dom::NodeId(f.list),
            "transform",
            "translateX(400px)",
        );
    }
    f.app.resolve_and_repaint(800.0, 600.0);
    // Mid-slide: the list is still under x = 250, so it scrolls and the drag
    // is re-resolved there (the memo is taken mid-slide).
    frame_after(&mut f.app, 5);
    // Let the slide finish; the frame that runs its last tick hit-tests
    // afresh (motion) and finds the page, not the list.
    frame_after(&mut f.app, 120);
    let gen_before = f
        .app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .hit_cache
        .generation();
    let at = scroll_top(&f.app, f.list);
    let hits_before = hit_tests(&f.app);
    frame_after(&mut f.app, 20);
    let gen_after = f
        .app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .hit_cache
        .generation();
    eprintln!(
        "R2-1: at={at} after={} hits={} gen {gen_before}->{gen_after}",
        scroll_top(&f.app, f.list),
        hit_tests(&f.app) - hits_before
    );
    assert_eq!(
        scroll_top(&f.app, f.list),
        at,
        "the list slid away from the pointer and must not scroll"
    );
}

/// R2-2: every input that moves geometry without a pointer move must make
/// the next idle drag frame hit-test afresh (positive control for the reuse).
#[test]
fn r2_the_reuse_is_dropped_by_scroll_resize_dpr_and_dom_writes() {
    fn idle_frame_hits(f: &mut Fixture, change: &dyn Fn(&mut Fixture)) -> u64 {
        // Settle into reuse first.
        frame(&mut f.app);
        frame(&mut f.app);
        let before = hit_tests(&f.app);
        frame(&mut f.app);
        assert_eq!(hit_tests(&f.app) - before, 0, "control: idle frame reuses");
        change(f);
        let before = hit_tests(&f.app);
        frame(&mut f.app);
        hit_tests(&f.app) - before
    }
    type Change = Box<dyn Fn(&mut Fixture)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "scroll of another scroller",
            Box::new(|f: &mut Fixture| {
                let body = f.app.doc.as_ref().unwrap().borrow().tree.body_id;
                set_scroll_top(&mut f.app, body, 10.0);
            }),
        ),
        (
            "dom write from a timer",
            Box::new(|f: &mut Fixture| {
                let mut d = f.app.doc.as_ref().unwrap().borrow_mut();
                rinch_core::dom::DomDocument::set_style(
                    &mut *d,
                    rinch_core::dom::NodeId(f.list),
                    "left",
                    "40px",
                );
            }),
        ),
        (
            "window resize",
            Box::new(|f: &mut Fixture| {
                f.app.handle_event(
                    PlatformEvent::Resized {
                        width: 700,
                        height: 500,
                    },
                    (700, 500),
                    1.0,
                );
            }),
        ),
        (
            "dpr change",
            Box::new(|f: &mut Fixture| {
                f.app.set_device_pixel_ratio(1.5);
                f.app.resolve_and_repaint(800.0, 600.0);
            }),
        ),
    ];
    for (name, change) in cases {
        let mut f = fixture_with("", Some(1200.0));
        start_drag(&mut f.app);
        move_to(&mut f.app, (X, 200.0));
        let hits = idle_frame_hits(&mut f, &*change);
        eprintln!("R2-2 {name}: {hits} hit test(s)");
        assert!(hits >= 1, "{name}: the next drag frame reused a stale hit");
    }
}
