//! `Modal`/`Drawer`'s `lock_scroll`, driven through the real wheel path
//! (issue #474).
//!
//! The prop was declared, documented and read by nothing: an open dialog and
//! the page behind it both scrolled, and the page was usually the one under the
//! pointer because the dialog's backdrop covers it.
//!
//! **Why here and not in `rinch-components`.** That crate's harness mounts into
//! `MockDomDocument`, which has no CSS engine and no event dispatch, so it can
//! see that a prop is read and nothing about what a wheel does afterwards.
//! `RinchApp::doc` is `pub(crate)`.
//!
//! **What "locked" means on desktop.** Not `overflow: hidden` on the body — the
//! wheel *arm* is refused for a container outside every locking overlay
//! (`NodeTree::scroll_locked_out`). So two things have to be asserted every
//! time, and the second is the one a fixture parked on a fixed point would miss:
//! the page must not move, **and the dialog's own scroller must**. A gate that
//! rejects everything passes the first assertion perfectly.
//!
//! **The page here is an explicit scroller, not `<body>`.** With the modal open
//! its root is `position: fixed; inset: 0`, so the hit node is the backdrop and
//! the ancestor walk from it never reaches the page — the geometric fallback
//! (`find_scroll_container_at_point`) is what resolves it. That is the real
//! shape of the bug, and the fixture would not reproduce it through the
//! ancestor chain alone.

use super::*;
use std::cell::Cell;

use super::hit_testing::find_scrollbar_hit;
use super::wheel_scroll_dispatch_tests::{offsets, wheel};
use rinch_components::{Drawer, Modal};
use rinch_core::{Component, Signal};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// A wheel gesture big enough to land **off** both ends of the page's scroll
/// range: 700px of a 1400px range. At 0 a gate that rejects everything and one
/// that rejects nothing agree, and at the clamp so do a moved and an unmoved
/// container.
const WHEEL_DY: f64 = -700.0;
const EXPECTED_PAGE_SCROLL: f64 = 700.0;

/// Mount `build`'s tree under the real theme **and** component stylesheets, so
/// the modal root's `position: fixed` and the panel's box are the ones the
/// shipped sheet gives them.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn reactive(sig: Signal<bool>) -> Rc<dyn Fn() -> bool> {
    Rc::new(move || sig.get())
}

/// The point the wheel goes in at: over the page **and** over an open modal's
/// backdrop, below the panel, which is where a user scrolls by mistake.
const AIM: (f32, f32) = (400.0, 450.0);

/// The page scroller behind the overlay, and the overlay's own scroller.
struct Fixture {
    app: RinchApp,
    page: usize,
    inner: usize,
}

/// A page that scrolls, with `build`'s overlay on top of it.
///
/// The page is `position: absolute` so it is out of the overlay's way in flow
/// terms while still filling the viewport geometrically — the arrangement a
/// full-window app has.
fn mount_with_overlay(
    build: impl FnOnce(&mut RenderScope, &[NodeHandle]) -> NodeHandle + 'static,
) -> Fixture {
    let page: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let inner: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let page_in = page.clone();
    let inner_in = inner.clone();

    let app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");

        let page_el = scope.create_element("div");
        page_el.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 800px; height: 600px; overflow: auto",
        );
        // Overflowing on **both** axes: the vertical range is 1400 and the
        // horizontal one 1200, so a 700px gesture on either lands off both ends
        // of its own range.
        let tall = scope.create_element("div");
        tall.set_attribute("style", "width: 2000px; height: 2000px");
        page_el.append_child(&tall);
        page_in.set(Some(page_el.node_id().0));
        root.append_child(&page_el);

        // The dialog's own scrollable body — the thing that must keep working.
        let inner_el = scope.create_element("div");
        inner_el.set_attribute("style", "width: 300px; height: 100px; overflow: auto");
        let inner_tall = scope.create_element("div");
        inner_tall.set_attribute("style", "width: 100%; height: 1000px");
        inner_el.append_child(&inner_tall);
        inner_in.set(Some(inner_el.node_id().0));

        let overlay = build(scope, &[inner_el]);
        root.append_child(&overlay);
        root
    });

    Fixture {
        app,
        page: page.get().expect("the page's node id"),
        inner: inner.get().expect("the inner scroller's node id"),
    }
}

