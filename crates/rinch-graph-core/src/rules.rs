//! What may connect to what.

use crate::graph::{Graph, GraphError, Port, PortDir};
use crate::ids::{EdgeId, PortId};

/// The app's connection policy. Every method has a default, so a graph that
/// accepts any output-to-input connection implements nothing
/// ([`AnyConnection`]).
///
/// [`check_connect`] consults it while a wire is being dragged (for every
/// candidate port under the pointer) and again when the wire is dropped, so
/// `can_connect` must be cheap.
pub trait Rules<N, P, E> {
    /// May this output feed this input? The place for a type check
    /// (`vec3` into `float`, say). Direction, duplicates, capacity and cycles
    /// are checked separately.
    fn can_connect(&self, graph: &Graph<N, P, E>, from: &Port<P>, to: &Port<P>) -> bool {
        let _ = (graph, from, to);
        true
    }

    /// How many edges may end at an input port. `None` is unlimited.
    /// Default: one.
    fn max_inputs(&self, port: &Port<P>) -> Option<usize> {
        let _ = port;
        Some(1)
    }

    /// How many edges may leave an output port. `None` is unlimited.
    /// Default: unlimited.
    fn max_outputs(&self, port: &Port<P>) -> Option<usize> {
        let _ = port;
        None
    }

    /// Dropping a wire onto a full single-edge input replaces the edge already
    /// there (the usual node-editor behaviour). `false` refuses the
    /// connection with [`GraphError::PortFull`] instead. Default: `true`.
    fn replace_when_full(&self) -> bool {
        true
    }

    /// Whether the graph may contain cycles. A self-loop (a node feeding
    /// itself) is a cycle. Default: `false`.
    fn allow_cycles(&self) -> bool {
        false
    }
}

/// Rules that accept every output-to-input connection: one edge per input
/// (replacing), unlimited fan-out, no cycles.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnyConnection;

impl<N, P, E> Rules<N, P, E> for AnyConnection {}

/// What [`check_connect`] decided: the connection in its proper direction and
/// the edges that would be removed to make room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectPlan {
    /// The `Out` port.
    pub from: PortId,
    /// The `In` port.
    pub to: PortId,
    /// Edges removed first (a full input being replaced).
    pub replaces: Vec<EdgeId>,
}

/// Decide whether `a` and `b` may be connected, in either order (a wire can be
/// dragged from an input back to an output). Pure: changes nothing.
pub fn check_connect<N, P, E>(
    graph: &Graph<N, P, E>,
    rules: &(impl Rules<N, P, E> + ?Sized),
    a: PortId,
    b: PortId,
) -> Result<ConnectPlan, GraphError> {
    let pa = graph.port(a).ok_or(GraphError::NoSuchPort(a))?;
    let pb = graph.port(b).ok_or(GraphError::NoSuchPort(b))?;
    let (from, to) = match (pa.dir, pb.dir) {
        (PortDir::Out, PortDir::In) => (pa, pb),
        (PortDir::In, PortDir::Out) => (pb, pa),
        _ => return Err(GraphError::WrongDirection),
    };
    if let Some(existing) = graph.edge_between(from.id, to.id) {
        return Err(GraphError::AlreadyConnected(existing));
    }
    if !rules.allow_cycles() && graph.reaches(to.node, from.node) {
        return Err(GraphError::WouldCycle);
    }
    if !rules.can_connect(graph, from, to) {
        return Err(GraphError::Denied);
    }
    if let Some(max) = rules.max_outputs(from)
        && graph.port_edges(from.id).len() >= max
    {
        return Err(GraphError::PortFull(from.id));
    }
    let mut replaces = Vec::new();
    if let Some(max) = rules.max_inputs(to) {
        let existing = graph.port_edges(to.id);
        if existing.len() >= max {
            if max == 0 || !rules.replace_when_full() {
                return Err(GraphError::PortFull(to.id));
            }
            // Free exactly enough room: the oldest edges go first.
            let excess = existing.len() + 1 - max;
            replaces.extend(existing.iter().take(excess).copied());
        }
    }
    Ok(ConnectPlan {
        from: from.id,
        to: to.id,
        replaces,
    })
}
