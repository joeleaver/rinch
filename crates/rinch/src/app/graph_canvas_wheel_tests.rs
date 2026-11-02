//! The node-graph canvas's wheel over node bodies that hold scroll
//! containers, driven through the real desktop wheel path (`handle_event`)
//! the way the desktop loop drives it.
//!
//! The canvas (`rinch_graph_view::graph_canvas`) offers a plain wheel to a
//! scroller inside a node that can still move that way, and pans only when
//! none can — the browser's scroll chaining; a scroller that reaches its end
//! mid-gesture keeps the rest of that gesture (it moves nothing) and a new
//! wheel pans. Ctrl/Cmd+wheel and a pinch zoom over anything; an element
//! marked `data-graph-nowheel` takes itself out of the canvas's wheel. The
//! page around the canvas never scrolls from a wheel the canvas takes.
//!
//! Before #2064's follow-up the canvas consumed every wheel, so a list in a
//! node could never be wheel-scrolled.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_graph_view::{
    AnyConnection, CanvasOptions, GraphState, GraphView, NO_WHEEL_ATTR, Point, Viewport,
    graph_canvas,
};

const VP: (u32, u32) = (800, 600);

type View = GraphView<&'static str, (), ()>;

struct Fixture {
    app: RinchApp,
    view: View,
    page: NodeHandle,
    canvas: NodeHandle,
    /// A vertical list: 60px tall, 300px of rows.
    list: NodeHandle,
    /// A horizontal strip: 120px wide, 400px of content, no vertical overflow.
    strip: NodeHandle,
    /// The child of a `data-graph-nowheel` element (no scroller of its own).
    opted_child: NodeHandle,
    /// A plain body, marked `data-graph-nowheel="false"` (the escape: not
    /// opted out).
    plain: NodeHandle,
    /// An 80px scroller (160px of content) holding `inner`, a 40px list
    /// (200px of rows).
    outer: NodeHandle,
    inner: NodeHandle,
}

fn el(scope: &mut RenderScope, style: &str) -> NodeHandle {
    let e = scope.create_element("div");
    e.set_attribute("style", style);
    e
}

/// A page scroller (500px tall, its content 1200px) holding a 600×400
/// canvas, whose four nodes are the list, the strip, the opted-out element
/// and the plain body.
fn mount() -> Fixture {
    type Out = (View, [NodeHandle; 8]);
    let out: Rc<RefCell<Option<Out>>> = Rc::new(RefCell::new(None));
    let out_in = out.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let view: View = GraphView::new(GraphState::new(), AnyConnection);
        view.edit(|tx| {
            tx.add_node(Point::new(20.0, 20.0), "list", [])?;
            tx.add_node(Point::new(240.0, 20.0), "strip", [])?;
            tx.add_node(Point::new(20.0, 160.0), "opted", [])?;
            tx.add_node(Point::new(240.0, 160.0), "plain", [])?;
            tx.add_node(Point::new(420.0, 20.0), "nested", [])?;
            Ok(())
        })
        .unwrap();
        let parts: Rc<RefCell<Vec<(&'static str, NodeHandle)>>> = Rc::new(RefCell::new(Vec::new()));
        let parts_in = parts.clone();
        let v = view;
        let render = move |scope: &mut RenderScope, id| {
            let kind = v.with_state(|s| s.graph().node(id).unwrap().data);
            match kind {
                "list" => {
                    let list = el(scope, "width: 120px; height: 60px; overflow-y: auto");
                    list.append_child(&el(scope, "height: 300px"));
                    parts_in.borrow_mut().push(("list", list.clone()));
                    list
                }
                "strip" => {
                    let strip = el(scope, "width: 120px; height: 40px; overflow-x: auto");
                    strip.append_child(&el(scope, "width: 400px; height: 20px"));
                    parts_in.borrow_mut().push(("strip", strip.clone()));
                    strip
                }
                "opted" => {
                    let opted = el(scope, "width: 120px; padding: 4px");
                    opted.set_attribute(NO_WHEEL_ATTR, "");
                    let child = el(scope, "height: 30px");
                    opted.append_child(&child);
                    parts_in.borrow_mut().push(("opted_child", child));
                    opted
                }
                "nested" => {
                    let outer = el(scope, "width: 120px; height: 80px; overflow-y: auto");
                    outer.append_child(&el(scope, "height: 20px"));
                    let inner = el(scope, "width: 100px; height: 40px; overflow-y: auto");
                    inner.append_child(&el(scope, "height: 200px"));
                    outer.append_child(&inner);
                    outer.append_child(&el(scope, "height: 100px"));
                    parts_in.borrow_mut().push(("outer", outer.clone()));
                    parts_in.borrow_mut().push(("inner", inner));
                    outer
                }
                _ => {
                    let plain = el(scope, "width: 120px; height: 40px");
                    plain.set_attribute(NO_WHEEL_ATTR, "false");
                    parts_in.borrow_mut().push(("plain", plain.clone()));
                    plain
                }
            }
        };
        let page = el(scope, "width: 700px; height: 500px; overflow-y: auto");
        let canvas = graph_canvas(
            scope,
            &view,
            render,
            CanvasOptions {
                style: "width: 600px; height: 400px".into(),
                ..CanvasOptions::default()
            },
        );
        page.append_child(&canvas);
        page.append_child(&el(scope, "height: 800px"));
        let find = |name: &str| {
            parts
                .borrow()
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, h)| h.clone())
                .unwrap_or_else(|| panic!("{name} was not rendered"))
        };
        *out_in.borrow_mut() = Some((
            view,
            [
                page.clone(),
                canvas,
                find("list"),
                find("strip"),
                find("opted_child"),
                find("plain"),
                find("outer"),
                find("inner"),
            ],
        ));
        page
    });
    // Every test runs on the test clock, from 0 ms; `Fixture::at` moves it.
    rinch_graph_view::__set_wheel_clock(Some(0));
    // Shift turns a wheel sideways, as on Linux and Windows (set, so a macOS
    // build runs the same tests).
    rinch_core::events::set_shift_wheel_turns_sideways(true);
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (view, [page, canvas, list, strip, opted_child, plain, outer, inner]) =
        out.borrow_mut().take().expect("mounted");
    Fixture {
        app,
        view,
        page,
        canvas,
        list,
        strip,
        opted_child,
        plain,
        outer,
        inner,
    }
}