/// A `Modal` with `lock_scroll` as given, over a scrollable page.
fn modal_over_page(open: Signal<bool>, lock_scroll: bool) -> Fixture {
    mount_with_overlay(move |scope, children| {
        Modal {
            opened_fn: Some(reactive(open)),
            lock_scroll,
            ..Default::default()
        }
        .render(scope, children)
    })
}

/// The centre of a node's painted box, which is where a pointer event aimed at
/// it has to go in.
fn centre(app: &RinchApp, node_id: usize) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let n = d.tree.get(node_id).expect("node still in the tree");
    assert!(
        n.layout.width > 0.0 && n.layout.height > 0.0,
        "node {node_id} has no box to aim at: {:?}",
        n.layout
    );
    let (x, y, w, h) = painted_element_box(&d.tree, node_id);
    (x + w / 2.0, y + h / 2.0)
}

fn scroll_top(app: &RinchApp, id: usize) -> f64 {
    offsets(app, id).1
}

fn scroll_left(app: &RinchApp, id: usize) -> f64 {
    offsets(app, id).0
}

// ── 1. The precondition, and the prop's off switch ───────────────────────────

/// **The positive control.** Everything below asserts that something did *not*
/// move, which is exactly what a fixture whose wheel never reached a scroller
/// would also assert. This says the instrument fires.
///
/// It doubles as `lock_scroll: false`: the page behind an open modal goes on
/// scrolling, which is the behaviour every `Modal` had before #474 and which the
/// prop's `false` value must preserve. Without this half, a fix that locked
/// unconditionally and ignored the prop passes the whole file.
#[test]
fn the_page_scrolls_behind_an_open_modal_when_lock_scroll_is_off() {
    let open = Signal::new(true);
    let mut f = modal_over_page(open, false);

    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);

    assert_eq!(
        scroll_top(&f.app, f.page),
        EXPECTED_PAGE_SCROLL,
        "with lock_scroll off the page behind must still scroll — if this is 0 \
         the wheel never reached a scroller and nothing below proves anything"
    );
}

/// A modal that is mounted but **closed** locks nothing.
///
/// `Modal::render` runs once and `opened_fn` only rewrites a class, so a closed
/// modal stays in the tree. A lock taken at render rather than on the open edge
/// would make the page unscrollable from first paint, for every app that mounts
/// a dialog it has not shown yet — which is every app.
#[test]
fn a_closed_but_mounted_modal_locks_nothing() {
    let open = Signal::new(false);
    let mut f = modal_over_page(open, true);

    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);

    assert_eq!(
        scroll_top(&f.app, f.page),
        EXPECTED_PAGE_SCROLL,
        "a closed modal must leave the page alone"
    );
}

// ── 2. The lock itself ───────────────────────────────────────────────────────

/// The page does not move while a `lock_scroll` modal is open — **and** the
/// modal's own scroller still does.
///
/// The second half is not decoration. The gate is "reject a container outside
/// every locking root", and the cheapest wrong version of it is "reject every
/// container while anything is locked", which passes the first assertion
/// exactly. A dialog with a scrollable body is the common case and it would be
/// dead.
#[test]
fn an_open_lock_scroll_modal_stops_the_page_but_not_its_own_scroller() {
    let open = Signal::new(true);
    let mut f = modal_over_page(open, true);

    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&f.app, f.page),
        0.0,
        "the page behind a locking modal must not move"
    );

    let inside = centre(&f.app, f.inner);
    wheel(&mut f.app, inside, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&f.app, f.inner),
        -WHEEL_DY,
        "the modal's own scroller is inside the locking root and must still \
         scroll — and by the whole gesture, which its 900px range (1000 - 100) \
         leaves room for, so this is not the clamp agreeing with itself"
    );
    assert_eq!(
        scroll_top(&f.app, f.page),
        0.0,
        "and scrolling inside the modal must not leak to the page"
    );
}

/// Closing the modal releases the lock: the page scrolls again.
///
/// The effect's edge guard has to run in both directions. One that only ever
/// locks passes every other test in this file.
#[test]
fn closing_the_modal_releases_the_lock() {
    let open = Signal::new(true);
    let mut f = modal_over_page(open, true);

    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(scroll_top(&f.app, f.page), 0.0, "precondition: locked");

    open.set(false);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&f.app, f.page),
        EXPECTED_PAGE_SCROLL,
        "a closed modal holds no lock"
    );
}

