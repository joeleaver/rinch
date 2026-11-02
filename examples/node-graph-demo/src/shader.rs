//! A typed shader graph: floats and colours, evaluated live.

use std::collections::HashMap;

use rinch::graph::{
    CanvasOptions, Graph, GraphState, GraphView, NodeId, Point, Port, PortDir, PortId, Rules,
    graph_canvas,
};
use rinch::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    Float,
    Color,
}

/// What a node does. The node's data in the graph.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Number(f32),
    Color([f32; 3]),
    Add,
    Multiply,
    Mix,
    Output,
}

/// A port's data: its label and value type.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub name: &'static str,
    pub ty: Ty,
}

type View = GraphView<Op, Slot, ()>;

/// A float may feed a colour input (it becomes grey); otherwise types match.
struct Typed;
impl Rules<Op, Slot, ()> for Typed {
    fn can_connect(&self, _: &Graph<Op, Slot, ()>, from: &Port<Slot>, to: &Port<Slot>) -> bool {
        from.data.ty == to.data.ty || (from.data.ty == Ty::Float && to.data.ty == Ty::Color)
    }
}

const PALETTE: [[f32; 3]; 5] = [
    [0.95, 0.35, 0.3],
    [0.25, 0.6, 0.95],
    [0.35, 0.8, 0.45],
    [0.98, 0.78, 0.25],
    [0.6, 0.4, 0.85],
];

impl Op {
    fn title(&self) -> &'static str {
        match self {
            Op::Number(_) => "Number",
            Op::Color(_) => "Colour",
            Op::Add => "Add",
            Op::Multiply => "Multiply",
            Op::Mix => "Mix",
            Op::Output => "Output",
        }
    }

    fn ports(&self) -> Vec<(PortDir, Slot)> {
        let s = |name, ty| Slot { name, ty };
        match self {
            Op::Number(_) => vec![(PortDir::Out, s("value", Ty::Float))],
            Op::Color(_) => vec![(PortDir::Out, s("colour", Ty::Color))],
            Op::Add | Op::Multiply => vec![
                (PortDir::In, s("a", Ty::Color)),
                (PortDir::In, s("b", Ty::Color)),
                (PortDir::Out, s("result", Ty::Color)),
            ],
            Op::Mix => vec![
                (PortDir::In, s("a", Ty::Color)),
                (PortDir::In, s("b", Ty::Color)),
                (PortDir::In, s("t", Ty::Float)),
                (PortDir::Out, s("result", Ty::Color)),
            ],
            Op::Output => vec![(PortDir::In, s("colour", Ty::Color))],
        }
    }

    fn menu() -> Vec<Op> {
        Self::MENU.to_vec()
    }

    const MENU: [Op; 6] = [
        Op::Number(0.5),
        Op::Color([0.25, 0.6, 0.95]),
        Op::Add,
        Op::Multiply,
        Op::Mix,
        Op::Output,
    ];
}

/// Evaluate every output port, in dependency order. Unconnected inputs are
/// black (colour) or zero (float).
fn evaluate(g: &Graph<Op, Slot, ()>) -> HashMap<PortId, [f32; 3]> {
    let mut out: HashMap<PortId, [f32; 3]> = HashMap::new();
    let Ok(order) = g.topo_order() else {
        return out;
    };
    for n in order {
        let node = g.node(n).unwrap();
        let input = |i: usize| -> [f32; 3] {
            let ins = g.node_ports(n, PortDir::In);
            ins.get(i)
                .and_then(|p| g.port_edges(*p).first())
                .and_then(|e| out.get(&g.edge(*e)?.from).copied())
                .unwrap_or([0.0; 3])
        };
        let value = match &node.data {
            Op::Number(v) => [*v; 3],
            Op::Color(c) => *c,
            Op::Add => zip(input(0), input(1), |a, b| (a + b).min(1.0)),
            Op::Multiply => zip(input(0), input(1), |a, b| a * b),
            Op::Mix => {
                let t = input(2)[0];
                zip(input(0), input(1), |a, b| a + (b - a) * t)
            }
            Op::Output => continue,
        };
        for p in g.node_ports(n, PortDir::Out) {
            out.insert(p, value);
        }
    }
    out
}

