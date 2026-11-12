//! A change under one row of a reactive tree redraws what changed, not every
//! row above it.
//!
//! The rows of a `data_source` tree used to compare by their whole subtree,
//! and a row's children were drawn from the snapshot the row was drawn with.
//! So a child added three levels down made every ancestor compare unequal,
//! the root's list redrew the root, and with it every row of the tree: found
//! in Pimble, where an agent adding notes rebuilt the whole explorer for each
//! one (and every row's DOM was kept, rinch#723).

use super::*;
use std::cell::{Cell, RefCell};

use rinch_components::{RenderTreeNode, Tree, TreeNodeData, UseTreeOptions, UseTreeReturn};
use rinch_core::Component;
use rinch_core::reactive::Signal;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

struct P {
    app: RinchApp,
    data: Signal<Vec<TreeNodeData>>,
    /// Rows drawn, by value, in order.
    drawn: Rc<RefCell<Vec<String>>>,
    _state: UseTreeReturn,
}

fn node(value: &str, children: Vec<TreeNodeData>) -> TreeNodeData {
    TreeNodeData::new(value, value.to_uppercase()).with_children(children)
}

/// A root with two rows, each with two children, everything expanded.
fn shape(extra: Option<&str>) -> Vec<TreeNodeData> {
    let mut a1 = vec![node("a1x", vec![])];
    if let Some(extra) = extra {
        a1.push(node(extra, vec![]));
    }
    vec![node(
        "root",
        vec![
            node("a", vec![node("a1", a1), node("a2", vec![])]),
            node("b", vec![node("b1", vec![]), node("b2", vec![])]),
        ],
    )]
}

fn build() -> P {
    let drawn: Rc<RefCell<Vec<String>>> = Rc::default();
    let signal: Rc<Cell<Option<Signal<Vec<TreeNodeData>>>>> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: ["root", "a", "a1", "b"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ..Default::default()
    });
    let (d, sig) = (drawn.clone(), signal.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let data = Signal::new(shape(None));
        sig.set(Some(data));
        let d = d.clone();
        let render: RenderTreeNode = Rc::new(move |payload, scope: &mut RenderScope| {
            d.borrow_mut().push(payload.node_value.to_string());
            let span = scope.create_element("span");
            span.append_child(&scope.create_text(&payload.node.label));
            span
        });
        let tree = Tree {
            data: data.get(),
            data_source: Some(Rc::new(move || data.get())),
            tree: Some(state),
            render_node: Some(render),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&tree);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    P {
        app,
        data: signal.get().unwrap(),
        drawn,
        _state: state,
    }
}

fn shown(app: &RinchApp) -> Vec<String> {
    let doc = app.doc.as_ref().unwrap().borrow();
    let mut out = Vec::new();
    for (_, n) in doc.tree.nodes.iter() {
        if n.parent.is_some() && n.attributes.get("role").map(String::as_str) == Some("none") {
            if let Some(v) = n.attributes.get("data-value") {
                out.push(v.clone());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_child_added_deep_down_draws_only_that_child() {
    let mut p = build();
    let first: Vec<String> = p.drawn.borrow().clone();
    assert_eq!(first.len(), 8, "premise: every row drawn once: {first:?}");
    p.drawn.borrow_mut().clear();

    p.data.set(shape(Some("a1y")));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        *p.drawn.borrow(),
        vec!["a1y".to_string()],
        "only the new row is drawn"
    );
    assert!(
        shown(&p.app).contains(&"a1y".to_string()),
        "the new row is in the tree"
    );
}

#[test]
fn a_child_removed_deep_down_draws_nothing_and_goes() {
    let mut p = build();
    p.data.set(shape(Some("a1y")));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    p.drawn.borrow_mut().clear();

    p.data.set(shape(None));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    assert!(
        p.drawn.borrow().is_empty(),
        "no row drawn again: {:?}",
        p.drawn.borrow()
    );
    assert!(
        !shown(&p.app).contains(&"a1y".to_string()),
        "the removed row is gone"
    );
}

#[test]
fn a_row_relabelled_is_drawn_again_alone() {
    let mut p = build();
    p.drawn.borrow_mut().clear();
    let mut data = shape(None);
    data[0].children[1].children[0].label = "B ONE".into();
    p.data.set(data);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(*p.drawn.borrow(), vec!["b1".to_string()]);
}

#[test]
fn a_row_given_its_first_child_is_drawn_again_with_it() {
    let mut p = build();
    p.drawn.borrow_mut().clear();
    let mut data = shape(None);
    data[0].children[0].children[1]
        .children
        .push(node("a2x", vec![]));
    p.data.set(data);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    // `a2` grows a chevron and a subtree, so it is drawn again with its child.
    assert_eq!(*p.drawn.borrow(), vec!["a2".to_string(), "a2x".to_string()]);
}
