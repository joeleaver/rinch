//! #2766: content that a layout, a scroll or a removal moves under a mouse
//! that is not moving is hovered, as in Chrome, which hit-tests again at the
//! pointer's last position after a layout or scroll.
//!
//! Before, desktop rinch hit-tested only on a real move, so `:hover`,
//! `onmouseenter`/`onmouseleave` and the cursor stayed with whatever was
//! there before until the mouse moved. The case from deskrinch: a column of
//! notification popups, the pointer resting on the upper one; the upper one
//! is closed, the lower one slides up under the pointer, and is never told
//! the pointer entered it, so its expiry is never held.
//!
//! Each test drives the app in the desktop loop's order: events, then an
//! `AboutToWait` turn (where the frame's layout runs, and the hover is looked
//! at again), then the redraw.

use super::perf_expect::{SIZE, new_app};
use super::*;
use rinch_core::dom::NodeHandle;
use rinch_platform::CursorStyle;

type Log = Rc<RefCell<Vec<String>>>;

const CSS: &str = "html, body { margin: 0; }
    body { font: 14px/20px sans-serif; }
    #col { position: absolute; left: 10px; top: 10px; width: 200px; }
    .pop { height: 50px; user-select: text; }
    .pop:hover { background-color: rgb(255, 0, 0); }
    #up { cursor: crosshair; }
    #low { cursor: pointer; }
    #far { position: absolute; left: 400px; top: 10px; width: 300px; }
    #box { position: absolute; left: 400px; top: 400px; width: 100px; height: 60px; }
    #inner { height: 20px; }
    .scroller { position: absolute; left: 10px; top: 200px; width: 200px; height: 100px;
                overflow-y: auto; }
    .row { height: 20px; }
    .row:hover { background-color: rgb(0, 0, 255); }";

const RED: peniko::Color = peniko::Color::from_rgba8(255, 0, 0, 255);
const BLUE: peniko::Color = peniko::Color::from_rgba8(0, 0, 255, 255);

/// The popup column, a far-away text whose length a test changes, and a
/// 20-row scroller. Every element with an id hears enter and leave.
struct Scene {
    app: RinchApp,
    log: Log,
    up: NodeHandle,
    far: NodeHandle,
    scroller: NodeHandle,
    inner: NodeHandle,
}

fn mount() -> Scene {
    let handles: Rc<RefCell<Vec<NodeHandle>>> = Rc::default();
    let out = handles.clone();
    let mut app = new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(CSS));
        root.append_child(&style);
        let col = scope.create_element("div");
        col.set_attribute("id", "col");
        for id in ["up", "low"] {
            let pop = scope.create_element("div");
            pop.set_attribute("id", id);
            pop.set_attribute("class", "pop");
            pop.append_child(&scope.create_text(id));
            col.append_child(&pop);
            if id == "up" {
                out.borrow_mut().push(pop.clone());
            }
        }
        root.append_child(&col);
        let far = scope.create_element("div");
        far.set_attribute("id", "far");
        far.append_child(&scope.create_text("short"));
        root.append_child(&far);
        out.borrow_mut().push(far);
        let scroller = scope.create_element("div");
        scroller.set_attribute("class", "scroller");
        for i in 0..20 {
            let row = scope.create_element("div");
            row.set_attribute("class", "row");
            row.set_attribute("id", &format!("r{i}"));
            row.append_child(&scope.create_text(&format!("row {i}")));
            scroller.append_child(&row);
        }
        root.append_child(&scroller);
        out.borrow_mut().push(scroller);
        // A fixed-height box whose child sits at its top.
        let bx = scope.create_element("div");
        bx.set_attribute("id", "box");
        let inner = scope.create_element("div");
        inner.set_attribute("id", "inner");
        bx.append_child(&inner);
        root.append_child(&bx);
        out.borrow_mut().push(inner);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let log: Log = Rc::default();
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let ids: Vec<(u64, String)> = d
            .tree
            .nodes
            .iter()
            .filter_map(|(n, node)| node.attributes.get("id").map(|id| (n, id.clone())))
            .collect();
        for (n, id) in ids {
            for (attr, what) in [("data-onenter", "enter"), ("data-onleave", "leave")] {
                let log = log.clone();
                let id = id.clone();
                let h = events::register_handler(Rc::new(move || {
                    log.borrow_mut().push(format!("{what}:{id}"));
                }));
                d.set_attribute(rinch_core::dom::NodeId(n), attr, &h.0.to_string());
            }
        }
    }
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let h = handles.borrow();
    Scene {
        app,
        log,
        up: h[0].clone(),
        far: h[1].clone(),
        scroller: h[2].clone(),
        inner: h[3].clone(),
    }
}