fn zip(a: [f32; 3], b: [f32; 3], f: impl Fn(f32, f32) -> f32) -> [f32; 3] {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

fn css(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgb({}, {}, {})", b(c[0]), b(c[1]), b(c[2]))
}

/// The colour reaching `node`'s first input.
fn input_color(g: &Graph<Op, Slot, ()>, node: NodeId) -> [f32; 3] {
    let values = evaluate(g);
    g.node_ports(node, PortDir::In)
        .first()
        .and_then(|p| g.port_edges(*p).first())
        .and_then(|e| values.get(&g.edge(*e)?.from).copied())
        .unwrap_or([0.0; 3])
}

fn add_node(view: &View, op: Op, at: Point) -> Option<NodeId> {
    view.edit(|tx| tx.add_node(at, op.clone(), op.ports())).ok()
}

/// The starting graph: two colours mixed by a number, into the output.
fn sample() -> GraphState<Op, Slot, ()> {
    let mut s = GraphState::new();
    s.edit(|tx| {
        let red = tx.add_node(
            Point::new(40.0, 40.0),
            Op::Color(PALETTE[0]),
            Op::Color(PALETTE[0]).ports(),
        )?;
        let blue = tx.add_node(
            Point::new(40.0, 170.0),
            Op::Color(PALETTE[1]),
            Op::Color(PALETTE[1]).ports(),
        )?;
        let t = tx.add_node(
            Point::new(40.0, 300.0),
            Op::Number(0.3),
            Op::Number(0.3).ports(),
        )?;
        let mix = tx.add_node(Point::new(320.0, 140.0), Op::Mix, Op::Mix.ports())?;
        let out = tx.add_node(Point::new(580.0, 160.0), Op::Output, Op::Output.ports())?;
        let port = |tx: &rinch::graph::Tx<'_, Op, Slot, ()>, n: NodeId, i: usize| {
            tx.graph().node(n).unwrap().ports[i]
        };
        let links = [
            (port(tx, red, 0), port(tx, mix, 0)),
            (port(tx, blue, 0), port(tx, mix, 1)),
            (port(tx, t, 0), port(tx, mix, 2)),
            (port(tx, mix, 3), port(tx, out, 0)),
        ];
        for (a, b) in links {
            tx.connect(&Typed, a, b, ())?;
        }
        Ok(())
    })
    .expect("the sample graph is valid");
    s.clear_history();
    s
}

pub fn demo(__scope: &mut RenderScope) -> NodeHandle {
    let view: View = GraphView::new(sample(), Typed);
    // A wire dropped on empty canvas opens this menu at the drop point.
    let menu: Signal<Option<(PortId, Point)>> = Signal::new(None);
    let render = move |__scope: &mut RenderScope, id: NodeId| node_body(__scope, view, id);
    let canvas = graph_canvas(
        __scope,
        &view,
        render,
        CanvasOptions {
            controls: true,
            grid: true,
            fit_on_mount: true,
            on_connect_to_empty: Some(std::rc::Rc::new(move |port, at| menu.set(Some((port, at))))),
            ..Default::default()
        },
    );
    rsx! {
        div { style: "position: absolute; inset: 0; display: flex; flex-direction: column;",
            div { style: "display: flex; gap: 6px; padding: 6px 12px; align-items: center; border-bottom: 1px solid #eef0f2;",
                for op in Op::menu() {
                    Button { key: Op::title(op), size: "xs", variant: "default",
                        onclick: {
                                                        let op = Op::clone(&op);
                            move || {
                                let vp = view.viewport();
                                let at = vp.to_graph(Point::new(200.0, 120.0));
                                if let Some(n) = add_node(&view, op.clone(), at) {
                                    view.set_selection(rinch::graph::Selection::nodes([n]));
                                }
                            }
                        },
                        {format!("+ {}", Op::title(&op))}
                    }
                }
                div { style: "flex: 1;" }
                Button { size: "xs", variant: "light", disabled: {move || !view.can_undo()},
                    onclick: move || { view.undo(); }, "Undo" }
                Button { size: "xs", variant: "light", disabled: {move || !view.can_redo()},
                    onclick: move || { view.redo(); }, "Redo" }
                Button { size: "xs", variant: "light", color: "red",
                    onclick: move || view.delete_selected(), "Delete" }
            }
            div { style: "flex: 1; min-height: 0; position: relative;",
                {canvas}
                if let Some((port, at)) = menu.get() {
                    div {
                        style: {
                                                        move || {
                                let p = view.viewport().to_canvas(at);
                                format!("position: absolute; left: {}px; top: {}px; background: white; border: 1px solid #ced4da; border-radius: 6px; padding: 4px; box-shadow: 0 4px 12px rgba(0,0,0,.12); display: flex; flex-direction: column; min-width: 120px;", p.x, p.y)
                            }
                        },
                        for op in Op::menu() {
                            div { key: Op::title(op), class: "menu-item",
                                style: "padding: 4px 8px; border-radius: 4px; cursor: pointer;",
                                onclick: {
                                                                        let op = Op::clone(&op);
                                    move || {
                                        menu.set(None);
                                        let Some(n) = add_node(&view, op.clone(), at) else { return };
                                        // Connect the first port the rules allow.
                                        let target = view.with_state(|s| {
                                            s.graph().node(n).map(|node| node.ports.clone())
                                        }).unwrap_or_default().into_iter().find(|p| view.check_connect(port, *p).is_ok());
                                        if let Some(t) = target {
                                            let _ = view.connect(port, t);
                                        }
                                    }
                                },
                                {Op::title(&op)}
                            }
                        }
                        div { style: "padding: 4px 8px; color: #868e96; cursor: pointer;",
                            onclick: move || menu.set(None), "Cancel" }
                    }
                }
            }
        }
    }
}

