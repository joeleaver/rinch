//! Scroll-into-view moves **every** scrolling ancestor, innermost first, the
//! page last — as the browser's `scrollIntoView({block: "nearest", inline:
//! "nearest"})` does, which is what `rinch-web` calls (#842).
//!
//! Desktop used to adjust the nearest `overflow-y` scroller only, on the
//! vertical axis only, so a target in a scroller below the fold of a scrolling
//! page was scrolled into its scroller and left 1,000px below the window. The
//! nearest scroller's visible band was also read as its border-box origin
//! against its content-box height, so a padded scroller was scrolled by its top
//! padding when nothing needed to move (#1367).
//!
//! Every expected offset is **Chrome 156's** (`google-chrome --headless=new
//! --window-size=800,743`, which gives an 800x600 viewport; `* {
//! scrollbar-width: none }` so a classic bar does not narrow Chrome's
//! scrollport where rinch's overlay bar does not), read off
//! `scrollLeft`/`scrollTop` after `t.scrollIntoView({block: "nearest", inline:
//! "nearest"})`. The page's offsets are Chrome's `document.scrollingElement`'s;
//! rinch's page scroller is `<body>` (the UA sheet's `overflow-y: auto`). Each
//! fixture's HTML is in the test's doc, with `margin: 0` on `<body>` and `box-sizing:
//! border-box` on every `div`: rinch sizes every box as `border-box` (#1278),
//! so the scrollers say so too.
//!
//! None of the boxes holds text: the geometry is the CSS alone, on any host.
use super::*;
use std::collections::HashMap;

const VP: (f32, f32) = (800.0, 600.0);

/// The two scrollers of the 2- and 3-level fixtures: every padding and border
/// side different, so a frame mixed up on either axis moves an answer.
const S1: &str = "width: 300px; height: 200px; overflow: auto; box-sizing: border-box; \
    padding: 13px 7px 11px 9px; border: solid black; border-width: 5px 3px 4px 6px";
const S1_OUTER: &str = "width: 400px; height: 300px; overflow: auto; box-sizing: border-box; \
    padding: 17px 5px 9px 12px; border: solid black; border-width: 3px 7px 2px 4px";
const S2_INNER: &str = "width: 250px; height: 150px; overflow: auto; box-sizing: border-box; \
    padding: 6px 11px 14px 8px; border: solid black; border-width: 2px 4px 6px 3px";

/// A node of a fixture tree: `(name, style, children)`. The name is how the
/// test reads the node back (`""` for an anonymous one); `t` is the target.
struct N(&'static str, &'static str, Vec<N>);

struct Rig {
    app: RinchApp,
    ids: HashMap<&'static str, u64>,
    body: u64,
}

impl Rig {
    fn new(tree: N) -> Rig {
        let ids: Rc<RefCell<HashMap<&'static str, u64>>> = Rc::default();
        let ids_in = ids.clone();
        let tree = Rc::new(tree);
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            fn build(
                scope: &mut RenderScope,
                n: &N,
                ids: &RefCell<HashMap<&'static str, u64>>,
            ) -> NodeHandle {
                let el = scope.create_element("div");
                el.set_attribute("style", n.1);
                if !n.0.is_empty() {
                    ids.borrow_mut().insert(n.0, el.node_id().0);
                }
                for kid in &n.2 {
                    let k = build(scope, kid, ids);
                    el.append_child(&k);
                }
                el
            }
            build(scope, &tree, &ids_in)
        });
        app.mount_component(VP.0, VP.1);
        app.resolve_and_repaint(VP.0, VP.1);
        let ids = ids.borrow().clone();
        let body = {
            let d = app.doc.as_ref().unwrap().borrow();
            let root = d.tree.nodes[ids["root"]]
                .parent
                .expect("the root has a parent");
            assert_eq!(
                d.tree.nodes[root].tag(),
                Some("body"),
                "the fixture's root is mounted in <body>"
            );
            root
        };
        Rig { app, ids, body }
    }

    fn id(&self, name: &str) -> u64 {
        if name == "page" {
            self.body
        } else {
            self.ids[name]
        }
    }

    /// `(scrollLeft, scrollTop)` of a named scroller, or of the page.
    fn scroll(&self, name: &str) -> (f64, f64) {
        self.app.doc.as_ref().unwrap().borrow().tree.nodes[self.id(name)].scroll_offset
    }

    fn set_scroll(&mut self, name: &str, left: f64, top: f64) {
        let id = self.id(name);
        let doc = self.app.doc.as_ref().unwrap();
        doc.borrow_mut().tree.nodes[id].scroll_offset = (left, top);
    }

    /// One frame of the real path: the request queued, the DOM dirtied (as
    /// `set_focus_target` does), and `resolve_and_repaint` — which applies it
    /// after the layout. The viewport is nudged so the layout is not skipped.
    fn request(&mut self, fraction: Option<f32>) {
        let t = self.ids["t"];
        {
            let doc = self.app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            match fraction {
                None => d.request_scroll_into_view(rinch_core::dom::NodeId(t)),
                Some(f) => d.request_scroll_to_fraction(rinch_core::dom::NodeId(t), f, 16.0),
            }
            d.tree.dirty_nodes.insert(t);
        }
        self.app.resolve_and_repaint(VP.0, VP.1);
    }
}

