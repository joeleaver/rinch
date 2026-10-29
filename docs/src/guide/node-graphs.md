# Node Graphs

Rinch has a foundation for node-and-wire editors: shader and material graphs,
flowcharts, org charts, audio routing, state machines. You supply what is
inside each node and the rules for what may connect; the canvas does the rest:
pan and zoom, dragging nodes, drawing wires, selection, keyboard and undo.

Enable it with the `graph` feature:

```toml
rinch = { workspace = true, features = ["desktop", "components", "theme", "graph"] }
```

Everything is under `rinch::graph` (it is not in the prelude). The model on its
own, with no rinch dependency, is the `rinch-graph-core` crate
(`rinch::graph::model`). A complete example with a typed shader graph and an
org chart is `examples/node-graph-demo` (`cargo run -p node-graph-demo`).

## The model

A `Graph<N, P, E>` holds nodes, ports and edges, each carrying your data: `N`
for a node (what it does), `P` for a port (its label and type), `E` for an edge.
Ports face `PortDir::In` or `PortDir::Out`; an edge always runs from an output
to an input. Ids (`NodeId`, `PortId`, `EdgeId`) are integers and never reused,
so an id held across an undo still names the same item after the redo.

Changes go through `GraphState::edit`, which makes everything inside it one
undo step and rolls all of it back if any part fails:

```rust
use rinch::graph::{AnyConnection, GraphState, Point, PortDir};

let mut state: GraphState<&str, &str, ()> = GraphState::new();
let (a, b) = state.edit(|tx| {
    let a = tx.add_node(Point::new(0.0, 0.0), "source", [(PortDir::Out, "out")])?;
    let b = tx.add_node(Point::new(200.0, 0.0), "sink", [(PortDir::In, "in")])?;
    Ok((a, b))
})?;
let out = state.graph().node(a).unwrap().ports[0];
let input = state.graph().node(b).unwrap().ports[0];
state.edit(|tx| tx.connect(&AnyConnection, out, input, ()))?;
state.undo(); // the edge is gone
```

`Tx` has `add_node`, `add_port`, `remove_node` (takes its edges with it),
`remove_selected`, `move_nodes`/`move_by`, `set_node_data`/`update_node_data`,
`connect`, `disconnect`, and `apply` for any `Step`.

Queries for evaluating a graph: `topo_order()` (an evaluation order, or the
nodes on a cycle), `upstream`/`downstream`, `predecessors`/`successors`,
`port_edges`, `node_ports`.

## Rules

`Rules` decides what may connect. Every method has a default; implement only
what differs:

| Method | Default | Use it for |
|---|---|---|
| `can_connect(graph, from, to)` | `true` | type checks (`vec3` into `float`) |
| `max_inputs(port)` | `Some(1)` | inputs that take several wires |
| `max_outputs(port)` | `None` | outputs that feed one thing only |
| `replace_when_full()` | `true` | refuse instead of replacing a full input's wire |
| `allow_cycles()` | `false` | feedback loops (a self-loop is a cycle) |

```rust
struct Typed;
impl Rules<Op, Slot, ()> for Typed {
    fn can_connect(&self, _: &Graph<Op, Slot, ()>, from: &Port<Slot>, to: &Port<Slot>) -> bool {
        from.data.ty == to.data.ty
    }
}
```

`AnyConnection` is the all-defaults rule set. `check_connect(graph, rules, a, b)`
asks without changing anything; the canvas uses it to light up the ports a wire
may be dropped on.

## The canvas

`GraphView` owns the state and the rules. Create it in a component and pass it
to `graph_canvas` with a function that renders the inside of a node:

```rust
use rinch::graph::{CanvasOptions, GraphView, NodeId, PortDir, graph_canvas};

#[component]
fn editor() -> NodeHandle {
    let view = GraphView::new(load_state(), Typed);
    let canvas = graph_canvas(
        __scope,
        &view,
        move |__scope: &mut RenderScope, id: NodeId| node_body(__scope, view, id),
        CanvasOptions { controls: true, grid: true, fit_on_mount: true, ..Default::default() },
    );
    rsx! { div { style: "height: 100%;", {canvas} } }
}
```

`GraphView` is `Copy`, like `Signal`: closures capture it without cloning. It
belongs to the component that created it.

The node function runs once per node, when the node appears. Place each port's
dot with `view.port_dot(__scope, port)` wherever the body wants it; wires
attach to the dot's centre. A port with no dot attaches to the middle of the
node's side (left/right for `Flow::LeftToRight`, top/bottom for
`Flow::TopToBottom`).

