//! The three other desktop gestures with #381's shape end on the same two
//! proofs of a missed release (issue #1028).
//!
//! #381 taught `handle_event` that a **left press** while a pointer-capture
//! drag is live, or the **window losing focus**, means the drag's release went
//! somewhere else (`RinchApp::heal_missed_release`). Three more pieces of
//! input state are armed by a press and cleared only by `MouseUp`:
//!
//! - an element-to-element drag (`pending_drag` before its threshold,
//!   `active_dnd` after): a stranded one kept following the pointer, and the
//!   **next unrelated click's release dropped it** — `data-ondrop` for a drop
//!   the user never made. Healed the way Escape and `PointerCancel` cancel it:
//!   `data-ondragleave` on the target, `data-ondragend` on the source, **no**
//!   `data-ondrop` — HTML's cancelled drag. A stranded *pending* drag is
//!   discarded, not clicked: its release is the one that went missing.
//! - a scrollbar-thumb drag, which kept scrolling the container on every move
//!   until some release arrived. Just ended.
//! - a read-only text-selection drag, which kept extending the selection. The
//!   gesture ends; the selection it made is kept, as a release would keep it.
//!
//! Every coordinate is off the fixed points the code has: the arming press,
//! the last move, the stray press and the moves after it are different points,
//! so state that survived reads a different number.

use super::*;
use std::cell::Cell;

const WINDOW: (u32, u32) = (800, 600);

fn send(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, WINDOW, 1.0);
}

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    send(
        app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
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

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    send(app, PlatformEvent::MouseMove { x, y });
}

fn counter(scope: &mut RenderScope, count: &Rc<Cell<u32>>) -> String {
    let c = count.clone();
    scope
        .register_handler(move || c.set(c.get() + 1))
        .0
        .to_string()
}

// ── Element-to-element drag ────────────────────────────────────────────────

/// Source (40,40)-(140,80), a second draggable (40,460)-(140,500), drop target
/// (400,200)-(500,300), and a probe button (600,40)-(700,80) whose
/// `data-onmousedown` records how many `data-ondragend`s had fired when the
/// press reached it.
#[derive(Default)]
struct Dnd {
    drops: Rc<Cell<u32>>,
    leaves: Rc<Cell<u32>>,
    ends: Rc<Cell<u32>>,
    source_clicks: Rc<Cell<u32>>,
    other_clicks: Rc<Cell<u32>>,
    ends_seen_by_probe: Rc<RefCell<Vec<u32>>>,
}

const SOURCE: (f32, f32) = (61.0, 57.0);
const OTHER_SOURCE: (f32, f32) = (73.0, 481.0);
const OVER_TARGET: (f32, f32) = (437.0, 243.0);
const ON_TARGET_LIST: (f32, f32) = (441.0, 251.0);
const PROBE: (f32, f32) = (653.0, 61.0);
const EMPTY: (f32, f32) = (611.0, 457.0);

fn mount_dnd() -> (RinchApp, Rc<Dnd>) {
    let log = Rc::new(Dnd::default());
    let l = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");

        let boxed = |scope: &mut RenderScope, style: &str| {
            let n = scope.create_element("div");
            n.set_attribute("style", &format!("position: absolute; {style}"));
            n
        };

        let source = boxed(scope, "left: 40px; top: 40px; width: 100px; height: 40px");
        source.set_attribute("draggable", "true");
        source.set_attribute("data-ondragend", &counter(scope, &l.ends));
        source.set_attribute("data-rid", &counter(scope, &l.source_clicks));
        root.append_child(&source);

        let other = boxed(scope, "left: 40px; top: 460px; width: 100px; height: 40px");
        other.set_attribute("draggable", "true");
        other.set_attribute("data-rid", &counter(scope, &l.other_clicks));
        root.append_child(&other);

        let target = boxed(
            scope,
            "left: 400px; top: 200px; width: 100px; height: 100px",
        );
        target.set_attribute("data-ondrop", &counter(scope, &l.drops));
        target.set_attribute("data-ondragleave", &counter(scope, &l.leaves));
        root.append_child(&target);

        let probe = boxed(scope, "left: 600px; top: 40px; width: 100px; height: 40px");
        let rid = scope.register_handler({
            let (ends, seen) = (l.ends.clone(), l.ends_seen_by_probe.clone());
            move || seen.borrow_mut().push(ends.get())
        });
        probe.set_attribute("data-onmousedown", &rid.0.to_string());
        root.append_child(&probe);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, log)
}

