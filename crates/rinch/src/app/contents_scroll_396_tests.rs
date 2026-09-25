//! A scroll region whose rows come from `rsx!` control flow scrolls (#396).
//!
//! `rsx!` wraps every `for`, `if`, `match`, reactive text and embedded
//! `Vec<NodeHandle>` in a `display: contents` element. That wrapper has no box,
//! and the scroll range used to be measured from the container's *direct*
//! children only — so an `overflow-y: auto` region holding nothing but a `for`
//! reported no content, painted no bar, and ignored the wheel.
//!
//! These drive the whole desktop path: the real macro output, a real
//! `PlatformEvent::MouseWheel` routed through `find_scroll_container` and
//! clamped by `DomDocument::scroll_height`. The Chrome 153 reference for the
//! markup (a 300px `overflow-y: auto` box around a `display: contents` wrapper
//! of forty 27px rows) is `scrollHeight` 1080, `clientHeight` 300: 780px of
//! travel.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_core::Signal;
use rinch_dom::computed_style::DisplayValue;
use rinch_macros::rsx;

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const ROWS: usize = 40;
/// Chrome 153: `scrollHeight - clientHeight` = 1080 - 300.
const TRAVEL: f64 = 780.0;

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// The one `.scroller` in the document, and whether every `.row` sits in a
/// `display: contents` wrapper somewhere below it rather than directly in it —
/// the precondition that puts a fixture on #396's shape. A `for` written
/// straight into an element is *not* wrapped (its rows are the element's own
/// children, which were always measured), so a fixture that did not check
/// this could pass unfixed.
fn scroller(app: &RinchApp) -> (usize, bool) {
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let with_class = |class: &str| -> Vec<usize> {
        d.tree
            .nodes
            .iter()
            .filter(|(_, n)| n.attributes.get("class").is_some_and(|c| c == class))
            .map(|(id, _)| id)
            .collect()
    };
    let id = with_class("scroller")[0];
    let rows = with_class("row");
    let wrapped = rows.len() == ROWS
        && rows.iter().all(|&r| {
            let parent = d.tree.nodes[r].parent.expect("a row is attached");
            parent != id && d.tree.nodes[parent].computed_style.display == DisplayValue::Contents
        });
    (id, wrapped)
}

fn scroll_top(app: &RinchApp, id: usize) -> f64 {
    let doc = app.doc.as_ref().expect("a mounted document");
    doc.borrow().scroll_top(rinch_core::dom::NodeId(id))
}

fn wheel(app: &mut RinchApp, delta_y: f64) {
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 50.0,
            y: 100.0,
            delta_x: 0.0,
            delta_y,
        },
        (VIEWPORT.0 as u32, VIEWPORT.1 as u32),
        1.0,
    );
}

/// One notch moves the region by the notch, and a long fling stops at the
/// Chrome travel — not at zero (the unfixed range) and not past the rows.
fn assert_scrolls(app: &mut RinchApp) {
    let (id, wrapped) = scroller(app);
    assert!(
        wrapped,
        "precondition: the rows sit in a display: contents wrapper"
    );
    assert_eq!(scroll_top(app, id), 0.0);
    wheel(app, -100.0);
    assert_eq!(scroll_top(app, id), 100.0, "one notch scrolls the region");
    wheel(app, -5000.0);
    assert_eq!(scroll_top(app, id), TRAVEL, "Chrome: 1080 - 300");
}

/// A `for` loop under an `if` is the whole content of an `overflow-y: auto`
/// region. The branch body is a lone control-flow node, so `rsx!` gives it a
/// `display: contents` wrapper to insert into.
#[test]
fn a_for_loop_in_an_if_in_a_scroll_region_scrolls_with_the_wheel() {
    let mut app = mount(|__scope: &mut RenderScope| {
        let loaded = Signal::new(true);
        let rows = Signal::new((0..ROWS).collect::<Vec<usize>>());
        rsx! {
            div { class: "scroller", style: "width: 200px; height: 300px; overflow-y: auto;",
                if loaded.get() {
                    for i in rows.get() {
                        div { key: i, class: "row", style: "height: 27px;" }
                    }
                }
            }
        }
    });
    assert_scrolls(&mut app);
}

/// A component whose whole output is a `for` — the other everyday shape: an
/// rsx! whose root is control flow is wrapped so it has a parent to insert
/// into.
#[test]
fn a_rows_component_in_a_scroll_region_scrolls_with_the_wheel() {
    fn rows(__scope: &mut RenderScope) -> NodeHandle {
        let rows = Signal::new((0..ROWS).collect::<Vec<usize>>());
        rsx! {
            for i in rows.get() {
                div { key: i, class: "row", style: "height: 27px;" }
            }
        }
    }
    let mut app = mount(|__scope: &mut RenderScope| {
        rsx! {
            div { class: "scroller", style: "width: 200px; height: 300px; overflow-y: auto;",
                { rows(__scope) }
            }
        }
    });
    assert_scrolls(&mut app);
}

/// Issue #396's own repro: an embedded `Vec<NodeHandle>`.
#[test]
fn an_embedded_vec_of_rows_scrolls_with_the_wheel() {
    let mut app = mount(|__scope: &mut RenderScope| {
        let rows: Vec<NodeHandle> = (0..ROWS)
            .map(|_| {
                rsx! { div { class: "row", style: "height: 27px;" } }
            })
            .collect();
        rsx! {
            div { class: "scroller", style: "width: 200px; height: 300px; overflow-y: auto;",
                {rows}
            }
        }
    });
    assert_scrolls(&mut app);
}
