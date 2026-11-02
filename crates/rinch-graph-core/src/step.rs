//! Steps: the smallest changes to a graph, each with an exact inverse.

use crate::geom::Point;
use crate::graph::{Edge, Graph, GraphError, Node, Port};
use crate::ids::{EdgeId, NodeId, PortId};

/// One change to a [`Graph`]. Applying a step returns the step that undoes it.
///
/// Steps are deliberately small. Removing a node with edges is three kinds of
/// step (the edges, then the node), which [`Tx::remove_node`](crate::Tx::remove_node)
/// writes for you; that keeps every inverse a single step.
///
/// A view keeps itself in sync by reading the steps a change applied
/// ([`GraphState::take_applied`](crate::GraphState::take_applied)).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Step<N, P, E> {
    /// Add a node with its ports. Ids are already chosen.
    InsertNode { node: Node<N>, ports: Vec<Port<P>> },
    /// Remove a node and its ports. Refused while any port has an edge.
    RemoveNode(NodeId),
    /// Add a port to an existing node at `index` in its port list.
    InsertPort { port: Port<P>, index: usize },
    /// Remove a port. Refused while it has an edge.
    RemovePort(PortId),
    /// Put nodes at these positions.
    MoveNodes(Vec<(NodeId, Point)>),
    /// Replace a node's data.
    SetNodeData(NodeId, N),
    /// Replace a port's data.
    SetPortData(PortId, P),
    /// Add an edge. Direction and duplicates are checked; the app's rules are
    /// not (see [`Tx::connect`](crate::Tx::connect)).
    InsertEdge(Edge<E>),
    /// Remove an edge.
    RemoveEdge(EdgeId),
    /// Replace an edge's data.
    SetEdgeData(EdgeId, E),
}

impl<N, P, E> Step<N, P, E> {
    /// Apply this step, returning its inverse. A refused step changes nothing.
    pub fn apply(self, graph: &mut Graph<N, P, E>) -> Result<Step<N, P, E>, GraphError> {
        Ok(match self {
            Step::InsertNode { node, ports } => {
                let id = node.id;
                graph.raw_insert_node(node, ports)?;
                Step::RemoveNode(id)
            }
            Step::RemoveNode(id) => {
                let (node, ports) = graph.raw_remove_node(id)?;
                Step::InsertNode { node, ports }
            }
            Step::InsertPort { port, index } => {
                let id = port.id;
                graph.raw_insert_port(port, index)?;
                Step::RemovePort(id)
            }
            Step::RemovePort(id) => {
                let (port, index) = graph.raw_remove_port(id)?;
                Step::InsertPort { port, index }
            }
            Step::MoveNodes(moves) => {
                // Check every id before moving any, so a refusal changes nothing.
                if let Some((missing, _)) = moves.iter().find(|(n, _)| graph.node(*n).is_none()) {
                    return Err(GraphError::NoSuchNode(*missing));
                }
                let mut old: Vec<(NodeId, Point)> = moves
                    .into_iter()
                    .map(|(n, p)| (n, graph.raw_set_pos(n, p).expect("checked above")))
                    .collect();
                // A node listed twice: undo must restore the first old position.
                old.reverse();
                Step::MoveNodes(old)
            }
            Step::SetNodeData(id, data) => {
                Step::SetNodeData(id, graph.raw_set_node_data(id, data)?)
            }
            Step::SetPortData(id, data) => {
                Step::SetPortData(id, graph.raw_set_port_data(id, data)?)
            }
            Step::InsertEdge(edge) => {
                let id = edge.id;
                graph.raw_insert_edge(edge)?;
                Step::RemoveEdge(id)
            }
            Step::RemoveEdge(id) => Step::InsertEdge(graph.raw_remove_edge(id)?),
            Step::SetEdgeData(id, data) => {
                Step::SetEdgeData(id, graph.raw_set_edge_data(id, data)?)
            }
        })
    }

    /// The nodes this step adds, removes or changes (an edge step names the
    /// nodes at its ends when they can still be found in `graph`).
    pub fn touched_nodes(&self, graph: &Graph<N, P, E>) -> Vec<NodeId> {
        match self {
            Step::InsertNode { node, .. } => vec![node.id],
            Step::RemoveNode(id) => vec![*id],
            Step::InsertPort { port, .. } => vec![port.node],
            Step::RemovePort(id) | Step::SetPortData(id, _) => {
                graph.port(*id).map(|p| vec![p.node]).unwrap_or_default()
            }
            Step::MoveNodes(moves) => moves.iter().map(|(n, _)| *n).collect(),
            Step::SetNodeData(id, _) => vec![*id],
            Step::InsertEdge(e) => [e.from, e.to]
                .iter()
                .filter_map(|p| graph.port(*p).map(|p| p.node))
                .collect(),
            Step::RemoveEdge(id) | Step::SetEdgeData(id, _) => graph
                .edge_nodes(*id)
                .map(|(a, b)| vec![a, b])
                .unwrap_or_default(),
        }
    }
}
