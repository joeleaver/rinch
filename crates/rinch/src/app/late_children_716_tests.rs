//! A late child's default, read through a real document and a real cascade
//! (#716).
//!
//! `rinch-components`' `late_children_716` renders onto a `MockDomDocument`,
//! which is a different `DomDocument` implementation from the one an app runs
//! on: `RinchDocument` keeps its own tree, recycles slab slots, and invalidates
//! style on every mutation. The hook that carries a container's default to a
//! late child walks *that* tree — ancestor by ancestor, from the node an
//! insertion just landed in — so a mock-only fixture leaves the real walk
//! untested.
//!
//! And the mock cannot answer the question that decides whether a user sees
//! anything: the row a `for` reconcile added carries an icon box, but does the
//! glyph inside it paint? It is built by the same `render_tabler_icon` the
//! render-time path uses, in a subtree the sheet has never seen before, and a
//! rule that failed to reach it would leave a `display: none` or a zero-sized
//! `<svg>` behind every structural assertion.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_components::list::{List, ListItem};
use rinch_core::Signal;
use rinch_dom::computed_style::DisplayValue;
use rinch_macros::rsx;
use rinch_tabler_icons::TablerIcon;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Mount `build`'s tree under the real theme **and** component stylesheets.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    restyle(&app);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn restyle(app: &RinchApp) {
    let doc = app.doc.as_ref().expect("a mounted document");
    let mut d = doc.borrow_mut();
    d.load_css(&rinch_theme::generate_theme_css(
        &rinch_theme::Theme::default(),
    ));
    d.load_css(&rinch_components::generate_component_css());
    d.recompute_all_styles_full();
}

/// Every node carrying `class`, in document order, **walked from the body** —
/// a scan of the arena still finds detached nodes.
fn nodes_with_class(app: &RinchApp, class: &str) -> Vec<usize> {
    fn walk(d: &RinchDocument, id: usize, class: &str, out: &mut Vec<usize>) {
        let Some(node) = d.tree.get(id) else { return };
        if node
            .attributes
            .get("class")
            .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        {
            out.push(id);
        }
        for child in node.children.clone() {
            walk(d, child, class, out);
        }
    }
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let mut out = Vec::new();
    walk(&d, d.tree.body_id, class, &mut out);
    out
}

fn display_of(app: &RinchApp, id: usize) -> DisplayValue {
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    d.tree
        .get(id)
        .expect("the node is in the tree")
        .computed_style
        .display
}

/// The `<svg>` inside `id`'s subtree, if there is one.
fn svg_under(app: &RinchApp, id: usize) -> Option<usize> {
    fn walk(d: &RinchDocument, id: usize, out: &mut Option<usize>) {
        if out.is_some() {
            return;
        }
        let Some(node) = d.tree.get(id) else { return };
        if node.tag() == Some("svg") {
            *out = Some(id);
            return;
        }
        for child in node.children.clone() {
            walk(d, child, out);
        }
    }
    let doc = app.doc.as_ref().expect("a mounted document");
    let d = doc.borrow();
    let mut out = None;
    walk(&d, id, &mut out);
    out
}

#[test]
fn a_row_a_reconcile_adds_gets_an_icon_that_lays_out_and_paints() {
    let items = Signal::new(vec!["one"]);
    let mut app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check,
                    for it in items.get() { ListItem { key: it, {it} } }
                }
            }
        }
    });

    assert_eq!(
        nodes_with_class(&app, "rinch-list__item-icon").len(),
        1,
        "precondition: the row present at the list's render has an icon box"
    );

    items.update(|v| v.push("two"));
    restyle(&app);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    let boxes = nodes_with_class(&app, "rinch-list__item-icon");
    assert_eq!(
        boxes.len(),
        2,
        "#716: the reconciled row takes the list's icon on a real document too \
         — the hook walks `RinchDocument`'s tree, which is not the one the \
         component crate's fixtures use"
    );

    let late = *boxes.last().expect("two boxes");
    assert_ne!(
        display_of(&app, late),
        DisplayValue::None,
        "the icon box paints. It was built after the sheet was loaded, so a \
         rule that failed to reach it would leave every structural assertion \
         standing and nothing on screen"
    );

    let svg = svg_under(&app, late).expect("the box holds a rendered glyph");
    assert_ne!(display_of(&app, svg), DisplayValue::None);

    let doc = app.doc.as_ref().expect("a mounted document");
    let layout = doc.borrow().tree.get(svg).expect("the svg").layout;
    assert!(
        layout.width > 0.0 && layout.height > 0.0,
        "and it has a box: a Tabler glyph is sized `1em` off its parent's \
         font-size, so a glyph nothing sized would lay out at zero and paint \
         nothing. It measured {:?}",
        (layout.width, layout.height)
    );
}