/// Press the source, carry it over the target, and lose the release.
fn strand_a_dom_drag(app: &mut RinchApp) {
    press(app, SOURCE);
    move_to(app, (93.0, 71.0));
    move_to(app, OVER_TARGET);
    assert!(
        app.active_dnd
            .as_ref()
            .is_some_and(|d| d.over_target.is_some()),
        "precondition: a DOM drag is live over the target"
    );
}

/// Positive control: a drag whose release arrives drops, once, every time.
#[test]
fn an_ordinary_dom_drag_still_drops() {
    let (mut app, log) = mount_dnd();
    for round in 1..=2 {
        strand_a_dom_drag(&mut app);
        release(&mut app, OVER_TARGET);
        assert_eq!(log.drops.get(), round, "round {round} drops");
        assert_eq!(log.ends.get(), round);
    }
}

/// The issue as measured: a later ordinary click on the target list dropped
/// the stranded drag. Now its press cancels it — `ondragleave` + `ondragend`,
/// no `ondrop` — and its release belongs to the new press alone.
#[test]
fn a_left_press_cancels_a_stranded_dom_drag_instead_of_dropping_it() {
    let (mut app, log) = mount_dnd();
    strand_a_dom_drag(&mut app);

    press(&mut app, ON_TARGET_LIST);
    assert!(app.active_dnd.is_none(), "the press ends the stranded drag");
    assert_eq!(log.ends.get(), 1, "the source hears ondragend");
    assert_eq!(log.leaves.get(), 1, "the target hears ondragleave");
    release(&mut app, ON_TARGET_LIST);
    assert_eq!(log.drops.get(), 0, "nobody dropped anything");
    assert_eq!(log.ends.get(), 1, "one ending, not two");
}

/// The cancel lands before the press is dispatched: the press's own
/// `data-onmousedown` already sees the drag ended.
#[test]
fn the_stranded_dom_drag_is_cancelled_before_the_press_is_dispatched() {
    let (mut app, log) = mount_dnd();
    strand_a_dom_drag(&mut app);

    press(&mut app, PROBE);
    assert_eq!(*log.ends_seen_by_probe.borrow(), vec![1]);
    release(&mut app, PROBE);
    assert_eq!(log.drops.get(), 0);
}

/// A stray press on **another draggable** used to replace `pending_drag`,
/// turn its own release into a click, and leave the stale drag live for one
/// more click. The new press is now a gesture of its own and nothing else.
#[test]
fn a_press_on_another_draggable_cancels_the_stranded_drag_and_clicks_itself() {
    let (mut app, log) = mount_dnd();
    strand_a_dom_drag(&mut app);

    press(&mut app, OTHER_SOURCE);
    release(&mut app, OTHER_SOURCE);
    assert!(app.active_dnd.is_none(), "no stale drag survives the click");
    assert_eq!(log.other_clicks.get(), 1, "the new press is a click");
    assert_eq!((log.drops.get(), log.ends.get()), (0, 1));

    // And the one after it drops nothing either.
    press(&mut app, ON_TARGET_LIST);
    release(&mut app, ON_TARGET_LIST);
    assert_eq!(log.drops.get(), 0);
}

/// A press on a draggable whose release went missing before the threshold was
/// crossed is not a click: the next press elsewhere discards it, and its own
/// release clicks nothing at the old press position.
#[test]
fn a_stranded_pending_drag_is_discarded_not_clicked() {
    let (mut app, log) = mount_dnd();
    press(&mut app, SOURCE);
    assert!(app.pending_drag.is_some(), "precondition: a pending drag");

    press(&mut app, EMPTY);
    assert!(app.pending_drag.is_none(), "the press discards it");
    release(&mut app, EMPTY);
    assert_eq!(log.source_clicks.get(), 0, "the source was never clicked");
}

/// A blur ends a live DOM drag the same way (the #381 trade-off), and the
/// release that follows the refocus drops nothing.
#[test]
fn a_window_blur_cancels_a_live_dom_drag() {
    let (mut app, log) = mount_dnd();
    strand_a_dom_drag(&mut app);

    send(&mut app, PlatformEvent::WindowFocus(false));
    assert!(app.active_dnd.is_none());
    assert_eq!((log.ends.get(), log.leaves.get()), (1, 1));
    send(&mut app, PlatformEvent::WindowFocus(true));
    release(&mut app, ON_TARGET_LIST);
    assert_eq!(log.drops.get(), 0);
    assert_eq!(log.ends.get(), 1);
}