/// **The effect's edge guard.** An `opened_fn` is an arbitrary getter, so its
/// effect re-runs whenever *anything* it read changes, not only when `opened`
/// flips — a `Memo` recomputing, a sibling signal in the same closure. Each
/// re-run while open would take another lock, and since a close only ever
/// releases one, the page would be left locked for ever by a dialog that is
/// visibly shut.
///
/// The nudge signal is read *inside* the getter, which is what makes the re-run
/// happen without changing the answer — the shape a real `opened_fn` reading a
/// store field has.
#[test]
fn reopening_the_effect_without_changing_opened_does_not_ratchet_the_lock() {
    let open = Signal::new(true);
    let nudge = Signal::new(0u32);
    let mut f = mount_with_overlay(move |scope, children| {
        Modal {
            opened_fn: Some(Rc::new(move || {
                nudge.get();
                open.get()
            })),
            lock_scroll: true,
            ..Default::default()
        }
        .render(scope, children)
    });

    // Re-run the effect three times over, `opened` unchanged at `true`.
    for i in 1..=3 {
        nudge.set(i);
    }
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&f.app, f.page),
        0.0,
        "precondition: still open, still locked"
    );

    open.set(false);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&f.app, f.page),
        EXPECTED_PAGE_SCROLL,
        "one close releases the one lock — four opens did not take four"
    );
}

/// `Drawer` carries the same prop and must do the same thing.
///
/// It shares `arm_lock_scroll` with `Modal`, and a test that only covered
/// `Modal` is exactly how a shared helper comes to be called from one of two
/// call sites.
#[test]
fn a_drawer_locks_the_page_too_and_honours_its_own_off_switch() {
    for lock_scroll in [true, false] {
        let expected = if lock_scroll {
            0.0
        } else {
            EXPECTED_PAGE_SCROLL
        };
        let open = Signal::new(true);
        let mut f = mount_with_overlay(move |scope, children| {
            Drawer {
                opened_fn: Some(reactive(open)),
                lock_scroll,
                ..Default::default()
            }
            .render(scope, children)
        });

        wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
        assert_eq!(
            scroll_top(&f.app, f.page),
            expected,
            "Drawer with lock_scroll: {lock_scroll}"
        );
    }
}

// ── 3. Nesting ───────────────────────────────────────────────────────────────

/// Two locking modals, the inner one in a child scope the test can dispose —
/// what `show_dom` gives an `if` branch, and the only way to unmount one overlay
/// and not the other.
struct Nested {
    f: Fixture,
    inner_scope: Rc<RefCell<Option<RenderScope>>>,
    inner_modal_root: Rc<Cell<Option<usize>>>,
}

impl Nested {
    fn mount(outer_open: Signal<bool>, inner_open: Signal<bool>) -> Self {
        let inner_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));
        let inner_modal_root: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let scope_in = inner_scope.clone();
        let root_in = inner_modal_root.clone();

        let f = mount_with_overlay(move |scope, children| {
            let holder = scope.create_element("div");

            let outer = Modal {
                opened_fn: Some(reactive(outer_open)),
                lock_scroll: true,
                ..Default::default()
            }
            .render(scope, children);
            holder.append_child(&outer);

            let doc = scope.doc_weak().upgrade().expect("document alive");
            let mut child = RenderScope::new(doc, holder.node_id());
            let inner = {
                let _owner = child.push_owner();
                Modal {
                    opened_fn: Some(reactive(inner_open)),
                    lock_scroll: true,
                    ..Default::default()
                }
                .render(&mut child, &[])
            };
            root_in.set(Some(inner.node_id().0));
            holder.append_child(&inner);
            *scope_in.borrow_mut() = Some(child);

            holder
        });

        Self {
            f,
            inner_scope,
            inner_modal_root,
        }
    }

    /// Unmount the inner modal: take its subtree out of the document and
    /// dispose its scope, which is what happens to a `show_dom` branch that
    /// stops matching.
    ///
    /// In the harsher order, deliberately: `show_dom` disposes *before* it
    /// removes, so the release runs while the node is still in the tree. Here
    /// it runs after, which is what a release that tried to look its own node
    /// up would fail on.
    fn unmount_inner(&mut self) {
        let id = self.inner_modal_root.get().expect("the inner modal's root");
        {
            let doc = self.f.app.doc.as_ref().unwrap();
            doc.borrow_mut().remove_node(rinch_core::dom::NodeId(id));
        }
        let scope = self.inner_scope.borrow_mut().take().expect("mounted once");
        scope.dispose();
        self.f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
}

