//! The graph: nodes, ports, edges and the indexes between them.
//!
//! Reading is public. Changing goes through [`Step`](crate::Step)s (usually
//! via [`GraphState::edit`](crate::GraphState::edit)) so that every change has
//! an inverse; the raw mutators here are crate-private.

use std::collections::BTreeMap;
use std::fmt;

use crate::geom::Point;
use crate::ids::{EdgeId, NodeId, PortId};

/// Which way data flows through a port. Edges always run from an `Out` port to
/// an `In` port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PortDir {
    In,
    Out,
}

/// A node: a position in graph space, its ports in order, and the app's data.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Node<N> {
    pub id: NodeId,
    pub pos: Point,
    /// The node's ports, in display order.
    pub ports: Vec<PortId>,
    pub data: N,
}

/// A port on a node.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Port<P> {
    pub id: PortId,
    pub node: NodeId,
    pub dir: PortDir,
    pub data: P,
}

/// An edge from an `Out` port to an `In` port.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Edge<E> {
    pub id: EdgeId,
    pub from: PortId,
    pub to: PortId,
    pub data: E,
}

/// Why a change was refused. A refused [`GraphState::edit`](crate::GraphState::edit)
/// leaves the graph as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    NoSuchNode(NodeId),
    NoSuchPort(PortId),
    NoSuchEdge(EdgeId),
    /// An id that is already in use.
    DuplicateId(u64),
    /// A connection needs one `Out` port and one `In` port.
    WrongDirection,
    /// These two ports are already connected.
    AlreadyConnected(EdgeId),
    /// The rules refused this pair of ports (a type mismatch, say).
    Denied,
    /// The connection would close a cycle and the rules forbid cycles.
    WouldCycle,
    /// The port already has as many edges as the rules allow.
    PortFull(PortId),
    /// A node or port can only be removed once its edges are gone.
    StillConnected,
    /// A port named for a node that does not hold it.
    PortNotOnNode(PortId, NodeId),
    /// Any other refusal, from app code inside an edit.
    Other(String),
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::NoSuchNode(id) => write!(f, "no node {id}"),
            GraphError::NoSuchPort(id) => write!(f, "no port {id}"),
            GraphError::NoSuchEdge(id) => write!(f, "no edge {id}"),
            GraphError::DuplicateId(id) => write!(f, "id {id} is already in use"),
            GraphError::WrongDirection => {
                write!(f, "a connection needs one output and one input")
            }
            GraphError::AlreadyConnected(id) => write!(f, "already connected by {id}"),
            GraphError::Denied => write!(f, "the rules refuse this connection"),
            GraphError::WouldCycle => write!(f, "the connection would close a cycle"),
            GraphError::PortFull(id) => write!(f, "port {id} takes no more edges"),
            GraphError::StillConnected => write!(f, "remove its edges first"),
            GraphError::PortNotOnNode(p, n) => write!(f, "port {p} is not on node {n}"),
            GraphError::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for GraphError {}

/// Nodes, ports and edges. Iteration is in id order, which is creation order.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Graph<N, P, E> {
    nodes: BTreeMap<NodeId, Node<N>>,
    ports: BTreeMap<PortId, Port<P>>,
    edges: BTreeMap<EdgeId, Edge<E>>,
    /// Edges touching each port, in id order. Derived: rebuilt on load.
    #[cfg_attr(feature = "serde", serde(skip))]
    port_edges: BTreeMap<PortId, Vec<EdgeId>>,
    /// The next id to hand out. Shared by all three kinds.
    next_id: u64,
}