impl Fixture {
    fn center(&self, h: &NodeHandle) -> (f32, f32) {
        let d = self.app.doc.as_ref().unwrap().borrow();
        let (x, y, w, h) = painted_element_box(&d.tree, h.node_id().0);
        (x + w / 2.0, y + h / 2.0)
    }

    /// A point on the canvas's background, clear of every node.
    fn background(&self) -> (f32, f32) {
        let d = self.app.doc.as_ref().unwrap().borrow();
        let (x, y, _, _) = painted_element_box(&d.tree, self.canvas.node_id().0);
        (x + 520.0, y + 360.0)
    }

    /// Put the canvas's gesture clock at `ms` (from the mount's 0).
    fn at(&self, ms: u64) {
        rinch_graph_view::__set_wheel_clock(Some(ms));
    }

    fn modifiers(&mut self, m: Modifiers) {
        self.app
            .handle_event(PlatformEvent::ModifiersChanged(m), VP, 1.0);
    }

    /// A wheel at `at` in the DOM's sign (positive `dy` scrolls down), sent
    /// with winit's, then a frame.
    fn wheel(&mut self, at: (f32, f32), dx: f64, dy: f64) {
        let (x, y) = at;
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
        self.app.handle_event(
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x: -dx,
                delta_y: -dy,
            },
            VP,
            1.0,
        );
        self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }

    fn offset(&self, h: &NodeHandle) -> (f64, f64) {
        let d = self.app.doc.as_ref().unwrap().borrow();
        d.tree.get(h.node_id().0).unwrap().scroll_offset
    }

    fn viewport(&self) -> Viewport {
        rinch_core::untracked(|| self.view.viewport())
    }

    fn pan(&self) -> (f32, f32) {
        let vp = self.viewport();
        (vp.x, vp.y)
    }
}

/// The fixture's premise: the list, the strip and the page overflow, and the
/// canvas starts unpanned.
#[test]
fn the_fixture_overflows_where_it_says() {
    let f = mount();
    let d = f.app.doc.as_ref().unwrap().borrow();
    use rinch_dom::paint::scrollbar::{ScrollbarAxis, overflows};
    assert!(overflows(
        &d.tree,
        f.list.node_id().0,
        ScrollbarAxis::Vertical
    ));
    assert!(overflows(
        &d.tree,
        f.strip.node_id().0,
        ScrollbarAxis::Horizontal
    ));
    assert!(!overflows(
        &d.tree,
        f.strip.node_id().0,
        ScrollbarAxis::Vertical
    ));
    assert!(overflows(
        &d.tree,
        f.page.node_id().0,
        ScrollbarAxis::Vertical
    ));
    drop(d);
    assert_eq!(f.pan(), (0.0, 0.0));
}

/// (1) A plain wheel over a node's list that can still move scrolls the list;
/// the canvas does not pan and the page does not scroll.
#[test]
fn a_wheel_over_a_scrollable_node_body_scrolls_it_not_the_canvas() {
    let mut f = mount();
    let at = f.center(&f.list);
    f.wheel(at, 0.0, 40.0);
    assert_eq!(f.offset(&f.list).1, 40.0, "the list scrolled");
    assert_eq!(f.pan(), (0.0, 0.0), "the canvas did not pan");
    assert_eq!(f.offset(&f.page), (0.0, 0.0), "the page did not scroll");
}

