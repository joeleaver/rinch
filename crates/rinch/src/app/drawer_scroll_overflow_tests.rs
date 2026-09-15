//! A closed `Drawer` inside an `overflow: auto` container grows no scrollbars
//! (issue #765), through the real component and the real stylesheet.
//!
//! `rinch-dom`'s `scrollable_overflow_tests` pin the rule on hand-written
//! `position: fixed` markup. This pins the shape the bug was *reported* on,
//! which a hand-written fixture cannot: that `Drawer`'s root really is
//! `position: fixed` with all four insets, that #761's `visibility: hidden`
//! close leaves it that way (the earlier `display: none` close hid the bug by
//! zeroing the box), and therefore that a container holding one reports only
//! its real content.
//!
//! The counterfactual in each test is the point. Swapping the drawer for an
//! ordinary in-flow box of the same size brings both bars back, so these
//! assertions are about the drawer being out of flow and not about the
//! container being empty or the stylesheet failing to load.

use super::*;

use rinch_components::Drawer;
use rinch_core::Component;
use rinch_dom::paint::scrollbar::scrollbars;

/// Small enough that a viewport-sized box overflows it on both axes.
const SCROLLER: &str = "width: 200px; height: 100px; overflow: auto";
const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// A scroll container holding a `Drawer` in the given open state, plus one
/// 100x50 in-flow child that fits.
fn mount_drawer(opened: bool) -> RinchApp {
    mount(move |scope: &mut RenderScope| {
        vec![
            Drawer {
                opened,
                ..Default::default()
            }
            .render(scope, &[]),
        ]
    })
}

/// The counterfactual: an ordinary in-flow box the size a drawer's root would
/// cover if it counted.
fn mount_in_flow_box() -> RinchApp {
    mount(|scope: &mut RenderScope| {
        let d = scope.create_element("div");
        d.set_attribute("style", "width: 800px; height: 600px");
        vec![d]
    })
}

fn mount(build: impl Fn(&mut RenderScope) -> Vec<NodeHandle> + 'static) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let container = scope.create_element("div");
        container.set_attribute("id", "scroller");
        container.set_attribute("style", SCROLLER);
        for node in build(scope) {
            container.append_child(&node);
        }
        let fits = scope.create_element("div");
        fits.set_attribute("style", "width: 100px; height: 50px");
        container.append_child(&fits);
        container
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// The id of the node carrying `id="scroller"`.
fn scroller_id(app: &RinchApp) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").is_some_and(|v| v == "scroller"))
        .map(|(id, _)| id)
        .expect("the scroll container is in the tree")
}

/// `(horizontal, vertical)` — whether each bar exists.
fn bars(app: &RinchApp) -> (bool, bool) {
    let id = scroller_id(app);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let b = scrollbars(&d.tree, id, 1.0);
    (b.horizontal.is_some(), b.vertical.is_some())
}

#[test]
fn a_closed_drawer_gives_its_scrolling_ancestor_no_scrollbars() {
    assert_eq!(
        bars(&mount_drawer(false)),
        (false, false),
        "a closed drawer is `position: fixed`, not 800x600 of scrollable content"
    );
}

/// An **open** drawer is the same box in the same place — #761 only changed how
/// the closed one is hidden — so it must answer the same way. This is also the
/// half that was already broken before #761 (the issue calls the class
/// pre-existing), so a fix that only looked at `visibility` would fail here.
#[test]
fn an_open_drawer_gives_its_scrolling_ancestor_no_scrollbars_either() {
    assert_eq!(bars(&mount_drawer(true)), (false, false));
}

/// The positive control: the same container with a real in-flow box of the same
/// size does grow both bars, so the two assertions above are discriminating.
#[test]
fn an_in_flow_box_of_the_same_size_does_grow_both_bars() {
    assert_eq!(
        bars(&mount_in_flow_box()),
        (true, true),
        "800x600 of in-flow content in a 200x100 container overflows both axes"
    );
}
