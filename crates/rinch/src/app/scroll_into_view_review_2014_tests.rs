//! Review of PR #2014 (#842): probes of rinch's own UI under the new
//! every-scroller walk.
use super::*;
use std::cell::{Cell, RefCell};

use rinch_components::Modal;
use rinch_core::{Component, Signal};

const VP: (f32, f32) = (800.0, 600.0);

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VP.0, VP.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0, VP.1);
    app
}

fn body_of(app: &RinchApp) -> u64 {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.tag() == Some("body"))
        .map(|(id, _)| id)
        .unwrap()
}

fn off(app: &RinchApp, id: u64) -> (f64, f64) {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id].scroll_offset
}

fn set_off(app: &RinchApp, id: u64, o: (f64, f64)) {
    app.doc.as_ref().unwrap().borrow_mut().tree.nodes[id].scroll_offset = o;
}

fn request(app: &mut RinchApp, t: u64) {
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.request_scroll_into_view(rinch_core::dom::NodeId(t));
        d.tree.dirty_nodes.insert(t);
    }
    app.resolve_and_repaint(VP.0, VP.1);
}

/// A lock_scroll Modal over a <body>-scrolled page: a target in the modal's own
/// scroller, below its scrollport. The scroller moves, the page does not.
#[test]
fn review_2014_modal_reveal_does_not_scroll_the_page_behind() {
    let open = Signal::new(true);
    let t_id = Rc::new(Cell::new(0u64));
    let s_id = Rc::new(Cell::new(0u64));
    let (ti, si) = (t_id.clone(), s_id.clone());
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 3000px");
        let inner = scope.create_element("div");
        inner.set_attribute("style", "width: 300px; height: 100px; overflow: auto");
        let pad = scope.create_element("div");
        pad.set_attribute("style", "height: 400px");
        let t = scope.create_element("div");
        t.set_attribute("style", "width: 40px; height: 30px");
        inner.append_child(&pad);
        inner.append_child(&t);
        ti.set(t.node_id().0);
        si.set(inner.node_id().0);
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            lock_scroll: true,
            ..Default::default()
        }
        .render(scope, &[inner]);
        root.append_child(&modal);
        root
    });
    let body = body_of(&app);
    set_off(&app, body, (0.0, 500.0));
    request(&mut app, t_id.get());
    let inner = off(&app, s_id.get());
    let page = off(&app, body);
    assert!(
        inner.1 > 0.0,
        "positive control: the modal's own scroller moved"
    );
    assert_eq!(page, (0.0, 500.0), "the page behind the modal stays put");
}

/// A scroller that is itself `position: fixed` (a fixed sidebar): it is moved,
/// and the page is not.
#[test]
fn review_2014_a_fixed_scroller_is_itself_scrolled() {
    let t_id = Rc::new(Cell::new(0u64));
    let s_id = Rc::new(Cell::new(0u64));
    let (ti, si) = (t_id.clone(), s_id.clone());
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 3000px");
        let side = scope.create_element("div");
        side.set_attribute(
            "style",
            "position: fixed; left: 0; top: 0; width: 200px; height: 300px; overflow: auto",
        );
        let pad = scope.create_element("div");
        pad.set_attribute("style", "height: 900px");
        let t = scope.create_element("div");
        t.set_attribute("style", "width: 40px; height: 30px");
        side.append_child(&pad);
        side.append_child(&t);
        root.append_child(&side);
        ti.set(t.node_id().0);
        si.set(side.node_id().0);
        root
    });
    let body = body_of(&app);
    set_off(&app, body, (0.0, 500.0));
    request(&mut app, t_id.get());
    let side = off(&app, s_id.get());
    let page = off(&app, body);
    assert_eq!(side, (0.0, 630.0), "Chrome: 900 + 30 - 300");
    assert_eq!(page, (0.0, 500.0));
}

