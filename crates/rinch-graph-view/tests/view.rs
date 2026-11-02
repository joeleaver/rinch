//! The view handle without a document: edits mirror into its signals, and the
//! geometry falls back sensibly before anything is measured.

use rinch_core::untracked;
use rinch_graph_view::{
    AnyConnection, Flow, GraphState, GraphView, NodeId, Point, PortDir, PortId, Selection, Viewport,
};

type View = GraphView<&'static str, (), ()>;

fn two_nodes(view: &View) -> (NodeId, PortId, NodeId, PortId) {
    let (a, b) = view
        .edit(|tx| {
            let a = tx.add_node(Point::new(0.0, 0.0), "a", [(PortDir::Out, ())])?;
            let b = tx.add_node(Point::new(200.0, 100.0), "b", [(PortDir::In, ())])?;
            Ok((a, b))
        })
        .unwrap();
    let port = |n| view.with_state(|s| s.graph().node(n).unwrap().ports[0]);
    (a, port(a), b, port(b))
}

#[test]
fn edits_undo_and_selection_reach_the_signals() {
    let view: View = GraphView::new(GraphState::new(), AnyConnection);
    assert!(!untracked(|| view.can_undo()));
    let (a, out, b, input) = two_nodes(&view);
    let e = view.connect(input, out).expect("either order connects");
    assert!(untracked(|| view.can_undo()));
    assert_eq!(view.with_state(|s| s.graph().edge_count()), 1);

    view.set_selection(Selection::nodes([a, b]));
    assert_eq!(untracked(|| view.selection()).nodes.len(), 2);
    view.delete_selected();
    assert_eq!(view.with_state(|s| s.graph().node_count()), 0);
    assert!(untracked(|| view.selection()).is_empty());

    assert!(view.undo());
    assert!(view.with_state(|s| s.graph().edge(e).is_some()));
    assert_eq!(
        untracked(|| view.selection()).nodes.len(),
        2,
        "undo restores the selection"
    );
    assert!(untracked(|| view.can_redo()));
}

#[test]
fn unmeasured_ports_attach_to_the_node_side_the_flow_names() {
    let view: View = GraphView::new(GraphState::new(), AnyConnection);
    let (_, out, _, input) = two_nodes(&view);
    // Nothing is mounted: nodes have no size yet, so both ends sit at the
    // node origins.
    assert_eq!(
        untracked(|| view.port_point(out)),
        Some(Point::new(0.0, 0.0))
    );
    assert_eq!(
        untracked(|| view.port_point(input)),
        Some(Point::new(200.0, 100.0))
    );
    view.configure(|c| c.flow = Flow::TopToBottom);
    assert_eq!(
        untracked(|| view.port_point(input)),
        Some(Point::new(200.0, 100.0))
    );
}

#[test]
fn edge_picking_uses_the_drawn_curve() {
    let view: View = GraphView::new(GraphState::new(), AnyConnection);
    let (_, out, _, input) = two_nodes(&view);
    let e = view.connect(out, input).unwrap();
    // The bezier from (0,0) to (200,100) passes through its midpoint.
    assert_eq!(view.edge_at(Point::new(100.0, 50.0), 4.0), Some(e));
    assert_eq!(view.edge_at(Point::new(100.0, 90.0), 4.0), None);
}

#[test]
fn zoom_is_clamped_and_kept_around_the_anchor() {
    let view: View = GraphView::new(GraphState::new(), AnyConnection);
    view.configure(|c| {
        c.min_zoom = 0.5;
        c.max_zoom = 2.0;
    });
    view.zoom_around(10.0, Point::new(100.0, 100.0));
    let vp = untracked(|| view.viewport());
    assert_eq!(vp.zoom, 2.0);
    assert_eq!(
        vp.to_graph(Point::new(100.0, 100.0)),
        Point::new(100.0, 100.0)
    );
    view.set_viewport(Viewport::default());
    view.zoom_around(0.01, Point::ZERO);
    assert_eq!(untracked(|| view.viewport()).zoom, 0.5);
}

#[test]
fn a_view_is_copy() {
    let view: View = GraphView::new(GraphState::new(), AnyConnection);
    let copy = view;
    let _ = copy.edit(|tx| tx.add_node(Point::ZERO, "x", []));
    assert_eq!(view.with_state(|s| s.graph().node_count()), 1);
}
