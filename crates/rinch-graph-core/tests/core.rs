use rinch_graph_core::{
    AnyConnection, EdgeId, GraphError, GraphState, NodeId, Point, Port, PortDir, PortId, Rules,
    Selection, Step, check_connect,
};

/// Port payload for a typed graph: the value type.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ty {
    Float,
    Vec3,
}

type State = GraphState<&'static str, Ty, ()>;

struct Typed;
impl Rules<&'static str, Ty, ()> for Typed {
    fn can_connect(
        &self,
        _: &rinch_graph_core::Graph<&'static str, Ty, ()>,
        from: &Port<Ty>,
        to: &Port<Ty>,
    ) -> bool {
        from.data == to.data
    }
}

/// A node with one input and one output of `ty`.
fn pass(state: &mut State, name: &'static str, ty: Ty) -> (NodeId, PortId, PortId) {
    let id = state
        .edit(|tx| tx.add_node(Point::ZERO, name, [(PortDir::In, ty), (PortDir::Out, ty)]))
        .unwrap();
    let ports = &state.graph().node(id).unwrap().ports;
    (id, ports[0], ports[1])
}

/// The graph's contents. Not the id counter, which keeps counting through
/// undo (ids are never reused).
fn snapshot(state: &State) -> String {
    graph_snapshot(state.graph())
}

fn graph_snapshot(g: &rinch_graph_core::Graph<&'static str, Ty, ()>) -> String {
    let mut out = String::new();
    for n in g.nodes() {
        out += &format!("{n:?}\n");
    }
    for p in g.ports() {
        out += &format!("{p:?} edges={:?}\n", g.port_edges(p.id));
    }
    for e in g.edges() {
        out += &format!("{e:?}\n");
    }
    out
}

#[test]
fn connect_in_either_order_and_undo() {
    let mut s = State::new();
    let (_, _, a_out) = pass(&mut s, "a", Ty::Float);
    let (_, b_in, _) = pass(&mut s, "b", Ty::Float);
    // Dragged from the input back to the output.
    let e = s
        .edit(|tx| tx.connect(&AnyConnection, b_in, a_out, ()))
        .unwrap();
    let edge = s.graph().edge(e).unwrap();
    assert_eq!((edge.from, edge.to), (a_out, b_in));
    assert!(s.undo());
    assert_eq!(s.graph().edge_count(), 0);
    assert!(s.redo());
    assert_eq!(
        s.graph().edge(e).unwrap().from,
        a_out,
        "redo restores the same id"
    );
}

#[test]
fn refusals() {
    let mut s = State::new();
    let (a, a_in, a_out) = pass(&mut s, "a", Ty::Float);
    let (_, b_in, b_out) = pass(&mut s, "b", Ty::Vec3);
    let (_, c_in, _) = pass(&mut s, "c", Ty::Float);
    let g = s.graph();
    assert_eq!(
        check_connect(g, &AnyConnection, a_out, b_out),
        Err(GraphError::WrongDirection)
    );
    assert_eq!(
        check_connect(g, &AnyConnection, a_out, a_in),
        Err(GraphError::WouldCycle),
        "a self-loop is a cycle"
    );
    assert_eq!(
        check_connect(g, &Typed, a_out, b_in),
        Err(GraphError::Denied)
    );
    assert!(check_connect(g, &Typed, a_out, c_in).is_ok());
    let _ = (a, b_out);
}

#[test]
fn cycles_are_refused_across_nodes() {
    let mut s = State::new();
    let (_, a_in, a_out) = pass(&mut s, "a", Ty::Float);
    let (_, b_in, b_out) = pass(&mut s, "b", Ty::Float);
    let (_, c_in, c_out) = pass(&mut s, "c", Ty::Float);
    s.edit(|tx| {
        tx.connect(&AnyConnection, a_out, b_in, ())?;
        tx.connect(&AnyConnection, b_out, c_in, ())
    })
    .unwrap();
    assert_eq!(
        s.edit(|tx| tx.connect(&AnyConnection, c_out, a_in, ())),
        Err(GraphError::WouldCycle)
    );

    struct Cyclic;
    impl Rules<&'static str, Ty, ()> for Cyclic {
        fn allow_cycles(&self) -> bool {
            true
        }
    }
    s.edit(|tx| tx.connect(&Cyclic, c_out, a_in, ())).unwrap();
    assert!(s.graph().topo_order().is_err());
}

