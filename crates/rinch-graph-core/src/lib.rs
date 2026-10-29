//! The model behind a node-and-wire editor: a shader graph, a flowchart, an org
//! chart, an audio router.
//!
//! - [`Graph`] holds nodes, their ports and the edges between ports. Each item
//!   carries an app payload (`N`, `P`, `E`).
//! - [`Rules`] decides what may connect to what (type checks, fan-in, cycles).
//! - Every change is a [`Step`] with an exact inverse, and changes are grouped
//!   into transactions through [`GraphState::edit`], which is what undo and redo
//!   work on.
//!
//! Nothing here renders or depends on rinch. `rinch-graph-view` puts a canvas
//! on top. The design record is `docs/design/node-graph.md`.
//!
//! ```
//! use rinch_graph_core::{GraphState, Point, PortDir, AnyConnection};
//!
//! let mut state: GraphState<&str, &str, ()> = GraphState::new();
//! let (a, b) = state
//!     .edit(|tx| {
//!         let a = tx.add_node(Point::new(0.0, 0.0), "source", [(PortDir::Out, "out")])?;
//!         let b = tx.add_node(Point::new(200.0, 0.0), "sink", [(PortDir::In, "in")])?;
//!         Ok((a, b))
//!     })
//!     .unwrap();
//! let out = state.graph().node(a).unwrap().ports[0];
//! let input = state.graph().node(b).unwrap().ports[0];
//! state.edit(|tx| tx.connect(&AnyConnection, out, input, ())).unwrap();
//! assert_eq!(state.graph().edge_count(), 1);
//! state.undo();
//! assert_eq!(state.graph().edge_count(), 0);
//! ```

mod geom;
mod graph;
mod ids;
pub mod layout;
mod query;
mod rules;
mod state;
mod step;

pub use geom::{Point, Rect, Size};
pub use graph::{Edge, Graph, GraphError, Node, Port, PortDir};
pub use ids::{EdgeId, NodeId, PortId};
pub use query::CycleError;
pub use rules::{AnyConnection, ConnectPlan, Rules, check_connect};
pub use state::{GraphState, Selection, Tx};
pub use step::Step;
