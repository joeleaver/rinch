//! One hit test per question, shared across an event's consumers (#908): the
//! answers a press or release acts on must still be the ones a fresh
//! `hit_test` would give.
//!
//! A press used to hit-test the same point nine times — the `data-onmousedown`
//! dispatch, the editor, the focus claim, the draggable search, and five phases
//! of the click. `RinchApp::shared_hit` answers the repeats from the first,
//! keyed by the hit cache's generation. These fixtures pin the two ways a
//! remembered answer can go stale, each against the mutant that lets it:
//!
//! - a handler that changes the tree between two consumers of one event moves
//!   the generation, and the next consumer must re-test (mutant: ignore the
//!   generation);
//! - an animation tick moves a `transform` **without** moving the generation
//!   (the hit cache keys a transform by whether it is the identity, not by its
//!   value), so nothing may be remembered from one event to the next (mutant:
//!   do not clear the memo at the top of `handle_event`).

use super::*;
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);

fn frame(app: &mut RinchApp) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
}

fn counter(scope: &mut RenderScope, count: &Rc<Cell<u32>>) -> String {
    let c = count.clone();
    scope
        .register_handler(move || c.set(c.get() + 1))
        .0
        .to_string()
}

/// A box on top of another, both clickable. The top box's `data-onmousedown`
/// takes it out of the document, so the click — which a left press fires from
/// `MouseDown`, after that handler — lands on the box that is under the pointer
/// now: the one beneath. On `main` every phase of the click ran its own hit
/// test, so this held by construction; sharing one must not lose it.
#[test]
fn a_press_whose_mousedown_removes_the_target_clicks_what_is_beneath() {
    let top_clicks = Rc::new(Cell::new(0u32));
    let under_clicks = Rc::new(Cell::new(0u32));
    let (t, u) = (top_clicks.clone(), under_clicks.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 300px; height: 200px");
        let under = scope.create_element("div");
        under.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 300px; height: 200px",
        );
        under.set_attribute("data-rid", &counter(scope, &u));
        root.append_child(&under);

        let top = scope.create_element("div");
        top.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 30px; width: 100px; height: 80px",
        );
        top.set_attribute("data-rid", &counter(scope, &t));
        let me = top.clone();
        let remove = scope.register_handler(move || me.remove());
        top.set_attribute("data-onmousedown", &remove.0.to_string());
        root.append_child(&top);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    frame(&mut app);

    let (x, y) = (90.0, 70.0);
    let before = {
        let d = app.doc.as_ref().unwrap().borrow();
        super::hit_testing::hit_test(&d.tree, x, y)
    };
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, SIZE, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, SIZE, 1.0);

    let after = {
        let d = app.doc.as_ref().unwrap().borrow();
        super::hit_testing::hit_test(&d.tree, x, y)
    };
    assert_ne!(
        before, after,
        "positive control: the mousedown handler took the top box out"
    );
    assert_eq!(
        (top_clicks.get(), under_clicks.get()),
        (0, 1),
        "the click went to the box the press found before its mousedown \
         handler removed it, not to the one under the pointer now"
    );
}

