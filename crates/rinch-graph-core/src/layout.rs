//! Automatic layout. Each function returns positions; applying them is an
//! ordinary [`Tx::move_nodes`](crate::Tx::move_nodes), so a layout is one undo
//! step like any other move.

use std::collections::BTreeSet;

use crate::geom::{Point, Size};
use crate::graph::Graph;
use crate::ids::NodeId;

/// Spacing for [`tree`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeSpacing {
    /// Space between siblings (and between neighbouring subtrees).
    pub sibling: f32,
    /// Space between a parent and its children.
    pub level: f32,
}

impl Default for TreeSpacing {
    fn default() -> Self {
        TreeSpacing {
            sibling: 24.0,
            level: 64.0,
        }
    }
}

/// A top-down tree layout: roots (nodes nothing feeds) side by side from
/// `origin`, each node centred over its children, every level one row.
///
/// Edges define the tree: a node's children are the nodes its outputs feed,
/// in id order. A node reachable from several parents is placed under the
/// first one reached; a cycle is broken where it is met. `size` gives each
/// node's measured size (zero is fine for an unmeasured node).
pub fn tree<N, P, E>(
    graph: &Graph<N, P, E>,
    size: impl Fn(NodeId) -> Size,
    spacing: TreeSpacing,
    origin: Point,
) -> Vec<(NodeId, Point)> {
    // Assign every node to exactly one parent, breadth first from the roots,
    // so a node's depth is its shortest distance from a root.
    let mut roots: Vec<NodeId> = graph
        .node_ids()
        .filter(|n| graph.predecessors(*n).is_empty())
        .collect();
    let mut placed: BTreeSet<NodeId> = BTreeSet::new();
    let mut children: std::collections::BTreeMap<NodeId, Vec<NodeId>> = Default::default();
    let mut order: Vec<NodeId> = Vec::new();
    let mut claim = |roots: &[NodeId], placed: &mut BTreeSet<NodeId>, order: &mut Vec<NodeId>| {
        let mut frontier: Vec<NodeId> = roots
            .iter()
            .copied()
            .filter(|r| placed.insert(*r))
            .collect();
        while !frontier.is_empty() {
            order.extend(frontier.iter().copied());
            let mut next = Vec::new();
            for n in frontier {
                for c in graph.successors(n) {
                    if placed.insert(c) {
                        children.entry(n).or_default().push(c);
                        next.push(c);
                    }
                }
            }
            frontier = next;
        }
    };
    claim(&roots, &mut placed, &mut order);
    // Nodes only on cycles have no root: start a tree at the first of them.
    while let Some(n) = graph.node_ids().find(|n| !placed.contains(n)) {
        roots.push(n);
        claim(&[n], &mut placed, &mut order);
    }

    // Each subtree's width, children before parents.
    let mut width: std::collections::BTreeMap<NodeId, f32> = Default::default();
    for n in order.iter().rev() {
        let own = size(*n).width;
        let kids = children.get(n).map_or(0.0, |c| {
            c.iter().map(|k| width[k]).sum::<f32>() + spacing.sibling * (c.len() as f32 - 1.0)
        });
        width.insert(*n, own.max(kids));
    }

    // Row heights: the tallest node on each level.
    let mut depth: std::collections::BTreeMap<NodeId, usize> = Default::default();
    for r in &roots {
        depth.insert(*r, 0);
    }
    for n in &order {
        let d = depth[n];
        for c in children.get(n).into_iter().flatten() {
            depth.insert(*c, d + 1);
        }
    }
    let levels = depth.values().copied().max().map_or(0, |d| d + 1);
    let mut row_height = vec![0.0f32; levels];
    for (n, d) in &depth {
        row_height[*d] = row_height[*d].max(size(*n).height);
    }
    let mut row_y = Vec::with_capacity(levels);
    let mut y = origin.y;
    for h in &row_height {
        row_y.push(y);
        y += h + spacing.level;
    }

    // Place top-down: each subtree gets a span; its root is centred in it.
    let mut out = Vec::with_capacity(order.len());
    let mut stack: Vec<(NodeId, f32)> = Vec::new();
    let mut x = origin.x;
    for r in &roots {
        stack.push((*r, x));
        x += width[r] + spacing.sibling;
    }
    while let Some((n, left)) = stack.pop() {
        let w = width[&n];
        let own = size(n).width;
        out.push((n, Point::new(left + (w - own) / 2.0, row_y[depth[&n]])));
        if let Some(kids) = children.get(&n) {
            let total: f32 = kids.iter().map(|k| width[k]).sum::<f32>()
                + spacing.sibling * (kids.len() as f32 - 1.0);
            let mut cx = left + (w - total) / 2.0;
            for k in kids {
                stack.push((*k, cx));
                cx += width[k] + spacing.sibling;
            }
        }
    }
    out.sort_by_key(|(n, _)| *n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnyConnection, GraphState, PortDir};

    #[test]
    fn a_parent_is_centred_over_its_children() {
        let mut s: GraphState<(), (), ()> = GraphState::new();
        let (boss, a, b) = s
            .edit(|tx| {
                let boss = tx.add_node(Point::ZERO, (), [(PortDir::Out, ())])?;
                let a = tx.add_node(Point::ZERO, (), [(PortDir::In, ())])?;
                let b = tx.add_node(Point::ZERO, (), [(PortDir::In, ())])?;
                Ok((boss, a, b))
            })
            .unwrap();
        let port = |n: NodeId, s: &GraphState<(), (), ()>| s.graph().node(n).unwrap().ports[0];
        let (o, ia, ib) = (port(boss, &s), port(a, &s), port(b, &s));
        s.edit(|tx| {
            tx.connect(&AnyConnection, o, ia, ())?;
            tx.connect(&AnyConnection, o, ib, ())
        })
        .unwrap();
        let pos = tree(
            s.graph(),
            |_| Size::new(100.0, 40.0),
            TreeSpacing {
                sibling: 20.0,
                level: 60.0,
            },
            Point::ZERO,
        );
        let at = |n| pos.iter().find(|(m, _)| *m == n).unwrap().1;
        assert_eq!(at(a), Point::new(0.0, 100.0));
        assert_eq!(at(b), Point::new(120.0, 100.0));
        assert_eq!(
            at(boss),
            Point::new(60.0, 0.0),
            "centred over the 220-wide row"
        );
    }

    #[test]
    fn every_node_is_placed_once_even_on_a_cycle() {
        let mut s: GraphState<(), (), ()> = GraphState::new();
        struct Cyclic;
        impl crate::Rules<(), (), ()> for Cyclic {
            fn allow_cycles(&self) -> bool {
                true
            }
        }
        let ids = s
            .edit(|tx| {
                (0..3)
                    .map(|_| tx.add_node(Point::ZERO, (), [(PortDir::In, ()), (PortDir::Out, ())]))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        let p = |n: NodeId, i: usize| s.graph().node(n).unwrap().ports[i];
        let links = [
            (p(ids[0], 1), p(ids[1], 0)),
            (p(ids[1], 1), p(ids[2], 0)),
            (p(ids[2], 1), p(ids[0], 0)),
        ];
        s.edit(|tx| {
            for (a, b) in links {
                tx.connect(&Cyclic, a, b, ())?;
            }
            Ok(())
        })
        .unwrap();
        let pos = tree(
            s.graph(),
            |_| Size::new(10.0, 10.0),
            TreeSpacing::default(),
            Point::ZERO,
        );
        assert_eq!(pos.len(), 3);
    }
}