/// **The lock is counted, not latched.** Two modals open; the inner one closes;
/// the page stays locked because the outer one still holds a lock. Both close
/// and it scrolls again.
///
/// A `bool` on the document passes until the first line of the second act and
/// then unlocks the page out from under a dialog that is still open — which is
/// the whole reason the backend counts.
#[test]
fn an_inner_modal_closing_leaves_the_outer_lock_standing() {
    let outer_open = Signal::new(true);
    let inner_open = Signal::new(true);
    let mut n = Nested::mount(outer_open, inner_open);

    wheel(&mut n.f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&n.f.app, n.f.page),
        0.0,
        "precondition: two locks held"
    );

    inner_open.set(false);
    n.f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    wheel(&mut n.f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&n.f.app, n.f.page),
        0.0,
        "one of the two closed; the other still holds the page"
    );

    outer_open.set(false);
    n.f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    wheel(&mut n.f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&n.f.app, n.f.page),
        EXPECTED_PAGE_SCROLL,
        "both closed, so the page is free"
    );
}

/// **Unmounting while open releases the lock.**
///
/// The overlay's effect is the only thing that would ever unlock, and an
/// unmounted scope's effect never runs again — so without `on_cleanup` the page
/// is unscrollable for the rest of the session with nothing on screen to explain
/// it. State armed by one event and cleared only by a second that may never
/// arrive; the cleanup is the second, independent clearing condition.
///
/// The inner modal is still **open** when it unmounts, which is the spelling
/// that matters: an unmount of a closed overlay releases nothing because it
/// holds nothing.
#[test]
fn unmounting_an_open_modal_releases_its_lock() {
    let outer_open = Signal::new(false);
    let inner_open = Signal::new(true);
    let mut n = Nested::mount(outer_open, inner_open);

    wheel(&mut n.f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&n.f.app, n.f.page),
        0.0,
        "precondition: the inner modal holds the only lock"
    );

    n.unmount_inner();

    wheel(&mut n.f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&n.f.app, n.f.page),
        EXPECTED_PAGE_SCROLL,
        "the lock went with the component"
    );
}

// ── 4. The other input route: the scrollbar ──────────────────────────────────