/// A box moved out from under the pointer by a `transform` transition that
/// completes between a press and its release. The tick writes the transform
/// straight into `computed_style` and, since both ends are non-identity, moves
/// no hit-cache generation — so a hit remembered from the press would send the
/// release's `data-onmouseup` to the box that has left.
#[test]
fn a_release_after_a_transform_tick_is_judged_where_the_box_is_now() {
    let box_ups = Rc::new(Cell::new(0u32));
    let floor_ups = Rc::new(Cell::new(0u32));
    let (b, f) = (box_ups.clone(), floor_ups.clone());
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(
            ".mover { position: absolute; left: 0px; top: 0px; width: 100px; height: 100px;
                      transform: translateX(10px); transition: transform 1ms linear; }
             .mover.away { transform: translateX(400px); }",
        );
        style.append_child(&css);
        root.append_child(&style);
        let floor = scope.create_element("div");
        floor.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 800px; height: 600px",
        );
        floor.set_attribute("data-onmouseup", &counter(scope, &f));
        root.append_child(&floor);
        let mover = scope.create_element("div");
        mover.set_attribute("class", "mover");
        mover.set_attribute("data-onmouseup", &counter(scope, &b));
        root.append_child(&mover);
        *out2.borrow_mut() = Some(mover);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    frame(&mut app);
    let mover = out.borrow().clone().unwrap();
    mover.set_attribute("class", "mover away");
    // Starts the transition; the box is still painted at its start value.
    frame(&mut app);

    let (x, y) = (60.0, 50.0);
    let hit_now = |app: &RinchApp| {
        let d = app.doc.as_ref().unwrap().borrow();
        super::hit_testing::hit_test(&d.tree, x, y)
    };
    assert_eq!(
        hit_now(&app),
        Some(mover.node_id().0),
        "precondition: the box is under the pointer before the tick"
    );
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, SIZE, 1.0);
    let generation = |app: &RinchApp| {
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .hit_cache
            .generation()
    };
    let before_tick = generation(&app);
    std::thread::sleep(std::time::Duration::from_millis(30));
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    assert_ne!(
        hit_now(&app),
        Some(mover.node_id().0),
        "positive control: the tick moved the box out from under the pointer"
    );
    assert_eq!(
        generation(&app),
        before_tick,
        "the fixture's premise: the tick moved no hit-cache generation (if it \
         did, the generation alone would protect the release and this fixture \
         would pin nothing)"
    );
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, SIZE, 1.0);
    assert_eq!(
        (box_ups.get(), floor_ups.get()),
        (0, 1),
        "the release reached the box the press had found, which the tick had \
         moved away"
    );
}

/// Review of #1003: a `data-onmousedown` handler that scrolls the scroller the
/// press is over. `set_scroll_top` moves the generation (`mark_scrolled`), so
/// the click that follows in the same `MouseDown` must land on the row that is
/// under the pointer after the scroll, as a fresh hit test would.
#[test]
fn a_mousedown_that_scrolls_clicks_the_row_now_under_the_pointer() {
    let clicks: Vec<Rc<Cell<u32>>> = (0..6).map(|_| Rc::new(Cell::new(0))).collect();
    let c2 = clicks.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "overflow-y: auto; width: 200px; height: 100px; display: block",
        );
        for (i, c) in c2.iter().enumerate() {
            let row = scope.create_element("div");
            row.set_attribute("style", "height: 50px; width: 200px; display: block");
            row.set_attribute("data-rid", &counter(scope, c));
            if i == 0 {
                let s = scroller.clone();
                let h = scope.register_handler(move || s.set_scroll_top(100.0));
                row.set_attribute("data-onmousedown", &h.0.to_string());
            }
            scroller.append_child(&row);
        }
        root.append_child(&scroller);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    frame(&mut app);
    let (x, y) = (50.0, 25.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, SIZE, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, SIZE, 1.0);
    let got: Vec<u32> = clicks.iter().map(|c| c.get()).collect();
    assert_eq!(got, vec![0, 0, 1, 0, 0, 0], "click went to {got:?}");
}

/// Review of #1003: one event asks about two points. A release under the drag
/// threshold fires `data-onmouseup` at the release point, then the deferred
/// click at the *press* point; the memo must not answer the second from the
/// first (mutant: drop the point from the key).
#[test]
fn a_release_asks_two_points_and_each_gets_its_own_answer() {
    let a = Rc::new(Cell::new(0u32));
    let b = Rc::new(Cell::new(0u32));
    let (a2, b2) = (a.clone(), b.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 400px; height: 200px");
        let left = scope.create_element("div");
        left.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 100px; height: 100px",
        );
        left.set_attribute("draggable", "true");
        left.set_attribute("data-rid", &counter(scope, &a2));
        root.append_child(&left);
        let right = scope.create_element("div");
        right.set_attribute(
            "style",
            "position: absolute; left: 100px; top: 0px; width: 100px; height: 100px",
        );
        right.set_attribute("data-rid", &counter(scope, &b2));
        right.set_attribute("data-onmouseup", &counter(scope, &Rc::new(Cell::new(0))));
        root.append_child(&right);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    frame(&mut app);
    let button = MouseButton::Left;
    app.handle_event(
        PlatformEvent::MouseDown {
            x: 98.0,
            y: 50.0,
            button,
        },
        SIZE,
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x: 101.0,
            y: 50.0,
            button,
        },
        SIZE,
        1.0,
    );
    assert_eq!((a.get(), b.get()), (1, 0));
}
