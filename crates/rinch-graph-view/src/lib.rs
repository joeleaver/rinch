//! A canvas for node-and-wire editors on rinch: shader graphs, flowcharts, org
//! charts.
//!
//! - [`GraphView`] owns a [`GraphState`](rinch_graph_core::GraphState) and the
//!   app's [`Rules`](rinch_graph_core::Rules), applies undoable edits, and
//!   mirrors the model into per-node, per-port and per-edge signals, so moving
//!   one node updates that node and its wires and nothing else.
//! - [`graph_canvas`] builds the canvas: pan and zoom, dragging nodes (and the
//!   selection), drawing and re-routing wires under the rules, picking edges,
//!   marquee selection, keyboard (Delete, undo/redo, select all, arrows, zoom).
//! - The app renders each node's inside and places its ports with
//!   [`GraphView::port_dot`].
//!
//! The design record is `docs/design/node-graph.md`; the user guide is
//! `docs/src/guide/node-graphs.md`.

// `rsx!` expands to `::rinch::core::…` paths; point them at rinch-core here.
extern crate self as rinch;
#[doc(hidden)]
pub mod core {
    pub use rinch_core::*;
}

mod canvas;
mod geometry;
mod style;
mod view;

pub use canvas::{CanvasOptions, ConnectToEmpty, graph_canvas};
pub use geometry::{EdgeShape, Flow, Viewport, distance_to_edge, edge_path};
pub use rinch_graph_core as model;
pub use rinch_graph_core::{
    AnyConnection, ConnectPlan, Edge, EdgeId, Graph, GraphError, GraphState, Node, NodeId, Point,
    Port, PortDir, PortId, Rect, Rules, Selection, Size, Step, Tx,
};
pub use view::{Config, GraphView, PendingWire, PortHint};