fn target() -> N {
    N("t", "width: 40px; height: 30px", vec![])
}

/// Two levels: a padded, bordered scroller 1,100px down and 1,000px right of
/// a scrolling page, the target 450px down and 500px right in it.
///
/// ```html
/// <div style="width:1700px;height:2400px;padding:1100px 0 0 1000px;box-sizing:border-box">
///  <div id=s1 style="{S1}">
///   <div style="width:900px;height:700px;padding:450px 0 0 500px;box-sizing:border-box">
///    <div id=t style="width:40px;height:30px"></div></div></div></div>
/// ```
fn two_levels() -> N {
    N(
        "root",
        "width: 1700px; height: 2400px; padding: 1100px 0 0 1000px; box-sizing: border-box",
        vec![N(
            "s1",
            S1,
            vec![N(
                "",
                "width: 900px; height: 700px; padding: 450px 0 0 500px; box-sizing: border-box",
                vec![target()],
            )],
        )],
    )
}

#[test]
fn two_levels_the_page_follows_the_scroller() {
    let mut r = Rig::new(two_levels());
    r.request(None);
    // s1's scrollport is its padding box, 291x191: the target's far edges sit
    // at 9+500+40 = 549 and 13+450+30 = 493 in it.
    assert_eq!(r.scroll("s1"), (258.0, 302.0), "Chrome: s1 (258, 302)");
    assert_eq!(r.scroll("page"), (497.0, 696.0), "Chrome: page (497, 696)");
}

/// Three levels: s2 inside s1 inside the page, each off screen in the one
/// around it on both axes.
fn three_levels() -> N {
    N(
        "root",
        "width: 1700px; height: 2400px; padding: 1100px 0 0 1000px; box-sizing: border-box",
        vec![N(
            "s1",
            S1_OUTER,
            vec![N(
                "",
                "width: 1200px; height: 1500px; padding: 800px 0 0 700px; box-sizing: border-box",
                vec![N(
                    "s2",
                    S2_INNER,
                    vec![N(
                        "",
                        "width: 800px; height: 900px; padding: 600px 0 0 450px; \
                         box-sizing: border-box",
                        vec![target()],
                    )],
                )],
            )],
        )],
    )
}

#[test]
fn three_levels_every_scroller_moves() {
    let mut r = Rig::new(three_levels());
    r.request(None);
    assert_eq!(r.scroll("s2"), (255.0, 494.0), "Chrome: s2 (255, 494)");
    assert_eq!(r.scroll("s1"), (569.0, 666.0), "Chrome: s1 (569, 666)");
    assert_eq!(r.scroll("page"), (593.0, 798.0), "Chrome: page (593, 798)");
}

/// The target placed (`NodeHandle::scroll_to_fraction(0.5, 16)`, what
/// `EditorHandle::scroll_into_view_aligned` uses) in its own scroller, and
/// brought into view of every scroller further out, as the web backend's
/// `request_scroll_to_fraction` does: s2's `scrollTop` put so the target's top
/// is half way down its 142px scrollport, then a `nearest` `scrollIntoView`.
/// Chrome's numbers are that code run over the three-level page.
#[test]
fn a_placed_target_is_brought_into_view_of_the_outer_scrollers() {
    let mut r = Rig::new(three_levels());
    r.request(Some(0.5));
    assert_eq!(r.scroll("s2"), (255.0, 535.0), "Chrome: s2 (255, 535)");
    assert_eq!(r.scroll("s1"), (569.0, 625.0), "Chrome: s1 (569, 625)");
    assert_eq!(r.scroll("page"), (593.0, 798.0), "Chrome: page (593, 798)");
}

/// A scroller taller than the window. What the page brings into view is the
/// **target** (clipped to the scrollers it is in), not the scroller around it:
/// the scroller's own box (1100..2100) is taller than the window and its top
/// would be aligned (scrollTop 1100), leaving the target 1,000px below.
///
/// ```html
/// <div style="width:800px;height:3000px;padding-top:1100px;box-sizing:border-box">
///  <div id=s1 style="width:300px;height:1000px;overflow:auto;padding:10px;border:4px solid">
///   <div style="height:3000px;padding-top:2000px;box-sizing:border-box">
///    <div id=t style="width:40px;height:30px"></div></div></div></div>
/// ```
#[test]
fn a_scroller_taller_than_the_window_brings_the_target_not_its_own_top() {
    let mut r = Rig::new(N(
        "root",
        "width: 800px; height: 3000px; padding-top: 1100px; box-sizing: border-box",
        vec![N(
            "s1",
            "width: 300px; height: 1000px; overflow: auto; box-sizing: border-box; padding: 10px; \
             border: 4px solid black",
            vec![N(
                "",
                "height: 3000px; padding-top: 2000px; box-sizing: border-box",
                vec![target()],
            )],
        )],
    ));
    r.request(None);
    assert_eq!(r.scroll("s1"), (0.0, 1048.0), "Chrome: s1 (0, 1048)");
    assert_eq!(r.scroll("page"), (0.0, 1496.0), "Chrome: page (0, 1496)");
}

