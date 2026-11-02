//! An element's own `onwheel`: offered to the element under the pointer and
//! then its ancestors, nearest first; a handler that consumes the wheel stops
//! the walk and scrolls no container; the payload has the DOM's sign, logical
//! pixels and the modifiers held, and a trackpad pinch arrives as a Ctrl wheel.
//!
//! Driven through the real `rsx!` output and the real wheel path
//! (`handle_event`), the way the desktop loop drives them.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::events::WheelEvent;
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    app: RinchApp,
    scroller: NodeHandle,
    leaf: NodeHandle,
    log: Log,
    events: Rc<RefCell<Vec<WheelEvent>>>,
    consume: Rc<Cell<bool>>,
}

/// A vertical scroll container with an `onwheel` that never consumes, holding
/// a tall `inner` whose `onwheel` consumes when `consume` is set or Ctrl is
/// held, holding a `leaf` with no handler of its own.
fn mount() -> Fixture {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let events: Rc<RefCell<Vec<WheelEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let consume = Rc::new(Cell::new(false));
    let handles: Rc<RefCell<Option<[NodeHandle; 2]>>> = Rc::new(RefCell::new(None));
    let (log_in, events_in, consume_in, handles_in) = (
        log.clone(),
        events.clone(),
        consume.clone(),
        handles.clone(),
    );
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let (l1, l2) = (log_in.clone(), log_in.clone());
        let ev = events_in.clone();
        let consume = consume_in.clone();
        let root = rsx! {
            div {
                style: "width: 300px; height: 100px; overflow-y: auto",
                onwheel: move |_w| {
                    l1.borrow_mut().push("scroller".into());
                    false
                },
                div {
                    style: "height: 400px",
                    onwheel: move |w: &WheelEvent| {
                        l2.borrow_mut().push("inner".into());
                        ev.borrow_mut().push(*w);
                        consume.get() || w.modifiers.ctrl
                    },
                    div { style: "height: 50px" }
                }
            }
        };
        let leaf = root.children()[0].children()[0].clone();
        *handles_in.borrow_mut() = Some([root.clone(), leaf]);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let [scroller, leaf] = handles.borrow_mut().take().expect("handles");
    Fixture {
        app,
        scroller,
        leaf,
        log,
        events,
        consume,
    }
}

impl Fixture {
    fn leaf_center(&self) -> (f32, f32) {
        let d = self.app.doc.as_ref().unwrap().borrow();
        let (x, y, w, h) = painted_element_box(&d.tree, self.leaf.node_id().0);
        (x + w / 2.0, y + h / 2.0)
    }