impl Scene {
    fn move_to(&mut self, x: f32, y: f32) {
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
        self.turn();
    }

    /// One `AboutToWait` turn and the redraw it asked for. Returns the turn's
    /// actions.
    fn turn(&mut self) -> Vec<AppAction> {
        let actions = self.app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        }
        actions
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn node(&self, id: &str) -> u64 {
        let d = self.app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
            .map(|(n, _)| n)
            .unwrap_or_else(|| panic!("no #{id}"))
    }

    /// The ids of the connected elements that are `:hover`, in document
    /// order.
    fn hovered(&self) -> Vec<String> {
        let d = self.app.doc.as_ref().unwrap().borrow();
        let ids: Vec<u64> = d.tree.nodes.iter().map(|(n, _)| n).collect();
        ids.into_iter()
            .filter(|&n| {
                rinch_core::dom::DomDocument::is_connected(&*d, rinch_core::dom::NodeId(n))
                    && d.tree.nodes[n].is_hovered
            })
            .filter_map(|n| d.tree.nodes[n].attributes.get("id").cloned())
            .collect()
    }

    fn background(&self, id: &str) -> Option<peniko::Color> {
        let n = self.node(id);
        let d = self.app.doc.as_ref().unwrap().borrow();
        d.tree.get(n).unwrap().computed_style.background_color()
    }
}

fn cursor_of(actions: &[AppAction]) -> Option<CursorStyle> {
    actions.iter().rev().find_map(|a| match a {
        AppAction::SetCursor(c) => Some(*c),
        _ => None,
    })
}

/// The deskrinch case. The pointer rests on the upper popup; it is
/// discarded (what an rsx `if` or keyed `for` does), and the layout slides
/// the lower one up under the pointer. The lower one is entered, matches
/// `:hover` and restyles, and the cursor becomes its `pointer`. The removed
/// popup hears nothing (it left the document), and the column, which the
/// pointer never left, is neither left nor entered again.
#[test]
fn a_removal_sliding_a_sibling_under_a_still_pointer_enters_it() {
    let mut s = mount();
    s.move_to(50.0, 30.0);
    assert_eq!(s.take_log(), ["enter:col", "enter:up"], "control");
    assert_eq!(s.hovered(), ["col", "up"]);
    assert_ne!(
        s.background("low"),
        Some(RED),
        "control: #low not :hover yet"
    );

    s.up.discard();
    let actions = s.turn();
    assert_eq!(s.take_log(), ["enter:low"]);
    assert_eq!(s.hovered(), ["col", "low"]);
    assert_eq!(s.background("low"), Some(RED), ".pop:hover restyled #low");
    assert_eq!(
        cursor_of(&actions),
        Some(CursorStyle::Pointer),
        "{actions:?}"
    );
    assert!(
        actions.contains(&AppAction::RequestRedraw),
        "the restyle is drawn: {actions:?}"
    );

    // Settled: the next turns look at nothing again and fire nothing.
    let actions = s.turn();
    assert!(s.take_log().is_empty());
    assert!(!actions.contains(&AppAction::RequestRedraw), "{actions:?}");

    // A real move away leaves #low and the column, innermost first, and
    // `:hover` goes from both.
    s.move_to(300.0, 150.0);
    assert_eq!(s.take_log(), ["leave:low", "leave:col"]);
    assert_eq!(s.hovered(), Vec::<String>::new());
}

/// The same with the popup detached (`remove`, which keeps the node for
/// reuse): a detached element keeps its attributes and handlers, and is
/// still not told it was left.
#[test]
fn a_detached_hovered_element_is_not_left_and_its_sibling_is_entered() {
    let mut s = mount();
    s.move_to(50.0, 30.0);
    s.take_log();
    s.up.remove();
    s.turn();
    assert_eq!(s.take_log(), ["enter:low"]);
    assert_eq!(s.hovered(), ["col", "low"]);
}

/// The removed element's side: when nothing slides under the pointer, its
/// ancestors that the pointer is no longer over are left and drop `:hover`.
/// Here the last popup is removed, the column shrinks to nothing, and the
/// pointer is over the page.
#[test]
fn ancestors_a_removal_leaves_the_pointer_outside_are_left() {
    let mut s = mount();
    s.move_to(50.0, 80.0);
    assert_eq!(s.take_log(), ["enter:col", "enter:low"], "control");
    let low = s.node("low");
    {
        let doc = s.app.doc.clone().unwrap();
        rinch_core::dom::DomDocument::discard_node(
            &mut *doc.borrow_mut(),
            rinch_core::dom::NodeId(low),
        );
    }
    s.turn();
    // #up is still at 10..60: the pointer at y 80 is below the column now.
    assert_eq!(s.take_log(), ["leave:col"]);
    assert_eq!(s.hovered(), Vec::<String>::new());
}