#[test]
fn a_full_input_is_replaced_in_one_undo_step() {
    let mut s = State::new();
    let (_, _, a_out) = pass(&mut s, "a", Ty::Float);
    let (_, _, b_out) = pass(&mut s, "b", Ty::Float);
    let (_, c_in, _) = pass(&mut s, "c", Ty::Float);
    let first = s
        .edit(|tx| tx.connect(&AnyConnection, a_out, c_in, ()))
        .unwrap();
    let second = s
        .edit(|tx| tx.connect(&AnyConnection, b_out, c_in, ()))
        .unwrap();
    assert!(s.graph().edge(first).is_none());
    assert_eq!(s.graph().port_edges(c_in), &[second]);
    s.undo();
    assert_eq!(
        s.graph().port_edges(c_in),
        &[first],
        "one undo brings the old wire back"
    );

    struct NoReplace;
    impl Rules<&'static str, Ty, ()> for NoReplace {
        fn replace_when_full(&self) -> bool {
            false
        }
    }
    assert_eq!(
        s.edit(|tx| tx.connect(&NoReplace, b_out, c_in, ())),
        Err(GraphError::PortFull(c_in))
    );
}

#[test]
fn removing_a_node_takes_its_edges_and_undo_restores_everything() {
    let mut s = State::new();
    let (_, _, a_out) = pass(&mut s, "a", Ty::Float);
    let (b, b_in, b_out) = pass(&mut s, "b", Ty::Float);
    let (_, c_in, _) = pass(&mut s, "c", Ty::Float);
    s.edit(|tx| {
        tx.connect(&AnyConnection, a_out, b_in, ())?;
        tx.connect(&AnyConnection, b_out, c_in, ())
    })
    .unwrap();
    let before = snapshot(&s);
    s.set_selection(Selection::nodes([b]));
    s.edit(|tx| tx.remove_selected()).unwrap();
    assert_eq!(s.graph().node_count(), 2);
    assert_eq!(s.graph().edge_count(), 0);
    assert!(
        s.selection().is_empty(),
        "the removed node left the selection"
    );
    s.undo();
    assert_eq!(snapshot(&s), before);
    assert_eq!(
        s.selection(),
        &Selection::nodes([b]),
        "undo restores the selection"
    );
}

#[test]
fn a_failed_edit_changes_nothing() {
    let mut s = State::new();
    let (a, _, a_out) = pass(&mut s, "a", Ty::Float);
    let (_, b_in, _) = pass(&mut s, "b", Ty::Vec3);
    let before = snapshot(&s);
    let r = s.edit(|tx| {
        tx.move_by([a], 50.0, 0.0)?;
        tx.add_node(Point::ZERO, "x", [])?;
        tx.connect(&Typed, a_out, b_in, ()) // refused: types differ
    });
    assert_eq!(r, Err(GraphError::Denied));
    assert_eq!(snapshot(&s), before);
    s.take_applied();
    assert!(s.undo(), "the node creations before are still undoable");
    assert!(
        s.take_applied().len() == 1,
        "and the failed edit left no entry"
    );
}

#[test]
fn moves_undo_to_the_first_position() {
    let mut s = State::new();
    let (a, _, _) = pass(&mut s, "a", Ty::Float);
    s.edit(|tx| tx.move_nodes(vec![(a, Point::new(1.0, 1.0)), (a, Point::new(2.0, 2.0))]))
        .unwrap();
    assert_eq!(s.graph().node(a).unwrap().pos, Point::new(2.0, 2.0));
    s.undo();
    assert_eq!(s.graph().node(a).unwrap().pos, Point::ZERO);
}

#[test]
fn every_step_round_trips() {
    let mut s = State::new();
    let (a, _, a_out) = pass(&mut s, "a", Ty::Float);
    let (b, b_in, _) = pass(&mut s, "b", Ty::Float);
    let mut seen: Vec<String> = vec![snapshot(&s)];
    let e = s
        .edit(|tx| tx.connect(&AnyConnection, a_out, b_in, ()))
        .unwrap();
    seen.push(snapshot(&s));
    s.edit(|tx| tx.set_node_data(a, "renamed")).unwrap();
    seen.push(snapshot(&s));
    s.edit(|tx| tx.set_edge_data(e, ())).unwrap();
    seen.push(snapshot(&s));
    let p = s
        .edit(|tx| tx.add_port(b, PortDir::In, Ty::Vec3, Some(0)))
        .unwrap();
    assert_eq!(
        s.graph().node(b).unwrap().ports[0],
        p,
        "inserted at the index"
    );
    seen.push(snapshot(&s));
    s.edit(|tx| tx.set_port_data(p, Ty::Float)).unwrap();
    seen.push(snapshot(&s));
    s.edit(|tx| tx.remove_port(b_in)).unwrap();
    assert_eq!(s.graph().edge_count(), 0, "removing a port takes its edge");
    seen.push(snapshot(&s));

    // Undo everything, checking each state on the way back.
    for expected in seen.iter().rev().skip(1) {
        assert!(s.undo());
        assert_eq!(&snapshot(&s), expected);
    }
    // Back through the two node creations to an empty graph.
    s.undo();
    s.undo();
    assert_eq!(s.graph().node_count(), 0);
    assert!(!s.can_undo());
    // And forward again to the end.
    while s.redo() {}
    assert_eq!(&snapshot(&s), seen.last().unwrap());
}