    /// A wheel over the leaf, with winit's sign (positive `delta_y` turns the
    /// wheel away from the user, scrolling the content down into view above).
    fn wheel(&mut self, delta_x: f64, delta_y: f64) {
        let (x, y) = self.leaf_center();
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
        self.app.handle_event(
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            },
            VP,
            1.0,
        );
    }

    fn modifiers(&mut self, m: Modifiers) {
        self.app
            .handle_event(PlatformEvent::ModifiersChanged(m), VP, 1.0);
    }

    fn scroll_top(&self) -> f64 {
        let d = self.app.doc.as_ref().unwrap().borrow();
        d.tree
            .get(self.scroller.node_id().0)
            .unwrap()
            .scroll_offset
            .1
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn take_events(&self) -> Vec<WheelEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// Nearest first: the leaf has no handler, so `inner` is offered the wheel,
/// then the scroller; neither consumes, so the scroller scrolls as before.
#[test]
fn onwheel_runs_nearest_first_and_an_unconsumed_wheel_still_scrolls() {
    let mut f = mount();
    assert_eq!(f.scroll_top(), 0.0);
    f.wheel(0.0, -40.0);
    assert_eq!(f.take_log(), ["inner", "scroller"]);
    assert!(
        f.scroll_top() > 0.0,
        "an unconsumed wheel scrolls the container: {}",
        f.scroll_top()
    );
}

/// A consumed wheel stops the walk and scrolls no ancestor container.
#[test]
fn a_consumed_wheel_stops_the_walk_and_scrolls_nothing() {
    let mut f = mount();
    f.consume.set(true);
    f.wheel(0.0, -40.0);
    assert_eq!(
        f.take_log(),
        ["inner"],
        "the scroller's onwheel is not asked"
    );
    assert_eq!(f.scroll_top(), 0.0, "a consumed wheel scrolls nothing");

    // Released, the same wheel scrolls again — the control for the above.
    f.consume.set(false);
    f.wheel(0.0, -40.0);
    assert_eq!(f.take_log(), ["inner", "scroller"]);
    assert!(f.scroll_top() > 0.0);
}

/// The payload has the DOM's sign (positive `delta_y` scrolls down, the
/// opposite of winit's), logical pixels, the pointer and the modifiers.
#[test]
fn the_wheel_carries_dom_signed_deltas_the_pointer_and_modifiers() {
    let mut f = mount();
    let (x, y) = f.leaf_center();
    f.modifiers(Modifiers {
        shift: true,
        alt: true,
        ..Modifiers::default()
    });
    f.wheel(12.0, -40.0);
    let events = f.take_events();
    assert_eq!(events.len(), 1);
    let w = events[0];
    assert_eq!((w.delta_x, w.delta_y), (-12.0, 40.0));
    assert_eq!((w.x, w.y), (x, y));
    assert!(w.modifiers.shift && w.modifiers.alt);
    assert!(!w.modifiers.ctrl && !w.modifiers.meta);

    // Ctrl is reported, and here the handler consumes on it: zoom, not scroll.
    // A fresh mount: the wheel above scrolled the leaf out from under the
    // pointer.
    let mut f = mount();
    f.modifiers(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    f.take_log();
    f.wheel(0.0, -40.0);
    let w = f.take_events()[0];
    assert!(w.modifiers.ctrl && !w.modifiers.shift);
    assert_eq!(f.take_log(), ["inner"]);
    assert_eq!(f.scroll_top(), 0.0);
}

/// A trackpad pinch no render surface takes is a Ctrl wheel whose
/// `zoom_factor` is the pinch's scale, and it scrolls nothing it consumed.
#[test]
fn a_pinch_arrives_as_a_ctrl_wheel_with_its_scale() {
    let mut f = mount();
    let (x, y) = f.leaf_center();
    f.app
        .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
    assert!(f.app.pinch_as_wheel(0.25), "the ctrl wheel was consumed");
    let w = f.take_events()[0];
    assert!(w.modifiers.ctrl);
    assert_eq!(w.delta_x, 0.0);
    assert!(
        w.delta_y < 0.0,
        "spreading the fingers zooms in: {}",
        w.delta_y
    );
    assert!((w.zoom_factor() - 1.25).abs() < 1e-4, "{}", w.zoom_factor());
    assert_eq!(f.take_log(), ["inner"]);

    assert!(f.app.pinch_as_wheel(-0.2));
    let w = f.take_events()[0];
    assert!((w.zoom_factor() - 0.8).abs() < 1e-4, "{}", w.zoom_factor());
}

/// No `onwheel` anywhere: the wheel scrolls exactly as it did before the
/// event existed.
#[test]
fn without_a_handler_the_wheel_scrolls_as_before() {
    let handles: Rc<RefCell<Option<[NodeHandle; 2]>>> = Rc::new(RefCell::new(None));
    let handles_in = handles.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = rsx! {
            div {
                style: "width: 300px; height: 100px; overflow-y: auto",
                div { style: "height: 400px", div { style: "height: 50px" } }
            }
        };
        let leaf = root.children()[0].children()[0].clone();
        *handles_in.borrow_mut() = Some([root.clone(), leaf]);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let [scroller, _leaf] = handles.borrow_mut().take().unwrap();
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 20.0,
            y: 20.0,
            delta_x: 0.0,
            delta_y: -40.0,
        },
        VP,
        1.0,
    );
    let d = app.doc.as_ref().unwrap().borrow();
    assert!(d.tree.get(scroller.node_id().0).unwrap().scroll_offset.1 > 0.0);
}