/// A wheel scroll under a still pointer: the row scrolled away is left and
/// the row scrolled under the pointer entered, `:hover` moves with them.
#[test]
fn a_scroll_under_a_still_pointer_leaves_and_enters() {
    let mut s = mount();
    // Row 2 of the scroller (200 + 2 * 20 + 10).
    s.move_to(50.0, 250.0);
    assert_eq!(s.take_log(), ["enter:r2"], "control");
    assert_eq!(s.background("r2"), Some(BLUE));

    s.app.handle_event(
        PlatformEvent::MouseWheel {
            x: 50.0,
            y: 250.0,
            delta_x: 0.0,
            delta_y: -60.0,
        },
        SIZE,
        1.0,
    );
    s.turn();
    let top = s.scroller.scroll_top();
    assert!(top > 0.0, "control: it scrolled");
    let row = 2 + (top / 20.0).floor() as usize;
    assert_ne!(row, 2, "control: another row is under the pointer");
    let r = format!("r{row}");
    assert_eq!(s.take_log(), ["leave:r2".to_string(), format!("enter:{r}")]);
    assert_eq!(s.hovered(), std::slice::from_ref(&r));
    assert_eq!(s.background(&r), Some(BLUE));
    assert_ne!(s.background("r2"), Some(BLUE));

    // A programmatic scroll does the same.
    s.scroller.set_scroll_top(0.0);
    s.turn();
    assert_eq!(s.take_log(), [format!("leave:{r}"), "enter:r2".to_string()]);
}

/// A layout that moves nothing under the pointer looks again and finds the
/// same element: no event, no cursor, no restyle and no frame beyond the
/// layout's own. (The cost is pinned in `perf_regression_tests`.)
#[test]
fn a_layout_that_moves_nothing_under_the_pointer_fires_nothing() {
    let mut s = mount();
    s.move_to(50.0, 30.0);
    s.take_log();
    s.far.set_style("width", "120px");
    let actions = s.turn();
    assert!(s.take_log().is_empty());
    assert_eq!(cursor_of(&actions), None, "{actions:?}");
    assert_eq!(s.hovered(), ["col", "up"]);
}

/// The mouse out of the window: its leave cleared hover (#2672), and a
/// removal that slides the lower popup under its last position must not
/// bring it back.
#[test]
fn nothing_is_hovered_again_after_the_mouse_left_the_window() {
    let mut s = mount();
    s.move_to(50.0, 30.0);
    s.take_log();
    s.app.handle_event(PlatformEvent::MouseLeft, SIZE, 1.0);
    s.turn();
    assert_eq!(s.take_log(), ["leave:up", "leave:col"], "control");
    s.up.discard();
    let actions = s.turn();
    assert!(s.take_log().is_empty());
    assert!(s.hovered().is_empty());
    assert_eq!(cursor_of(&actions), None);

    // Nor after a move outside the viewport (a captured drag's, say, that
    // has since ended).
    let mut s = mount();
    s.move_to(50.0, 30.0);
    s.app
        .handle_event(PlatformEvent::MouseMove { x: 50.0, y: -5.0 }, SIZE, 1.0);
    s.turn();
    s.take_log();
    s.up.discard();
    s.turn();
    assert!(s.take_log().is_empty(), "outside: {:?}", s.log.borrow());
}

/// No hover from touch: a finger's last position is not hit-tested again.
#[test]
fn a_fingers_last_position_is_not_hovered_again() {
    let mut s = mount();
    crate::render_surface::set_current_pointer(Some(crate::render_surface::SurfacePointer {
        id: 7,
        kind: crate::render_surface::SurfacePointerKind::Touch,
        pressure: 0.0,
        primary: true,
    }));
    s.app
        .handle_event(PlatformEvent::MouseMove { x: 50.0, y: 30.0 }, SIZE, 1.0);
    crate::render_surface::set_current_pointer(None);
    s.turn();
    s.take_log();
    s.up.discard();
    s.turn();
    assert!(s.take_log().is_empty(), "{:?}", s.log.borrow());
}

/// While a press holds the pointer (a text selection drag here), others hear
/// no boundary event, as Chrome sends none while the pointer is captured.
/// The release ends the capture, and the next turn hovers what is there.
#[test]
fn a_captured_pointer_hovers_nothing_until_the_release() {
    let mut s = mount();
    s.move_to(50.0, 30.0);
    s.take_log();
    let at = |button_down: bool| {
        if button_down {
            PlatformEvent::MouseDown {
                x: 50.0,
                y: 30.0,
                button: MouseButton::Left,
            }
        } else {
            PlatformEvent::MouseUp {
                x: 50.0,
                y: 30.0,
                button: MouseButton::Left,
            }
        }
    };
    s.app.handle_event(at(true), SIZE, 1.0);
    assert!(
        s.app.pointer_captured(),
        "control: the press holds the pointer"
    );
    s.up.discard();
    s.turn();
    assert!(s.take_log().is_empty(), "captured: {:?}", s.log.borrow());

    s.app.handle_event(at(false), SIZE, 1.0);
    assert!(!s.app.pointer_captured());
    s.turn();
    assert_eq!(s.take_log(), ["enter:low"]);
    assert_eq!(s.hovered(), ["col", "low"]);
}

