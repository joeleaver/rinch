//! The editing state: the graph, the selection, and undo history.

use std::collections::BTreeSet;

use crate::geom::Point;
use crate::graph::{Edge, Graph, GraphError, Node, Port, PortDir};
use crate::ids::{EdgeId, NodeId, PortId};
use crate::rules::{Rules, check_connect};
use crate::step::Step;

/// The selected nodes and edges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Selection {
    pub nodes: BTreeSet<NodeId>,
    pub edges: BTreeSet<EdgeId>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }

    pub fn nodes(nodes: impl IntoIterator<Item = NodeId>) -> Self {
        Selection {
            nodes: nodes.into_iter().collect(),
            edges: BTreeSet::new(),
        }
    }

    pub fn edges(edges: impl IntoIterator<Item = EdgeId>) -> Self {
        Selection {
            nodes: BTreeSet::new(),
            edges: edges.into_iter().collect(),
        }
    }

    /// Drop anything that is no longer in `graph`.
    fn prune<N, P, E>(&mut self, graph: &Graph<N, P, E>) {
        self.nodes.retain(|n| graph.node(*n).is_some());
        self.edges.retain(|e| graph.edge(*e).is_some());
    }
}

/// One undoable change: the steps that undo it, in the order to apply them,
/// and the selection on either side.
#[derive(Clone, Debug)]
struct Entry<N, P, E> {
    steps: Vec<Step<N, P, E>>,
    selection_before: Selection,
    selection_after: Selection,
}

/// A graph being edited: the [`Graph`], the [`Selection`] and undo history.
///
/// All changes go through [`edit`](Self::edit), which groups them into one
/// undoable transaction and rolls the whole transaction back if any part is
/// refused.
#[derive(Clone, Debug)]
pub struct GraphState<N, P, E> {
    graph: Graph<N, P, E>,
    selection: Selection,
    undo: Vec<Entry<N, P, E>>,
    redo: Vec<Entry<N, P, E>>,
    /// Steps applied since the last `take_applied`, in order (forward steps,
    /// undo and redo included), for a view to mirror.
    applied: Vec<Step<N, P, E>>,
    selection_dirty: bool,
    history_limit: usize,
}

impl<N, P, E> Default for GraphState<N, P, E> {
    fn default() -> Self {
        Self::from_graph(Graph::new())
    }
}

