//! Graph queries every evaluator needs: reachability and evaluation order.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::graph::{Graph, PortDir};
use crate::ids::NodeId;

/// The graph has a cycle, so it has no evaluation order. Holds the nodes
/// that could not be ordered (every node on a cycle, and everything
/// downstream of one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleError {
    pub unordered: Vec<NodeId>,
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the graph has a cycle through {:?}", self.unordered)
    }
}

impl std::error::Error for CycleError {}

impl<N, P, E> Graph<N, P, E> {
    /// The nodes fed directly by `node`'s outputs, each once, in id order.
    pub fn successors(&self, node: NodeId) -> Vec<NodeId> {
        self.neighbours(node, PortDir::Out)
    }

    /// The nodes feeding `node`'s inputs directly, each once, in id order.
    pub fn predecessors(&self, node: NodeId) -> Vec<NodeId> {
        self.neighbours(node, PortDir::In)
    }

    fn neighbours(&self, node: NodeId, dir: PortDir) -> Vec<NodeId> {
        let mut out = BTreeSet::new();
        for port in self.node_ports(node, dir) {
            for e in self.port_edges(port) {
                let Some(edge) = self.edge(*e) else { continue };
                let other = if dir == PortDir::Out {
                    edge.to
                } else {
                    edge.from
                };
                if let Some(p) = self.port(other) {
                    out.insert(p.node);
                }
            }
        }
        out.into_iter().collect()
    }

    /// Whether `to` can be reached from `from` along edges (a node reaches
    /// itself).
    pub fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }
        let mut seen = BTreeSet::from([from]);
        let mut queue = VecDeque::from([from]);
        while let Some(n) = queue.pop_front() {
            for next in self.successors(n) {
                if next == to {
                    return true;
                }
                if seen.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        false
    }

    /// Every node `node` depends on, directly or not (not `node` itself).
    pub fn upstream(&self, node: NodeId) -> BTreeSet<NodeId> {
        self.closure(node, Self::predecessors)
    }

    /// Every node that depends on `node`, directly or not (not `node` itself).
    pub fn downstream(&self, node: NodeId) -> BTreeSet<NodeId> {
        self.closure(node, Self::successors)
    }

    fn closure(&self, node: NodeId, step: fn(&Self, NodeId) -> Vec<NodeId>) -> BTreeSet<NodeId> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([node]);
        while let Some(n) = queue.pop_front() {
            for next in step(self, n) {
                if next != node && seen.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        seen
    }

    /// The nodes in an order where every node comes after all the nodes that
    /// feed it (Kahn's algorithm; ties broken by id, so the order is stable).
    pub fn topo_order(&self) -> Result<Vec<NodeId>, CycleError> {
        let mut indegree: BTreeMap<NodeId, usize> = self
            .node_ids()
            .map(|n| (n, self.predecessors(n).len()))
            .collect();
        let mut ready: BTreeSet<NodeId> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(n, _)| *n)
            .collect();
        let mut order = Vec::with_capacity(indegree.len());
        while let Some(n) = ready.pop_first() {
            order.push(n);
            for next in self.successors(n) {
                let d = indegree.get_mut(&next).expect("every node is counted");
                *d -= 1;
                if *d == 0 {
                    ready.insert(next);
                }
            }
        }
        if order.len() == indegree.len() {
            return Ok(order);
        }
        let ordered: BTreeSet<NodeId> = order.into_iter().collect();
        indegree.retain(|n, _| !ordered.contains(n));
        Err(CycleError {
            unordered: indegree.into_keys().collect(),
        })
    }
}