/// A target already in view of every scroller, each of them scrolled off its
/// origin: nothing moves.
#[test]
fn a_target_already_in_view_moves_nothing() {
    let mut r = Rig::new(N(
        "root",
        "width: 800px; height: 2400px; padding: 300px 0 0 50px; box-sizing: border-box",
        vec![N(
            "s1",
            S1,
            vec![N(
                "",
                "width: 900px; height: 700px; padding: 150px 0 0 120px; box-sizing: border-box",
                vec![target()],
            )],
        )],
    ));
    r.set_scroll("page", 0.0, 200.0);
    r.set_scroll("s1", 60.0, 100.0);
    r.request(None);
    assert_eq!(r.scroll("s1"), (60.0, 100.0), "Chrome: s1 (60, 100)");
    assert_eq!(r.scroll("page"), (0.0, 200.0), "Chrome: page (0, 200)");
}

/// A `position: fixed` box is where the walk stops: the scrollers inside it
/// move, the page under it does not — scrolling the page cannot move a box the
/// viewport holds. The panel sits below the window (top: 700px); without the
/// stop the page would scroll 197px for nothing.
#[test]
fn the_walk_stops_at_a_fixed_box() {
    let mut r = Rig::new(N(
        "root",
        "width: 800px; height: 2400px",
        vec![N(
            "",
            "position: fixed; left: 100px; top: 700px; width: 300px; height: 250px",
            vec![N(
                "s1",
                "width: 200px; height: 100px; overflow: auto; box-sizing: border-box; padding: 8px; \
                 border: 3px solid black",
                vec![N(
                    "",
                    "height: 500px; padding-top: 300px; box-sizing: border-box",
                    vec![target()],
                )],
            )],
        )],
    ));
    r.request(None);
    assert_eq!(r.scroll("s1"), (0.0, 244.0), "Chrome: s1 (0, 244)");
    assert_eq!(r.scroll("page"), (0.0, 0.0), "Chrome: page (0, 0)");
}

/// #1367: a padded scroller's scrollport is its padding box. A fully visible
/// first row moved the scroller by its top padding, and a row below landed one
/// padding past where Chrome puts it.
fn padded_rows(target_row: usize) -> N {
    let row = |i: usize| {
        N(
            if i == target_row { "t" } else { "" },
            "height: 60px",
            vec![],
        )
    };
    N(
        "root",
        "width: 800px; height: 600px",
        vec![N(
            "s1",
            "box-sizing: border-box; width: 200px; height: 100px; padding: 20px; \
             overflow: auto",
            vec![row(0), row(1), row(2)],
        )],
    )
}

#[test]
fn a_visible_first_row_in_a_padded_scroller_does_not_move_it() {
    let mut r = Rig::new(padded_rows(0));
    r.request(None);
    assert_eq!(r.scroll("s1"), (0.0, 0.0), "Chrome: s1 (0, 0)");
}

#[test]
fn a_row_below_a_padded_scrollport_lands_at_its_bottom_edge() {
    let mut r = Rig::new(padded_rows(2));
    r.request(None);
    assert_eq!(r.scroll("s1"), (0.0, 100.0), "Chrome: s1 (0, 100)");
}

/// A target taller than its scroller's scrollport: the scroller aligns its
/// **top** (a box larger than the scrollport sticking out past the end), and
/// the page then brings what the scroller shows of it — the target **clipped**
/// to the scrollport, 1000..1100 — into the window. Unclipped, the 300px
/// target (1000..1300) would have taken the page to 700.
///
/// ```html
/// <div style="width:800px;height:2400px;padding-top:1000px">
///  <div id=s1 style="width:200px;height:100px;overflow:auto">
///   <div style="height:1000px;padding-top:400px">
///    <div id=t style="width:40px;height:300px"></div></div></div></div>
/// ```
#[test]
fn the_page_brings_the_part_of_the_target_its_scroller_shows() {
    let mut r = Rig::new(N(
        "root",
        "width: 800px; height: 2400px; padding-top: 1000px; box-sizing: border-box",
        vec![N(
            "s1",
            "width: 200px; height: 100px; overflow: auto; box-sizing: border-box",
            vec![N(
                "",
                "height: 1000px; padding-top: 400px; box-sizing: border-box",
                vec![N("t", "width: 40px; height: 300px", vec![])],
            )],
        )],
    ));
    r.request(None);
    assert_eq!(r.scroll("s1"), (0.0, 400.0), "Chrome: s1 (0, 400)");
    assert_eq!(r.scroll("page"), (0.0, 500.0), "Chrome: page (0, 500)");
}