```rust
fn node_body(__scope: &mut RenderScope, view: GraphView<Op, Slot, ()>, id: NodeId) -> NodeHandle {
    let root = rsx! { div { style: "min-width: 150px;",
        div { {move || view.node_data(id).map(|op| op.title()).unwrap_or_default()} }
    } };
    for p in view.node_ports(id, PortDir::In) {
        let dot = view.port_dot(__scope, p);
        dot.set_style("margin-left", "-6px"); // sit on the frame's edge
        root.append_child(&dot);
    }
    root
}
```

Reading in node bodies is fine-grained: `view.node_data(id)` re-runs only when
that node's data changes, `view.port_data(port)` only for that port. Moving one
node updates that node's frame and its wires, nothing else. `view.graph(|g| …)`
re-runs on any change; use it for a whole-graph result such as a shader
preview.

An element in a node body that has its own `onclick` (a button, a swatch)
takes the press, so it does not start a node drag.

### What the user can do

| Gesture | Effect |
|---|---|
| Press a node and drag | moves it (and the rest of the selection); one undo step |
| Click a node | selects it; Shift/Ctrl/Cmd+click toggles it |
| Drag from an output (or an empty input) | draws a wire; candidate ports light up; drop on one to connect |
| Drag from an input that has a wire | picks the wire up: drop it on another input to move it, or on empty canvas to delete it |
| Drop a wire on empty canvas | calls `CanvasOptions::on_connect_to_empty(port, point)` (offer a menu of nodes there) |
| Click near a wire | selects it |
| Drag the background | pans; a click without dragging clears the selection |
| Wheel or two-finger trackpad scroll | pans (Shift turns a vertical wheel sideways) |
| Ctrl/Cmd+wheel, trackpad pinch | zooms around the pointer |
| Shift+drag the background | box selection |
| Delete / Backspace | removes the selection |
| Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z, Ctrl/Cmd+Y | undo, redo |
| Ctrl/Cmd+A | selects everything |
| Arrow keys (Shift: ×10) | nudge the selection |
| `+`, `-`, `0` | zoom in, out, fit |
| Escape | cancels a drag, or clears the selection |

The canvas takes keys while it (or something in it without its own key
handling) has focus; pressing anywhere on it focuses it.

Zooming is by Ctrl/Cmd+wheel and pinch (around the pointer), the controls, the
keys and `view.zoom_by`/`zoom_around`/`fit_view`/`set_viewport`. The canvas
consumes every wheel over it (its `onwheel`), so the page around it neither
scrolls nor zooms while the pointer is on the canvas.

### Settings

`view.configure(|c| …)` changes `Config`: `flow` (`LeftToRight`,
`TopToBottom`), `edge_shape` (`Bezier`, `Straight`, `Step`), `snap` (grid size
in graph units, `0` for none), `min_zoom`/`max_zoom`, `port_snap_radius` (how
close a dropped wire must be to a port, in screen pixels).

### Styling

The canvas brings its own stylesheet; re-theme it with these custom properties:
`--rinch-graph-bg`, `--rinch-graph-grid`, `--rinch-graph-node-bg`,
`--rinch-graph-node-border`, `--rinch-graph-accent`, `--rinch-graph-accent-soft`,
`--rinch-graph-port`, `--rinch-graph-port-bg`, `--rinch-graph-ok`. Classes:
`.rinch-graph-node` (and `.selected`), `.rinch-graph-port` (with `in`/`out`,
and `source`/`candidate`/`target` while a wire is drawn),
`.rinch-graph-marquee`, `.rinch-graph-controls`.

## Layout

`rinch::graph::model::layout::tree` computes a top-down tree layout (each
parent centred over its children). It returns positions; apply them with
`tx.move_nodes`, which makes the layout one undo step:

```rust
let positions = view.with_state(|s| tree(
    s.graph(),
    |n| view.node_rect(n).map(|r| Size::new(r.width, r.height)).unwrap_or_default(),
    TreeSpacing::default(),
    Point::ZERO,
));
let _ = view.edit(|tx| tx.move_nodes(positions));
view.fit_view();
```

## Saving

With the `serde` feature on `rinch-graph-core`, `Graph` serializes. After
loading, call `graph.reindex()` and hand the graph to
`GraphState::from_graph`, then `view.replace_state(state)`.