/// (1) A scroller that cannot move the wheel's way is no reason to decline:
/// at its top, a wheel up over the list pans the canvas at once.
#[test]
fn a_wheel_a_scroller_cannot_follow_pans_the_canvas() {
    let mut f = mount();
    let at = f.center(&f.list);
    f.wheel(at, 0.0, -40.0);
    assert_eq!(f.offset(&f.list), (0.0, 0.0));
    assert_eq!(f.pan(), (0.0, 40.0), "the canvas panned (content down)");
    assert_eq!(f.offset(&f.page), (0.0, 0.0));
}

/// (1) Scrolled to its end mid-gesture, the list keeps the rest of the
/// gesture: those wheels move nothing (a browser latches a gesture to its
/// scroller). A new wheel, after the gesture's gap, pans the canvas.
#[test]
fn at_the_end_the_gesture_stops_and_a_new_wheel_pans() {
    let mut f = mount();
    let at = f.center(&f.list);
    // 300 - 60 = 240px of range: six wheels of 40.
    for _ in 0..6 {
        f.wheel(at, 0.0, 40.0);
    }
    assert_eq!(f.offset(&f.list).1, 240.0, "the list is at its end");
    assert_eq!(f.pan(), (0.0, 0.0));

    // The same gesture goes on: nothing moves, the page included.
    f.wheel(at, 0.0, 40.0);
    f.wheel(at, 0.0, 40.0);
    assert_eq!(f.offset(&f.list).1, 240.0);
    assert_eq!(f.pan(), (0.0, 0.0), "the rest of the gesture does not pan");
    assert_eq!(f.offset(&f.page), (0.0, 0.0), "nor scroll the page");

    // A new gesture.
    f.at(600);
    f.wheel(at, 0.0, 40.0);
    assert_eq!(f.pan(), (0.0, -40.0), "a new wheel pans the canvas");
    assert_eq!(f.offset(&f.page), (0.0, 0.0));
    // That pan is a gesture of its own; after it, a new wheel scrolls the
    // list back up.
    f.at(1200);
    f.wheel(f.center(&f.list), 0.0, -40.0);
    assert_eq!(f.offset(&f.list).1, 200.0);
    assert_eq!(f.pan(), (0.0, -40.0));
}