fn node_body(__scope: &mut RenderScope, view: View, id: NodeId) -> NodeHandle {
    let Some(op) = view.with_state(|s| s.graph().node(id).map(|n| n.data.clone())) else {
        return __scope.create_element("div");
    };
    let inputs = view.node_ports(id, PortDir::In);
    let outputs = view.node_ports(id, PortDir::Out);
    let label = |p: PortId| {
        view.with_state(|s| s.graph().port(p).map(|p| p.data.name))
            .unwrap_or("")
    };

    let root = __scope.create_element("div");
    root.set_attribute("style", "min-width: 150px; padding-bottom: 6px;");
    let header = rsx! {
        div { style: "padding: 6px 10px; font-weight: 600; border-bottom: 1px solid #eef0f2; margin-bottom: 4px;",
            {op.title()}
        }
    };
    root.append_child(&header);

    for p in inputs {
        let row =
            rsx! { div { style: "display: flex; align-items: center; gap: 6px; height: 22px;" } };
        let dot = view.port_dot(__scope, p);
        dot.set_style("margin-left", "-6px");
        row.append_child(&dot);
        row.append_child(&rsx! { span { {label(p)} } });
        root.append_child(&row);
    }

    // Per-kind controls.
    let controls = match op {
        Op::Number(_) => {
            let step = move |v: &View, d: f32| {
                let _ = v.edit(|tx| {
                    tx.update_node_data(id, |op| {
                        if let Op::Number(x) = op {
                            *x = ((*x + d) * 10.0).round().clamp(0.0, 10.0) / 10.0;
                        }
                    })
                });
            };
            rsx! {
                div { style: "display: flex; align-items: center; gap: 6px; padding: 2px 10px;",
                    button { onclick: move || step(&view, -0.1), "\u{2212}" }
                    span { style: "min-width: 28px; text-align: center;",
                        {move || match view.node_data(id) { Some(Op::Number(x)) => format!("{x:.1}"), _ => String::new() }}
                    }
                    button { onclick: move || step(&view, 0.1), "+" }
                }
            }
        }
        Op::Color(_) => {
            rsx! {
                div { style: "padding: 2px 10px;",
                    div {
                        style: {move || match view.node_data(id) {
                            Some(Op::Color(c)) => format!("width: 100%; height: 22px; border-radius: 4px; background-color: {};", css(c)),
                            _ => String::new(),
                        }},
                        title: "Click for the next colour",
                        onclick: move || {
                            let _ = view.edit(|tx| tx.update_node_data(id, |op| {
                                if let Op::Color(c) = op {
                                    let i = PALETTE.iter().position(|p| p == c).map_or(0, |i| (i + 1) % PALETTE.len());
                                    *c = PALETTE[i];
                                }
                            }));
                        },
                    }
                }
            }
        }
        Op::Output => {
            rsx! {
                div { style: "padding: 2px 10px;",
                    div {
                        style: {move || {
                            let c = view.graph(|g| input_color(g, id));
                            format!("width: 120px; height: 80px; border-radius: 6px; border: 1px solid #dee2e6; background-color: {};", css(c))
                        }},
                    }
                }
            }
        }
        _ => __scope.create_element("div"),
    };
    root.append_child(&controls);

    for p in outputs {
        let row = rsx! { div { style: "display: flex; align-items: center; justify-content: flex-end; gap: 6px; height: 22px;" } };
        row.append_child(&rsx! { span { {label(p)} } });
        let dot = view.port_dot(__scope, p);
        dot.set_style("margin-right", "-6px");
        row.append_child(&dot);
        root.append_child(&row);
    }
    root
}