/// A press on the page's own scrollbar thumb is refused while the page is
/// locked, and the overlay's bar is not.
///
/// The wheel is not the only way to scroll a container on desktop: #178's
/// overlay bars are draggable, and a lock that only gated the wheel would leave
/// a 6px-wide hole in itself down the right-hand edge — under an open dialog,
/// where the pointer already is.
///
/// Asserted both ways round, because `find_scrollbar_hit` answering `None`
/// everywhere would pass the locked half on its own.
#[test]
fn a_locked_page_does_not_hand_over_its_scrollbar_thumb() {
    let open = Signal::new(true);
    let mut f = modal_over_page(open, true);

    // Where the page's vertical bar is: inside its right edge, at the top of the
    // track where the thumb starts.
    let bar = (VIEWPORT.0 - 3.0, 40.0);

    {
        let d = f.app.doc.as_ref().unwrap().borrow();
        assert!(
            find_scrollbar_hit(&d.tree, bar.0, bar.1).is_none(),
            "a locked page's bar must not be grabbable"
        );
    }

    // Press and drag it anyway: nothing moves.
    f.app.handle_event(
        PlatformEvent::MouseDown {
            x: bar.0,
            y: bar.1,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    f.app.handle_event(
        PlatformEvent::MouseMove {
            x: bar.0,
            y: bar.1 + 200.0,
        },
        (800, 600),
        1.0,
    );
    assert_eq!(
        scroll_top(&f.app, f.page),
        0.0,
        "a drag on a locked page's bar scrolls nothing"
    );

    // The same bar, unlocked, is grabbable — otherwise the assertions above are
    // about a bar that was never there.
    open.set(false);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let d = f.app.doc.as_ref().unwrap().borrow();
    let hit = find_scrollbar_hit(&d.tree, bar.0, bar.1)
        .expect("unlocked, the page's bar is exactly where the locked test looked");
    assert_eq!(
        hit.node_id, f.page,
        "and it belongs to the page, not to something else"
    );
}

// ── 5. The gate's other two branches ─────────────────────────────────────────

/// **The ancestor route**, which every fixture above leaves untested.
///
/// The wheel arm resolves a container two ways: the hit node's ancestor walk,
/// and a geometric fallback for when the hit is in another DOM branch. A modal
/// covers the viewport, so the hit is always its backdrop and the ancestor walk
/// always answers `None` — the fallback is the only branch those fixtures
/// exercise, and a gate applied to the fallback alone passes all of them.
///
/// The ancestor route is not hypothetical: it is the one a **custom overlay**
/// takes, which is what `docs/src/guide/focus.md` now tells people to build with
/// `root.set_scroll_locked(true)`. Such an overlay need not cover the viewport,
/// so the pointer lands on page content and the walk resolves the page.
///
/// The premise is asserted, not assumed: if the fixture ever stopped taking the
/// ancestor route it would silently go back to testing the fallback twice.
#[test]
fn a_lock_taken_by_a_non_covering_overlay_gates_the_ancestor_route_too() {
    // The modal stays **closed**, so nothing covers the page and no lock comes
    // from it; the lock below is the custom-overlay case the guide documents.
    let open = Signal::new(false);
    let mut f = modal_over_page(open, true);

    {
        let d = f.app.doc.as_ref().unwrap().borrow();
        let hit = hit_test(&d.tree, AIM.0, AIM.1).expect("the pointer is over the page");
        assert_eq!(
            find_scroll_container(&d.tree, hit),
            Some(f.page),
            "premise: the ancestor walk resolves the page here — without this \
             the fixture tests the geometric fallback a second time"
        );
    }

    let doc = f.app.doc.as_ref().unwrap();
    doc.borrow_mut()
        .set_scroll_locked(true, rinch_core::dom::NodeId(f.inner));

    wheel(&mut f.app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&f.app, f.page),
        0.0,
        "a lock refuses the ancestor route as well as the geometric one"
    );
}

/// **The horizontal axis.** "Both axes" was asserted by prose: no fixture above
/// passes a non-zero `delta_x`, so deleting the gate from the horizontal arm
/// changed nothing anywhere.
///
/// Checked with the lock off as well as on, in one fixture, because a page that
/// does not scroll sideways at all would pass the locked half for the wrong
/// reason.
#[test]
fn the_lock_gates_the_horizontal_axis_too() {
    let open = Signal::new(false);
    let mut f = modal_over_page(open, true);

    wheel(&mut f.app, AIM, WHEEL_DY, 0.0);
    assert_eq!(
        scroll_left(&f.app, f.page),
        EXPECTED_PAGE_SCROLL,
        "precondition: closed modal, so the page scrolls sideways"
    );

    open.set(true);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    wheel(&mut f.app, AIM, WHEEL_DY, 0.0);
    assert_eq!(
        scroll_left(&f.app, f.page),
        EXPECTED_PAGE_SCROLL,
        "and stops moving sideways once the modal locks it"
    );
}

// ── 6. A drag already in flight ──────────────────────────────────────────────

/// **A scrollbar drag armed before the lock ends when the lock arrives.**
///
/// The gate is at the *arm* (`find_scrollbar_hit`), and the `MouseMove`
/// continuation re-read nothing — so an overlay opened mid-drag by something
/// other than the user (a timer, a network reply, a menu callback) left the page
/// scrolling under the lock for as long as the button was held. State armed by
/// one event and cleared only by a second that may never arrive.
///
/// The positive control is the same sequence without the lock: without it, a
/// fixture whose drag was never armed asserts exactly the same thing.
#[test]
fn a_scrollbar_drag_already_in_flight_ends_when_the_lock_arrives() {
    for lock_midway in [false, true] {
        let open = Signal::new(false);
        let mut f = modal_over_page(open, true);
        let bar = (VIEWPORT.0 - 3.0, 40.0);

        f.app.handle_event(
            PlatformEvent::MouseDown {
                x: bar.0,
                y: bar.1,
                button: MouseButton::Left,
            },
            (800, 600),
            1.0,
        );
        assert!(
            f.app.scrollbar_drag.is_some(),
            "precondition: the press armed a drag on the page's bar"
        );
        let after_press = scroll_top(&f.app, f.page);

        if lock_midway {
            open.set(true);
            f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        }

        f.app.handle_event(
            PlatformEvent::MouseMove {
                x: bar.0,
                y: bar.1 + 200.0,
            },
            (800, 600),
            1.0,
        );

        let moved = scroll_top(&f.app, f.page) - after_press;
        if lock_midway {
            assert_eq!(moved, 0.0, "the lock arrived, so the drag is over");
            assert!(
                f.app.scrollbar_drag.is_none(),
                "and it is ended rather than left armed and inert"
            );
        } else {
            assert!(
                moved > 100.0,
                "control: with no lock the very same drag moves the page, got {moved}"
            );
        }
    }
}

// ── 7. Body portals ──────────────────────────────────────────────────────────

/// **A native `<select>` popup inside a locking modal still scrolls.**
///
/// The popup's option list is appended to `<body>` on purpose
/// (`select_widget.rs`), so it is not a descendant of the modal's root and the
/// lock refused its wheel and its thumb. `lock_scroll` defaults to `true`, so
/// that arrived in every app with a long `<select>` in a dialog, with no opt-in.
///
/// The page is asserted still locked in the same breath: the cure must be an
/// exemption for this subtree, not a hole in the lock.
#[test]
fn a_select_popup_inside_a_locking_modal_still_scrolls() {
    let open = Signal::new(true);
    let select_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let sel_in = select_id.clone();
    let page: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let page_in = page.clone();

    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let page_el = scope.create_element("div");
        page_el.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 800px; height: 600px; overflow: auto",
        );
        let tall = scope.create_element("div");
        tall.set_attribute("style", "width: 100%; height: 2000px");
        page_el.append_child(&tall);
        page_in.set(Some(page_el.node_id().0));
        root.append_child(&page_el);

        let select = scope.create_element("select");
        // Long enough to overflow the 260px popup cap several times over.
        for i in 0..40 {
            let opt = scope.create_element("option");
            let label = format!("Option {i}");
            opt.set_attribute("value", &label);
            let t = scope.create_text(&label);
            opt.append_child(&t);
            select.append_child(&opt);
        }
        sel_in.set(Some(select.node_id().0));

        let modal = Modal {
            opened_fn: Some(reactive(open)),
            lock_scroll: true,
            close_on_click_outside: false,
            ..Default::default()
        }
        .render(scope, &[select]);
        root.append_child(&modal);
        root
    });

    let page = page.get().expect("the page's node id");
    let select = select_id.get().expect("the select's node id");
    app.open_select_popup(select, VIEWPORT.0, VIEWPORT.1);
    let panel = app
        .open_select
        .as_ref()
        .expect("the popup is open")
        .panel_id;

    let max_scroll = {
        let d = app.doc.as_ref().unwrap().borrow();
        let nid = rinch_core::dom::NodeId(panel);
        d.scroll_height(nid) - d.client_height(nid)
    };
    assert!(
        max_scroll > -WHEEL_DY,
        "precondition: the option list must have more room than the gesture \
         uses, or the assertion below would sit on the clamp — got {max_scroll}"
    );

    let inside = centre(&app, panel);
    wheel(&mut app, inside, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&app, panel),
        -WHEEL_DY,
        "the popup is a body portal, so the lock must exempt it explicitly"
    );

    // And its thumb is grabbable, which the same gate refused.
    {
        let d = app.doc.as_ref().unwrap().borrow();
        let (px, py, pw, ph) = painted_element_box(&d.tree, panel);
        let hit = find_scrollbar_hit(&d.tree, px + pw - 3.0, py + ph / 2.0)
            .expect("the popup's own bar is grabbable");
        assert_eq!(hit.node_id, panel);
    }

    // The exemption is for the popup, not a hole in the lock.
    wheel(&mut app, AIM, 0.0, WHEEL_DY);
    assert_eq!(
        scroll_top(&app, page),
        0.0,
        "the page behind is still locked"
    );
}

/// The exemption leaves with the popup: closing it must not leave a hole behind
/// for whatever node id the slab hands out next.
#[test]
fn closing_the_select_popup_releases_its_exemption() {
    let open = Signal::new(true);
    let select_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let sel_in = select_id.clone();

    let mut app = mount(move |scope: &mut RenderScope| {
        let select = scope.create_element("select");
        for i in 0..40 {
            let opt = scope.create_element("option");
            let label = format!("Option {i}");
            opt.set_attribute("value", &label);
            let t = scope.create_text(&label);
            opt.append_child(&t);
            select.append_child(&opt);
        }
        sel_in.set(Some(select.node_id().0));
        Modal {
            opened_fn: Some(reactive(open)),
            lock_scroll: true,
            close_on_click_outside: false,
            ..Default::default()
        }
        .render(scope, &[select])
    });

    let select = select_id.get().expect("the select's node id");
    app.open_select_popup(select, VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .scroll_lock_exempt
            .len(),
        1,
        "precondition: the popup took an exemption"
    );

    app.close_select_popup();
    assert!(
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .scroll_lock_exempt
            .is_empty(),
        "the exemption goes with the popup"
    );
}