/// (1) Per axis, after Shift's sideways mapping: Shift+wheel over the
/// horizontal strip scrolls it sideways; over the vertical-only list it pans
/// the canvas sideways.
#[test]
fn shift_wheel_follows_the_axis_it_is_turned_to() {
    let mut f = mount();
    f.modifiers(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    let strip = f.center(&f.strip);
    f.wheel(strip, 0.0, 40.0);
    assert_eq!(
        f.offset(&f.strip),
        (40.0, 0.0),
        "the strip scrolled sideways"
    );
    assert_eq!(f.pan(), (0.0, 0.0));

    // A fresh canvas: on this one the wheel above began a gesture the strip
    // holds for the next 500 ms.
    let mut f = mount();
    f.modifiers(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    let list = f.center(&f.list);
    f.wheel(list, 0.0, 40.0);
    assert_eq!(
        f.offset(&f.list),
        (0.0, 0.0),
        "the list has no sideways room"
    );
    assert_eq!(f.pan(), (-40.0, 0.0), "the canvas panned sideways");
}

/// (3) Ctrl/Cmd+wheel and a pinch zoom the canvas even over a list that could
/// scroll, and the list does not move.
#[test]
fn ctrl_wheel_and_pinch_zoom_over_a_scrollable_node() {
    for (ctrl, meta) in [(true, false), (false, true)] {
        let mut f = mount();
        f.modifiers(Modifiers {
            ctrl,
            meta,
            ..Modifiers::default()
        });
        let at = f.center(&f.list);
        // Down, the way the list could scroll (not up, where it is stuck at
        // its top and a zoom would pass for the wrong reason): zooms out.
        f.wheel(at, 0.0, 50.0);
        assert!(f.viewport().zoom < 1.0, "zoomed out: {:?}", f.viewport());
        assert_eq!(f.offset(&f.list), (0.0, 0.0), "the list did not scroll");
    }

    let mut f = mount();
    let at = f.center(&f.list);
    f.app
        .handle_event(PlatformEvent::MouseMove { x: at.0, y: at.1 }, VP, 1.0);
    // Pinching in is a wheel down, which the list could follow.
    assert!(f.app.pinch_as_wheel(-0.2), "the canvas took the pinch");
    assert!((f.viewport().zoom - 0.8).abs() < 1e-4, "{:?}", f.viewport());
    assert_eq!(f.offset(&f.list), (0.0, 0.0));
}

/// (2) Inside a `data-graph-nowheel` element the canvas neither pans nor
/// zooms: the wheel goes on as if the canvas were not there — here, with
/// nothing inside to scroll, to the page.
#[test]
fn an_opted_out_element_neither_pans_nor_zooms_the_canvas() {
    let mut f = mount();
    let at = f.center(&f.opted_child);
    f.wheel(at, 0.0, 40.0);
    assert_eq!(f.pan(), (0.0, 0.0), "the canvas did not pan");
    assert_eq!(
        f.offset(&f.page).1,
        40.0,
        "normal handling: the page scrolled"
    );

    let mut f = mount();
    f.modifiers(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    let at = f.center(&f.opted_child);
    f.wheel(at, 0.0, -50.0);
    assert_eq!(f.viewport().zoom, 1.0, "the canvas did not zoom");
    f.app
        .handle_event(PlatformEvent::MouseMove { x: at.0, y: at.1 }, VP, 1.0);
    assert!(!f.app.pinch_as_wheel(0.25), "nor take a pinch");
    assert_eq!(f.viewport().zoom, 1.0);
}

/// (4) A wheel the canvas takes never scrolls the page around it: over a
/// plain body (whose `data-graph-nowheel="false"` is the escape, not an opt
/// out) and over the background, both directions.
#[test]
fn the_page_around_the_canvas_never_scrolls_from_a_wheel_it_takes() {
    let mut f = mount();
    let plain = f.center(&f.plain);
    f.wheel(plain, 0.0, 40.0);
    assert_eq!(f.pan(), (0.0, -40.0));
    let bg = f.background();
    f.wheel(bg, 0.0, 40.0);
    f.wheel(bg, 25.0, 0.0);
    assert_eq!(f.pan(), (-25.0, -80.0));
    assert_eq!(f.offset(&f.page), (0.0, 0.0), "the page never scrolled");
}

/// Review of #2084, finding 1: a diagonal wheel the canvas declines goes to
/// one scroller, whole, as Chrome and Firefox send it (measured: a strip that
/// scrolls only sideways takes a `(40, 6)` wheel as `(40, 0)` and the page
/// does not move). It used to split by axis on desktop, and the vertical part
/// walked past the canvas to the page.
#[test]
fn a_diagonal_wheel_over_a_strip_does_not_leak_to_the_page() {
    let mut f = mount();
    let at = f.center(&f.strip);
    f.wheel(at, 40.0, 6.0);
    assert_eq!(f.offset(&f.strip), (40.0, 0.0), "the strip took the wheel");
    assert_eq!(f.offset(&f.page), (0.0, 0.0), "the page did not move");
    assert_eq!(f.pan(), (0.0, 0.0));
}

/// Review of #2084, finding 2: a gesture the canvas pans with stays the
/// canvas's. Panning slides nodes under a still pointer, so a trackpad pan
/// would otherwise be taken over by the first list that arrives under it.
#[test]
fn a_panning_gesture_is_not_taken_over_by_a_list_it_slides_under_the_pointer() {
    let mut f = mount();
    let bg = f.background();
    f.wheel(bg, 40.0, 0.0);
    assert_eq!(f.pan(), (-40.0, 0.0));
    // 40 ms later, the pointer is over the list.
    f.at(40);
    let list = f.center(&f.list);
    f.wheel(list, 0.0, 40.0);
    assert_eq!(f.offset(&f.list), (0.0, 0.0), "the list was not taken");
    assert_eq!(f.pan(), (-40.0, -40.0), "the canvas kept panning");

    // A new gesture over the list scrolls it.
    f.at(600);
    let list = f.center(&f.list);
    f.wheel(list, 0.0, 40.0);
    assert_eq!(f.offset(&f.list), (0.0, 40.0));
    assert_eq!(f.pan(), (-40.0, -40.0));
}

/// Review of #2084, finding 3: a list at its end inside a scroller that can
/// still move: the wheel scrolls the outer one (a browser chains a new
/// gesture to the nearest ancestor that can move), not nothing.
#[test]
fn a_list_at_its_end_chains_to_the_scroller_around_it() {
    let mut f = mount();
    f.inner.set_scroll_top(1000.0);
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(
        f.offset(&f.inner).1,
        160.0,
        "precondition: inner at its end"
    );
    let at = f.center(&f.inner);
    f.wheel(at, 0.0, 30.0);
    assert_eq!(f.offset(&f.inner).1, 160.0);
    assert_eq!(f.offset(&f.outer).1, 30.0, "the outer scroller took it");
    assert_eq!(f.pan(), (0.0, 0.0));
    assert_eq!(f.offset(&f.page), (0.0, 0.0));
}