/// Only a left press is proof: a right press during a live DOM drag is a chord
/// the user can really make.
#[test]
fn a_right_press_leaves_a_live_dom_drag_alone() {
    let (mut app, log) = mount_dnd();
    strand_a_dom_drag(&mut app);
    send(
        &mut app,
        PlatformEvent::MouseDown {
            x: EMPTY.0,
            y: EMPTY.1,
            button: MouseButton::Right,
        },
    );
    assert!(app.active_dnd.is_some());
    assert_eq!(log.ends.get(), 0);
}

// ── Scrollbar-thumb drag ───────────────────────────────────────────────────

/// A 200x200 `overflow-y: auto` box at (0,200) holding 1000px of content, and
/// a plain button beside it.
fn mount_scroller() -> (RinchApp, usize) {
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 200px; width: 200px; height: 200px; \
             overflow-y: auto",
        );
        scroller.set_attribute("id", "scroller");
        let content = scope.create_element("div");
        content.set_attribute("style", "height: 1000px");
        scroller.append_child(&content);
        root.append_child(&scroller);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("scroller"))
            .map(|(id, _)| id)
            .unwrap()
    };
    (app, id)
}

fn scroll_top(app: &RinchApp, id: usize) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

/// Press the thumb, drag it, lose the release.
fn strand_a_scrollbar_drag(app: &mut RinchApp, id: usize) -> f64 {
    press(app, (195.0, 213.0));
    assert!(app.scrollbar_drag.is_some(), "precondition: thumb pressed");
    let before = scroll_top(app, id);
    move_to(app, (195.0, 247.0));
    let after = scroll_top(app, id);
    assert!(
        after > before,
        "precondition: the drag scrolls ({before} -> {after})"
    );
    after
}

#[test]
fn a_left_press_ends_a_stranded_scrollbar_drag() {
    let (mut app, id) = mount_scroller();
    let stranded_at = strand_a_scrollbar_drag(&mut app, id);

    press(&mut app, (611.0, 457.0));
    assert!(
        app.scrollbar_drag.is_none(),
        "the press ends the thumb drag"
    );
    move_to(&mut app, (617.0, 529.0));
    assert_eq!(
        scroll_top(&app, id),
        stranded_at,
        "a move after the stray press no longer scrolls the container"
    );
    release(&mut app, (617.0, 529.0));
}

#[test]
fn a_window_blur_ends_a_live_scrollbar_drag() {
    let (mut app, id) = mount_scroller();
    let stranded_at = strand_a_scrollbar_drag(&mut app, id);

    send(&mut app, PlatformEvent::WindowFocus(false));
    move_to(&mut app, (195.0, 311.0));
    assert_eq!(
        scroll_top(&app, id),
        stranded_at,
        "no scroll after the blur"
    );
}

// ── Read-only text selection ───────────────────────────────────────────────

fn mount_text() -> RinchApp {
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let p = scope.create_element("p");
        p.set_attribute(
            "style",
            "margin: 0px; width: 600px; font-size: 16px; line-height: 20px; user-select: text",
        );
        let t =
            scope.create_text("Stranded selections keep growing with every move of the pointer");
        p.append_child(&t);
        root.append_child(&p);
        let other = scope.create_element("div");
        other.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 460px; width: 100px; height: 40px",
        );
        other.set_attribute("draggable", "true");
        root.append_child(&other);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

fn focus_offset(app: &RinchApp) -> usize {
    app.text_selection
        .as_ref()
        .expect("a selection")
        .focus_offset
}

/// Press in the text, drag across some of it, lose the release.
fn strand_a_text_selection(app: &mut RinchApp) -> usize {
    press(app, (23.0, 9.0));
    assert!(app.text_selecting, "precondition: selecting");
    let anchor = focus_offset(app);
    move_to(app, (187.0, 11.0));
    let focus = focus_offset(app);
    assert_ne!(
        anchor, focus,
        "precondition: the move extends the selection"
    );
    focus
}

#[test]
fn a_window_blur_ends_a_text_selection_drag_and_keeps_the_selection() {
    let mut app = mount_text();
    let focus = strand_a_text_selection(&mut app);

    send(&mut app, PlatformEvent::WindowFocus(false));
    move_to(&mut app, (371.0, 13.0));
    assert_eq!(focus_offset(&app), focus, "the move no longer extends it");
    assert!(!app.text_selecting);
}

/// A press that never reaches the click path — here one on a draggable, which
/// only arms a pending drag — used to leave `text_selecting` armed underneath.
#[test]
fn a_left_press_ends_a_stranded_text_selection_drag() {
    let mut app = mount_text();
    strand_a_text_selection(&mut app);

    press(&mut app, (73.0, 481.0));
    assert!(!app.text_selecting, "the press ends the selection drag");
    release(&mut app, (73.0, 481.0));
}