#[test]
fn steps_refuse_bad_input_without_changing_anything() {
    let mut s = State::new();
    let (a, a_in, a_out) = pass(&mut s, "a", Ty::Float);
    let (_, b_in, _) = pass(&mut s, "b", Ty::Float);
    s.edit(|tx| tx.connect(&AnyConnection, a_out, b_in, ()))
        .unwrap();
    let before = snapshot(&s);
    let mut g = s.graph().clone();
    assert_eq!(
        Step::RemoveNode::<&str, Ty, ()>(a).apply(&mut g),
        Err(GraphError::StillConnected)
    );
    assert_eq!(
        Step::<&str, Ty, ()>::MoveNodes(vec![
            (a, Point::new(5.0, 5.0)),
            (NodeId(999), Point::ZERO)
        ])
        .apply(&mut g),
        Err(GraphError::NoSuchNode(NodeId(999)))
    );
    assert_eq!(
        Step::<&str, Ty, ()>::RemoveEdge(EdgeId(999)).apply(&mut g),
        Err(GraphError::NoSuchEdge(EdgeId(999)))
    );
    assert_eq!(graph_snapshot(&g), before);
    let _ = a_in;
}

#[test]
fn topo_order_and_reachability() {
    let mut s = State::new();
    let (a, _, a_out) = pass(&mut s, "a", Ty::Float);
    let (b, b_in, b_out) = pass(&mut s, "b", Ty::Float);
    let (c, c_in, _) = pass(&mut s, "c", Ty::Float);
    let (d, _, _) = pass(&mut s, "d", Ty::Float);
    // c <- b <- a, d alone. Created so that id order is not the answer.
    s.edit(|tx| {
        tx.connect(&AnyConnection, b_out, c_in, ())?;
        tx.connect(&AnyConnection, a_out, b_in, ())
    })
    .unwrap();
    let g = s.graph();
    let order = g.topo_order().unwrap();
    let pos = |n| order.iter().position(|x| *x == n).unwrap();
    assert!(pos(a) < pos(b) && pos(b) < pos(c));
    assert_eq!(order.len(), 4);
    assert!(g.reaches(a, c));
    assert!(!g.reaches(c, a));
    assert_eq!(g.upstream(c).into_iter().collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(g.downstream(a).into_iter().collect::<Vec<_>>(), vec![b, c]);
    assert!(g.upstream(d).is_empty());
}

#[test]
fn max_inputs_above_one_keeps_the_newest() {
    struct Many;
    impl Rules<&'static str, Ty, ()> for Many {
        fn max_inputs(&self, _: &Port<Ty>) -> Option<usize> {
            Some(2)
        }
    }
    let mut s = State::new();
    let outs: Vec<PortId> = (0..3).map(|_| pass(&mut s, "src", Ty::Float).2).collect();
    let (_, sink, _) = pass(&mut s, "sink", Ty::Float);
    let edges: Vec<EdgeId> = outs
        .iter()
        .map(|o| s.edit(|tx| tx.connect(&Many, *o, sink, ())).unwrap())
        .collect();
    assert_eq!(
        s.graph().port_edges(sink),
        &edges[1..],
        "the oldest was replaced"
    );
}

#[test]
fn the_applied_log_mirrors_edits_undo_and_redo() {
    let mut s = State::new();
    let (a, _, _) = pass(&mut s, "a", Ty::Float);
    s.take_applied();
    s.edit(|tx| tx.move_by([a], 10.0, 0.0)).unwrap();
    assert!(matches!(s.take_applied().as_slice(), [Step::MoveNodes(_)]));
    s.undo();
    match s.take_applied().as_slice() {
        [Step::MoveNodes(m)] => assert_eq!(m, &vec![(a, Point::ZERO)]),
        other => panic!("{other:?}"),
    }
    s.redo();
    assert_eq!(s.take_applied().len(), 1);
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip_rebuilds_indexes() {
    use rinch_graph_core::Graph;
    let mut s: GraphState<String, String, ()> = GraphState::new();
    let (a, b) = s
        .edit(|tx| {
            let a = tx.add_node(Point::ZERO, "a".into(), [(PortDir::Out, "f".into())])?;
            let b = tx.add_node(Point::ZERO, "b".into(), [(PortDir::In, "f".into())])?;
            Ok((a, b))
        })
        .unwrap();
    let (out, inp) = (
        s.graph().node(a).unwrap().ports[0],
        s.graph().node(b).unwrap().ports[0],
    );
    let e = s
        .edit(|tx| tx.connect(&AnyConnection, out, inp, ()))
        .unwrap();
    let json = serde_json::to_string(s.graph()).unwrap();
    let mut back: Graph<String, String, ()> = serde_json::from_str(&json).unwrap();
    back.reindex();
    assert_eq!(&back, s.graph());
    assert_eq!(back.port_edges(inp), &[e]);
    // New ids do not collide with loaded ones.
    let mut loaded = GraphState::from_graph(back);
    let c = loaded
        .edit(|tx| tx.add_node(Point::ZERO, "c".into(), []))
        .unwrap();
    assert!(c.0 > e.0);
}