impl<N, P, E> GraphState<N, P, E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start editing an existing graph (a loaded file, say). History starts
    /// empty.
    pub fn from_graph(graph: Graph<N, P, E>) -> Self {
        GraphState {
            graph,
            selection: Selection::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            applied: Vec::new(),
            selection_dirty: false,
            history_limit: 500,
        }
    }

    pub fn graph(&self) -> &Graph<N, P, E> {
        &self.graph
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Replace the selection. Selection changes on their own are not undo
    /// steps (an edit records the selection on either side of it).
    pub fn set_selection(&mut self, mut selection: Selection) {
        selection.prune(&self.graph);
        if selection != self.selection {
            self.selection = selection;
            self.selection_dirty = true;
        }
    }

    /// Whether the selection changed since the last call, and clear the flag.
    /// For a view to mirror.
    pub fn take_selection_changed(&mut self) -> bool {
        std::mem::take(&mut self.selection_dirty)
    }

    /// The steps applied since the last call (by edits, undo and redo), in
    /// order. For a view to mirror.
    pub fn take_applied(&mut self) -> Vec<Step<N, P, E>> {
        std::mem::take(&mut self.applied)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Forget all history.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Keep at most this many undo entries (default 500).
    pub fn set_history_limit(&mut self, limit: usize) {
        self.history_limit = limit;
        self.trim_history();
    }

    fn trim_history(&mut self) {
        if self.undo.len() > self.history_limit {
            let excess = self.undo.len() - self.history_limit;
            self.undo.drain(..excess);
        }
    }

    /// Give up the graph (to save it, say).
    pub fn into_graph(self) -> Graph<N, P, E> {
        self.graph
    }
}

impl<N: Clone, P: Clone, E: Clone> GraphState<N, P, E> {
    /// Make a change. Everything `f` does is one undoable transaction; if `f`
    /// returns an error, or any step it applies is refused, everything it did
    /// is rolled back and the error returned.
    pub fn edit<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_, N, P, E>) -> Result<R, GraphError>,
    ) -> Result<R, GraphError> {
        let selection_before = self.selection.clone();
        let mut tx = Tx {
            graph: &mut self.graph,
            selection: &mut self.selection,
            inverses: Vec::new(),
            forward: Vec::new(),
        };
        match f(&mut tx) {
            Ok(r) => {
                let Tx {
                    inverses, forward, ..
                } = tx;
                self.selection.prune(&self.graph);
                if self.selection != selection_before {
                    self.selection_dirty = true;
                }
                if !inverses.is_empty() {
                    let mut steps = inverses;
                    steps.reverse();
                    self.undo.push(Entry {
                        steps,
                        selection_before,
                        selection_after: self.selection.clone(),
                    });
                    self.redo.clear();
                    self.trim_history();
                    self.applied.extend(forward);
                }
                Ok(r)
            }
            Err(e) => {
                let Tx { inverses, .. } = tx;
                for step in inverses.into_iter().rev() {
                    step.apply(&mut self.graph)
                        .expect("an inverse of a step just applied always applies");
                }
                self.selection = selection_before;
                Err(e)
            }
        }
    }

    /// Undo the last edit. Returns `false` when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let redo = self.replay(entry.steps);
        self.selection = entry.selection_before.clone();
        self.selection.prune(&self.graph);
        self.selection_dirty = true;
        self.redo.push(Entry {
            steps: redo,
            selection_before: entry.selection_before,
            selection_after: entry.selection_after,
        });
        true
    }

    /// Redo the last undone edit. Returns `false` when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        let undo = self.replay(entry.steps);
        self.selection = entry.selection_after.clone();
        self.selection.prune(&self.graph);
        self.selection_dirty = true;
        self.undo.push(Entry {
            steps: undo,
            selection_before: entry.selection_before,
            selection_after: entry.selection_after,
        });
        true
    }

    /// Apply recorded steps in order and return the steps that reverse them,
    /// in the order to apply those.
    fn replay(&mut self, steps: Vec<Step<N, P, E>>) -> Vec<Step<N, P, E>> {
        let mut inverses = Vec::with_capacity(steps.len());
        for step in steps {
            self.applied.push(step.clone());
            let inverse = step
                .apply(&mut self.graph)
                .expect("history only holds steps that applied in this state");
            inverses.push(inverse);
        }
        inverses.reverse();
        inverses
    }
}

/// An edit in progress, handed to the closure of [`GraphState::edit`]. Each
/// method applies at once (so later calls see earlier ones) and is undone with
/// the rest if the edit fails.
pub struct Tx<'a, N, P, E> {
    graph: &'a mut Graph<N, P, E>,
    selection: &'a mut Selection,
    inverses: Vec<Step<N, P, E>>,
    forward: Vec<Step<N, P, E>>,
}