fn plain(tree: &'static [(&'static str, &'static str, usize)]) -> (RinchApp, Vec<u64>) {
    // A chain: each (name, style, depth) nests in the previous one.
    let ids: Rc<RefCell<Vec<u64>>> = Rc::default();
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let mut stack: Vec<NodeHandle> = Vec::new();
        let mut root = None;
        for (_, style, depth) in tree {
            let el = scope.create_element("div");
            el.set_attribute("style", style);
            ids_in.borrow_mut().push(el.node_id().0);
            stack.truncate(*depth);
            if let Some(p) = stack.last() {
                p.append_child(&el);
            } else {
                root = Some(el.clone());
            }
            stack.push(el);
        }
        root.unwrap()
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    let v = ids.borrow().clone();
    (app, v)
}

/// The page brings the target clipped by **every** inner scrollport, not the
/// innermost-but-one alone. Chrome 156: s2 (0, 400), s1 (0, 0), page (0, 1200).
/// Clipped by s1 only, the page goes to 1400.
#[test]
fn review_2014_the_page_sees_the_target_clipped_by_every_inner_scroller() {
    static T: &[(&str, &str, usize)] = &[
        (
            "root",
            "width: 800px; height: 3000px; padding-top: 1500px; box-sizing: border-box",
            0,
        ),
        (
            "s1",
            "width: 400px; height: 500px; overflow: auto; box-sizing: border-box",
            1,
        ),
        (
            "",
            "height: 1500px; padding-top: 200px; box-sizing: border-box",
            2,
        ),
        (
            "s2",
            "width: 200px; height: 100px; overflow: auto; box-sizing: border-box",
            3,
        ),
        (
            "",
            "height: 1000px; padding-top: 400px; box-sizing: border-box",
            4,
        ),
        ("t", "width: 40px; height: 300px", 5),
    ];
    let (mut app, ids) = plain(T);
    let body = body_of(&app);
    request(&mut app, ids[5]);
    assert_eq!(off(&app, ids[3]), (0.0, 400.0), "Chrome: s2 (0, 400)");
    assert_eq!(off(&app, ids[1]), (0.0, 0.0), "Chrome: s1 (0, 0)");
    assert_eq!(off(&app, body), (0.0, 1200.0), "Chrome: page (0, 1200)");
}

/// A target larger than the scrollport that covers it moves nothing.
/// Chrome 156: s1 stays at (0, 450).
#[test]
fn review_2014_a_covering_target_moves_nothing() {
    static T: &[(&str, &str, usize)] = &[
        ("root", "width: 800px; height: 600px", 0),
        (
            "s1",
            "width: 200px; height: 100px; overflow: auto; box-sizing: border-box",
            1,
        ),
        (
            "",
            "height: 1000px; padding-top: 400px; box-sizing: border-box",
            2,
        ),
        ("t", "width: 40px; height: 300px", 3),
    ];
    let (mut app, ids) = plain(T);
    set_off(&app, ids[1], (0.0, 450.0));
    request(&mut app, ids[3]);
    assert_eq!(off(&app, ids[1]), (0.0, 450.0), "Chrome: s1 (0, 450)");
}

/// A placed request skips an x-only scroller and is placed in the first
/// ancestor that scrolls **vertically** (the web's `overflow-y: auto|scroll`
/// ancestor): s1.scrollTop = 1000 - clamp(0.5 * 200, 16, 200 - 16 - 30) = 900.
/// `nearest` there would give 830.
#[test]
fn review_2014_placement_skips_an_x_only_scroller() {
    static T: &[(&str, &str, usize)] = &[
        ("root", "width: 800px; height: 600px", 0),
        (
            "s1",
            "width: 400px; height: 200px; overflow-y: auto; box-sizing: border-box",
            1,
        ),
        (
            "",
            "height: 2000px; padding-top: 1000px; box-sizing: border-box",
            2,
        ),
        (
            "sx",
            "width: 300px; height: 100px; overflow-x: auto; overflow-y: hidden; box-sizing: border-box",
            3,
        ),
        ("", "width: 900px; height: 100px", 4),
        ("t", "width: 40px; height: 30px", 5),
    ];
    let (mut app, ids) = plain(T);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.request_scroll_to_fraction(rinch_core::dom::NodeId(ids[5]), 0.5, 16.0);
        d.tree.dirty_nodes.insert(ids[5]);
    }
    app.resolve_and_repaint(VP.0, VP.1);
    assert_eq!(off(&app, ids[1]).1, 900.0, "placed in s1, not nearest");
}
