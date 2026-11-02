//! An org chart: one manager per person, laid out as a tree.

use rinch::graph::model::layout::{TreeSpacing, tree};
use rinch::graph::{
    AnyConnection, CanvasOptions, EdgeShape, Flow, GraphState, GraphView, NodeId, Point, PortDir,
    Selection, graph_canvas,
};
use rinch::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub name: String,
    pub title: String,
}

/// Every person has one input (their manager) and one output (their
/// reports). `AnyConnection` already means one wire per input (a new manager
/// replaces the old) and no cycles.
type View = GraphView<Person, (), ()>;

const NAMES: [&str; 12] = [
    "Ada",
    "Grace",
    "Linus",
    "Margaret",
    "Ken",
    "Barbara",
    "Dennis",
    "Frances",
    "Alan",
    "Radia",
    "Edsger",
    "Katherine",
];

fn person_ports() -> [(PortDir, ()); 2] {
    [(PortDir::In, ()), (PortDir::Out, ())]
}

fn sample() -> GraphState<Person, (), ()> {
    let mut s = GraphState::new();
    s.edit(|tx| {
        let p = |name: &str, title: &str| Person {
            name: name.into(),
            title: title.into(),
        };
        let ceo = tx.add_node(Point::new(300.0, 0.0), p("Ada", "CEO"), person_ports())?;
        let cto = tx.add_node(Point::new(80.0, 140.0), p("Grace", "CTO"), person_ports())?;
        let cfo = tx.add_node(
            Point::new(520.0, 140.0),
            p("Margaret", "CFO"),
            person_ports(),
        )?;
        let e1 = tx.add_node(
            Point::new(0.0, 280.0),
            p("Linus", "Engineer"),
            person_ports(),
        )?;
        let e2 = tx.add_node(
            Point::new(200.0, 280.0),
            p("Ken", "Engineer"),
            person_ports(),
        )?;
        let out = |tx: &rinch::graph::Tx<'_, Person, (), ()>, n: NodeId| {
            tx.graph().node(n).unwrap().ports[1]
        };
        let inp = |tx: &rinch::graph::Tx<'_, Person, (), ()>, n: NodeId| {
            tx.graph().node(n).unwrap().ports[0]
        };
        for (boss, report) in [(ceo, cto), (ceo, cfo), (cto, e1), (cto, e2)] {
            let (a, b) = (out(tx, boss), inp(tx, report));
            tx.connect(&AnyConnection, a, b, ())?;
        }
        Ok(())
    })
    .expect("the sample chart is valid");
    s.clear_history();
    s
}

fn auto_layout(view: &View) {
    let positions = view.with_state(|s| {
        tree(
            s.graph(),
            |n| {
                view.node_rect(n)
                    .map(|r| rinch::graph::Size::new(r.width, r.height))
                    .unwrap_or_default()
            },
            TreeSpacing {
                sibling: 24.0,
                level: 60.0,
            },
            Point::ZERO,
        )
    });
    let _ = view.edit(|tx| tx.move_nodes(positions));
    view.fit_view();
}

/// Add a report under each selected person (or a new top-level person).
fn add_report(view: &View) {
    let count = view.with_state(|s| s.graph().node_count());
    let name = NAMES[count % NAMES.len()].to_string();
    let managers: Vec<NodeId> = view.selection().nodes.into_iter().collect();
    let new = view.edit(|tx| {
        let mut added = Vec::new();
        let targets = if managers.is_empty() {
            vec![None]
        } else {
            managers.iter().copied().map(Some).collect()
        };
        for m in targets {
            let at = m
                .and_then(|m| tx.graph().node(m))
                .map_or(Point::new(0.0, 0.0), |n| n.pos.offset(0.0, 140.0));
            let person = Person {
                name: name.clone(),
                title: "New hire".into(),
            };
            let n = tx.add_node(at, person, person_ports())?;
            if let Some(m) = m {
                let a = tx.graph().node(m).unwrap().ports[1];
                let b = tx.graph().node(n).unwrap().ports[0];
                tx.connect(&AnyConnection, a, b, ())?;
            }
            added.push(n);
        }
        Ok(added)
    });
    if let Ok(added) = new {
        view.set_selection(Selection::nodes(added));
    }
}

pub fn demo(__scope: &mut RenderScope) -> NodeHandle {
    let view: View = GraphView::new(sample(), AnyConnection);
    view.configure(|c| {
        c.flow = Flow::TopToBottom;
        c.edge_shape = EdgeShape::Step;
        c.snap = 10.0;
    });
    let canvas = graph_canvas(
        __scope,
        &view,
        move |__scope: &mut RenderScope, id: NodeId| person_card(__scope, view, id),
        CanvasOptions {
            controls: true,
            grid: true,
            fit_on_mount: true,
            // Dropping a wire from someone onto empty canvas hires a report there.
            on_connect_to_empty: Some(std::rc::Rc::new(move |port, at| {
                let from_out =
                    view.with_state(|s| s.graph().port(port).map(|p| p.dir == PortDir::Out));
                if from_out != Some(true) {
                    return;
                }
                let _ = view.edit(|tx| {
                    let person = Person {
                        name: "New hire".into(),
                        title: "Report".into(),
                    };
                    let n = tx.add_node(at.offset(-60.0, 0.0), person, person_ports())?;
                    let b = tx.graph().node(n).unwrap().ports[0];
                    tx.connect(&AnyConnection, port, b, ())
                });
            })),
            ..Default::default()
        },
    );
    rsx! {
        div { style: "position: absolute; inset: 0; display: flex; flex-direction: column;",
            div { style: "display: flex; gap: 6px; padding: 6px 12px; align-items: center; border-bottom: 1px solid #eef0f2;",
                Button { size: "xs", variant: "default", onclick: move || add_report(&view), "+ Add report" }
                Button { size: "xs", variant: "default", onclick: move || auto_layout(&view), "Auto layout" }
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
            }
        }
    }
}

fn person_card(__scope: &mut RenderScope, view: View, id: NodeId) -> NodeHandle {
    let ports = view
        .with_state(|s| s.graph().node(id).map(|n| n.ports.clone()))
        .unwrap_or_default();
    let card = rsx! {
        div { style: "position: relative; width: 140px; padding: 10px 12px; text-align: center;",
            div { style: "font-weight: 600;", {move || view.node_data(id).map(|p| p.name).unwrap_or_default()} }
            div { style: "color: #868e96; font-size: 12px;", {move || view.node_data(id).map(|p| p.title).unwrap_or_default()} }
        }
    };
    // The manager wire enters at the top centre, reports leave at the bottom.
    for (port, edge) in ports.iter().zip(["top", "bottom"]) {
        let dot = view.port_dot(__scope, *port);
        dot.set_styles(&[
            ("position", "absolute"),
            ("left", "50%"),
            ("margin-left", "-6px"),
            (edge, "-6px"),
        ]);
        card.append_child(&dot);
    }
    card
}