impl<N: Clone, P: Clone, E: Clone> Tx<'_, N, P, E> {
    /// The graph as this edit has left it so far.
    pub fn graph(&self) -> &Graph<N, P, E> {
        self.graph
    }

    pub fn selection(&self) -> &Selection {
        self.selection
    }

    /// Set the selection this edit leaves behind (undo restores the one
    /// before).
    pub fn set_selection(&mut self, selection: Selection) {
        *self.selection = selection;
    }

    /// Apply any step.
    pub fn apply(&mut self, step: Step<N, P, E>) -> Result<(), GraphError> {
        let forward = step.clone();
        let inverse = step.apply(self.graph)?;
        self.forward.push(forward);
        self.inverses.push(inverse);
        Ok(())
    }

    /// Add a node at `pos` with ports `(direction, data)` in display order.
    pub fn add_node(
        &mut self,
        pos: Point,
        data: N,
        ports: impl IntoIterator<Item = (PortDir, P)>,
    ) -> Result<NodeId, GraphError> {
        let id = NodeId(self.graph.alloc());
        let ports: Vec<Port<P>> = ports
            .into_iter()
            .map(|(dir, data)| Port {
                id: PortId(self.graph.alloc()),
                node: id,
                dir,
                data,
            })
            .collect();
        let node = Node {
            id,
            pos,
            ports: ports.iter().map(|p| p.id).collect(),
            data,
        };
        self.apply(Step::InsertNode { node, ports })?;
        Ok(id)
    }

    /// Add a port to `node`, at `index` in its port list (`None`: last).
    pub fn add_port(
        &mut self,
        node: NodeId,
        dir: PortDir,
        data: P,
        index: Option<usize>,
    ) -> Result<PortId, GraphError> {
        let len = self
            .graph
            .node(node)
            .ok_or(GraphError::NoSuchNode(node))?
            .ports
            .len();
        let id = PortId(self.graph.alloc());
        self.apply(Step::InsertPort {
            port: Port {
                id,
                node,
                dir,
                data,
            },
            index: index.unwrap_or(len),
        })?;
        Ok(id)
    }

    /// Remove a port and the edges on it.
    pub fn remove_port(&mut self, port: PortId) -> Result<(), GraphError> {
        for e in self.graph.port_edges(port).to_vec() {
            self.disconnect(e)?;
        }
        self.apply(Step::RemovePort(port))
    }

    /// Remove a node, its ports and every edge touching it.
    pub fn remove_node(&mut self, node: NodeId) -> Result<(), GraphError> {
        for e in self.graph.node_edges(node) {
            self.disconnect(e)?;
        }
        self.apply(Step::RemoveNode(node))?;
        self.selection.nodes.remove(&node);
        Ok(())
    }

    /// Remove everything selected: the selected edges, then the selected nodes
    /// with their edges.
    pub fn remove_selected(&mut self) -> Result<(), GraphError> {
        let Selection { nodes, edges } = self.selection.clone();
        for e in edges {
            if self.graph.edge(e).is_some() {
                self.disconnect(e)?;
            }
        }
        for n in nodes {
            self.remove_node(n)?;
        }
        Ok(())
    }

    /// Put nodes at new positions.
    pub fn move_nodes(&mut self, moves: Vec<(NodeId, Point)>) -> Result<(), GraphError> {
        if moves.is_empty() {
            return Ok(());
        }
        self.apply(Step::MoveNodes(moves))
    }

    /// Move nodes by `(dx, dy)`.
    pub fn move_by(
        &mut self,
        nodes: impl IntoIterator<Item = NodeId>,
        dx: f32,
        dy: f32,
    ) -> Result<(), GraphError> {
        let moves = nodes
            .into_iter()
            .map(|n| {
                let pos = self.graph.node(n).ok_or(GraphError::NoSuchNode(n))?.pos;
                Ok((n, pos.offset(dx, dy)))
            })
            .collect::<Result<Vec<_>, GraphError>>()?;
        self.move_nodes(moves)
    }

    pub fn set_node_data(&mut self, node: NodeId, data: N) -> Result<(), GraphError> {
        self.apply(Step::SetNodeData(node, data))
    }

    /// Change a node's data in place.
    pub fn update_node_data(
        &mut self,
        node: NodeId,
        f: impl FnOnce(&mut N),
    ) -> Result<(), GraphError> {
        let mut data = self
            .graph
            .node(node)
            .ok_or(GraphError::NoSuchNode(node))?
            .data
            .clone();
        f(&mut data);
        self.set_node_data(node, data)
    }

    pub fn set_port_data(&mut self, port: PortId, data: P) -> Result<(), GraphError> {
        self.apply(Step::SetPortData(port, data))
    }

    pub fn set_edge_data(&mut self, edge: EdgeId, data: E) -> Result<(), GraphError> {
        self.apply(Step::SetEdgeData(edge, data))
    }

    /// Connect two ports, in either order, under `rules`. Removes the edges
    /// the rules say to replace. Returns the new edge.
    pub fn connect(
        &mut self,
        rules: &(impl Rules<N, P, E> + ?Sized),
        a: PortId,
        b: PortId,
        data: E,
    ) -> Result<EdgeId, GraphError> {
        let plan = check_connect(self.graph, rules, a, b)?;
        for e in plan.replaces {
            self.disconnect(e)?;
        }
        let id = EdgeId(self.graph.alloc());
        self.apply(Step::InsertEdge(Edge {
            id,
            from: plan.from,
            to: plan.to,
            data,
        }))?;
        Ok(id)
    }

    /// Remove an edge.
    pub fn disconnect(&mut self, edge: EdgeId) -> Result<(), GraphError> {
        self.apply(Step::RemoveEdge(edge))?;
        self.selection.edges.remove(&edge);
        Ok(())
    }
}