/// Each window is its own `RinchApp` with its own last pointer position:
/// the mouse goes into window `b`, leaves it, and moves in window `a`. A
/// layout in each then hit-tests again in `a` only; `b`'s last position is
/// not hovered again.
#[test]
fn a_window_the_mouse_left_is_not_hovered_again() {
    let mut a = mount();
    let mut b = mount();
    b.move_to(50.0, 30.0);
    assert_eq!(
        b.take_log(),
        ["enter:col", "enter:up"],
        "control: b hovered"
    );
    b.app.handle_event(PlatformEvent::MouseLeft, SIZE, 1.0);
    b.turn();
    assert_eq!(b.take_log(), ["leave:up", "leave:col"], "control: b left");
    a.move_to(50.0, 30.0);
    a.take_log();
    a.up.discard();
    b.up.discard();
    a.turn();
    b.turn();
    assert_eq!(a.take_log(), ["enter:low"], "control: the mouse's window");
    assert!(b.take_log().is_empty(), "{:?}", b.log.borrow());
    assert!(b.hovered().is_empty());
}

/// #2766 review: the hovered node removed in the same turn the mouse
/// leaves the window. The leave re-anchors hover first, so the column's
/// `:hover` goes too, instead of staying set for good.
#[test]
fn a_removal_in_the_turn_the_mouse_leaves_clears_the_ancestors_hover() {
    let mut s = mount();
    s.move_to(50.0, 30.0);
    s.take_log();
    s.up.discard();
    s.app.handle_event(PlatformEvent::MouseLeft, SIZE, 1.0);
    s.turn();
    assert_eq!(s.take_log(), ["leave:col"]);
    assert!(s.hovered().is_empty(), "{:?}", s.hovered());
}

/// #2766 review: the hovered child discarded with the pointer still over
/// its parent. Hover moves to the parent once, and the turns after it
/// hit-test nothing: the chain no longer names the removed child.
#[test]
fn idle_turns_after_the_hovered_child_goes_hit_test_nothing() {
    let mut s = mount();
    s.move_to(450.0, 410.0);
    assert_eq!(s.take_log(), ["enter:box", "enter:inner"], "control");
    s.inner.discard();
    s.turn();
    assert!(s.take_log().is_empty(), "{:?}", s.log.borrow());
    assert_eq!(s.hovered(), ["box"]);
    // Paint what the removal left, then count the idle turns.
    super::perf_expect::settle(&mut s.app);
    let (redraws, total) = super::perf_expect::idle_turns(&mut s.app, 10);
    assert_eq!(redraws, 0);
    assert_eq!(
        total.get(rinch_dom::perf::Counter::HitTests),
        0,
        "{total:?}"
    );
}

/// #2766 review: an element drag shows the grabbing hand, which is not
/// hover's cursor. After the drop, with no further move and nothing moved,
/// the next turn sends hover's cursor again.
#[test]
fn the_hover_cursor_comes_back_after_a_drop_without_a_move() {
    let mut s = mount();
    let low = s.node("low");
    {
        let doc = s.app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.set_attribute(rinch_core::dom::NodeId(low), "draggable", "true");
    }
    s.app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    s.move_to(50.0, 80.0);
    let button = MouseButton::Left;
    s.app.handle_event(
        PlatformEvent::MouseDown {
            x: 50.0,
            y: 80.0,
            button,
        },
        SIZE,
        1.0,
    );
    let actions = s
        .app
        .handle_event(PlatformEvent::MouseMove { x: 50.0, y: 95.0 }, SIZE, 1.0);
    assert_eq!(
        cursor_of(&actions),
        Some(CursorStyle::Grabbing),
        "control: the drag shows its own cursor: {actions:?}"
    );
    s.turn();
    s.app.handle_event(
        PlatformEvent::MouseUp {
            x: 50.0,
            y: 95.0,
            button,
        },
        SIZE,
        1.0,
    );
    let actions = s.turn();
    assert_eq!(
        cursor_of(&actions),
        Some(CursorStyle::Pointer),
        "{actions:?}"
    );
    // Once: the turn after sends none.
    let actions = s.turn();
    assert_eq!(cursor_of(&actions), None, "{actions:?}");
}