impl<N, P, E> Default for Graph<N, P, E> {
    fn default() -> Self {
        Graph {
            nodes: BTreeMap::new(),
            ports: BTreeMap::new(),
            edges: BTreeMap::new(),
            port_edges: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl<N, P, E> Graph<N, P, E> {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- reading -------------------------------------------------------

    pub fn node(&self, id: NodeId) -> Option<&Node<N>> {
        self.nodes.get(&id)
    }

    pub fn port(&self, id: PortId) -> Option<&Port<P>> {
        self.ports.get(&id)
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge<E>> {
        self.edges.get(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node<N>> {
        self.nodes.values()
    }

    pub fn ports(&self) -> impl Iterator<Item = &Port<P>> {
        self.ports.values()
    }

    pub fn edges(&self) -> impl Iterator<Item = &Edge<E>> {
        self.edges.values()
    }

    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    pub fn edge_ids(&self) -> impl Iterator<Item = EdgeId> + '_ {
        self.edges.keys().copied()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The edges touching `port`, in id order.
    pub fn port_edges(&self, port: PortId) -> &[EdgeId] {
        self.port_edges.get(&port).map_or(&[], Vec::as_slice)
    }

    /// The edges touching any of `node`'s ports, each once, in id order.
    pub fn node_edges(&self, node: NodeId) -> Vec<EdgeId> {
        let Some(n) = self.nodes.get(&node) else {
            return Vec::new();
        };
        let mut out: Vec<EdgeId> = n
            .ports
            .iter()
            .flat_map(|p| self.port_edges(*p).iter().copied())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// `node`'s ports facing `dir`, in display order.
    pub fn node_ports(&self, node: NodeId, dir: PortDir) -> Vec<PortId> {
        self.nodes.get(&node).map_or_else(Vec::new, |n| {
            n.ports
                .iter()
                .copied()
                .filter(|p| self.ports.get(p).is_some_and(|p| p.dir == dir))
                .collect()
        })
    }

    /// The edge from `from` to `to`, if there is one.
    pub fn edge_between(&self, from: PortId, to: PortId) -> Option<EdgeId> {
        self.port_edges(from)
            .iter()
            .copied()
            .find(|e| self.edges.get(e).is_some_and(|e| e.to == to))
    }

    /// The node each end of `edge` sits on: `(from_node, to_node)`.
    pub fn edge_nodes(&self, edge: EdgeId) -> Option<(NodeId, NodeId)> {
        let e = self.edges.get(&edge)?;
        Some((self.ports.get(&e.from)?.node, self.ports.get(&e.to)?.node))
    }

    // ---- ids -----------------------------------------------------------

    pub(crate) fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Keep `next_id` past an id that came from elsewhere (a redo, a load).
    fn reserve(&mut self, id: u64) {
        if id >= self.next_id {
            self.next_id = id + 1;
        }
    }

    fn id_in_use(&self, id: u64) -> bool {
        self.nodes.contains_key(&NodeId(id))
            || self.ports.contains_key(&PortId(id))
            || self.edges.contains_key(&EdgeId(id))
    }

    /// Rebuild the derived indexes (after deserializing).
    pub fn reindex(&mut self) {
        self.port_edges.clear();
        for e in self.edges.values() {
            self.port_edges.entry(e.from).or_default().push(e.id);
            self.port_edges.entry(e.to).or_default().push(e.id);
        }
        let max = self
            .nodes
            .keys()
            .map(|k| k.0)
            .chain(self.ports.keys().map(|k| k.0))
            .chain(self.edges.keys().map(|k| k.0))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
    }

    // ---- raw mutation (steps only) --------------------------------------

    pub(crate) fn raw_insert_node(
        &mut self,
        node: Node<N>,
        ports: Vec<Port<P>>,
    ) -> Result<(), GraphError> {
        if self.id_in_use(node.id.0) {
            return Err(GraphError::DuplicateId(node.id.0));
        }
        for p in &ports {
            if self.id_in_use(p.id.0) {
                return Err(GraphError::DuplicateId(p.id.0));
            }
            if p.node != node.id {
                return Err(GraphError::PortNotOnNode(p.id, node.id));
            }
        }
        if ports.len() != node.ports.len() || ports.iter().any(|p| !node.ports.contains(&p.id)) {
            return Err(GraphError::Other(
                "a node's port list and its ports must match".into(),
            ));
        }
        self.reserve(node.id.0);
        for p in ports {
            self.reserve(p.id.0);
            self.ports.insert(p.id, p);
        }
        self.nodes.insert(node.id, node);
        Ok(())
    }

    pub(crate) fn raw_remove_node(
        &mut self,
        id: NodeId,
    ) -> Result<(Node<N>, Vec<Port<P>>), GraphError> {
        let node = self.nodes.get(&id).ok_or(GraphError::NoSuchNode(id))?;
        if node.ports.iter().any(|p| !self.port_edges(*p).is_empty()) {
            return Err(GraphError::StillConnected);
        }
        let node = self.nodes.remove(&id).expect("checked above");
        let ports = node
            .ports
            .iter()
            .filter_map(|p| {
                self.port_edges.remove(p);
                self.ports.remove(p)
            })
            .collect();
        Ok((node, ports))
    }

    pub(crate) fn raw_insert_port(
        &mut self,
        port: Port<P>,
        index: usize,
    ) -> Result<(), GraphError> {
        if self.id_in_use(port.id.0) {
            return Err(GraphError::DuplicateId(port.id.0));
        }
        let node = self
            .nodes
            .get_mut(&port.node)
            .ok_or(GraphError::NoSuchNode(port.node))?;
        let index = index.min(node.ports.len());
        node.ports.insert(index, port.id);
        self.reserve(port.id.0);
        self.ports.insert(port.id, port);
        Ok(())
    }

    /// Removes the port and returns it with the index it had on its node.
    pub(crate) fn raw_remove_port(&mut self, id: PortId) -> Result<(Port<P>, usize), GraphError> {
        let port = self.ports.get(&id).ok_or(GraphError::NoSuchPort(id))?;
        if !self.port_edges(id).is_empty() {
            return Err(GraphError::StillConnected);
        }
        let node = self
            .nodes
            .get_mut(&port.node)
            .ok_or(GraphError::NoSuchNode(port.node))?;
        let index = node
            .ports
            .iter()
            .position(|p| *p == id)
            .ok_or(GraphError::PortNotOnNode(id, port.node))?;
        node.ports.remove(index);
        self.port_edges.remove(&id);
        Ok((self.ports.remove(&id).expect("checked above"), index))
    }

    pub(crate) fn raw_set_pos(&mut self, id: NodeId, pos: Point) -> Result<Point, GraphError> {
        let node = self.nodes.get_mut(&id).ok_or(GraphError::NoSuchNode(id))?;
        Ok(std::mem::replace(&mut node.pos, pos))
    }

    pub(crate) fn raw_set_node_data(&mut self, id: NodeId, data: N) -> Result<N, GraphError> {
        let node = self.nodes.get_mut(&id).ok_or(GraphError::NoSuchNode(id))?;
        Ok(std::mem::replace(&mut node.data, data))
    }

    pub(crate) fn raw_set_port_data(&mut self, id: PortId, data: P) -> Result<P, GraphError> {
        let port = self.ports.get_mut(&id).ok_or(GraphError::NoSuchPort(id))?;
        Ok(std::mem::replace(&mut port.data, data))
    }

    pub(crate) fn raw_set_edge_data(&mut self, id: EdgeId, data: E) -> Result<E, GraphError> {
        let edge = self.edges.get_mut(&id).ok_or(GraphError::NoSuchEdge(id))?;
        Ok(std::mem::replace(&mut edge.data, data))
    }

    /// Inserts an edge as given. Direction and existence are checked; the
    /// app's [`Rules`](crate::Rules) are not (that is `Tx::connect`'s job).
    pub(crate) fn raw_insert_edge(&mut self, edge: Edge<E>) -> Result<(), GraphError> {
        if self.id_in_use(edge.id.0) {
            return Err(GraphError::DuplicateId(edge.id.0));
        }
        let from = self
            .ports
            .get(&edge.from)
            .ok_or(GraphError::NoSuchPort(edge.from))?;
        let to = self
            .ports
            .get(&edge.to)
            .ok_or(GraphError::NoSuchPort(edge.to))?;
        if from.dir != PortDir::Out || to.dir != PortDir::In {
            return Err(GraphError::WrongDirection);
        }
        if let Some(existing) = self.edge_between(edge.from, edge.to) {
            return Err(GraphError::AlreadyConnected(existing));
        }
        self.reserve(edge.id.0);
        insert_sorted(self.port_edges.entry(edge.from).or_default(), edge.id);
        insert_sorted(self.port_edges.entry(edge.to).or_default(), edge.id);
        self.edges.insert(edge.id, edge);
        Ok(())
    }

    pub(crate) fn raw_remove_edge(&mut self, id: EdgeId) -> Result<Edge<E>, GraphError> {
        let edge = self.edges.remove(&id).ok_or(GraphError::NoSuchEdge(id))?;
        for port in [edge.from, edge.to] {
            if let Some(list) = self.port_edges.get_mut(&port) {
                list.retain(|e| *e != id);
                if list.is_empty() {
                    self.port_edges.remove(&port);
                }
            }
        }
        Ok(edge)
    }
}

fn insert_sorted(list: &mut Vec<EdgeId>, id: EdgeId) {
    if let Err(at) = list.binary_search(&id) {
        list.insert(at, id);
    }
}
