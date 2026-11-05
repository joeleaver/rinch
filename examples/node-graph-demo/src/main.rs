//! The node-graph canvas's two first consumers, one per tab:
//!
//! - **Shader graph**: typed ports (a float feeds a colour input, not the other
//!   way), one wire per input, live evaluation into an output swatch, a menu
//!   of nodes when a wire is dropped on empty canvas.
//! - **Org chart**: top-to-bottom flow, one manager per person, "add report",
//!   and an automatic tree layout that undoes in one step.

mod org;
mod shader;

use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let tab = Signal::new(0u8);
    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100vh; font-family: sans-serif; font-size: 13px;",
            div { style: "display: flex; gap: 8px; padding: 8px 12px; border-bottom: 1px solid #dee2e6; align-items: center;",
                b { style: "margin-right: 12px;", "Node graphs" }
                Button { variant: {|| if tab.get() == 0 { "filled".to_string() } else { "light".to_string() }}, size: "xs",
                    onclick: move || tab.set(0), "Shader graph" }
                Button { variant: {|| if tab.get() == 1 { "filled".to_string() } else { "light".to_string() }}, size: "xs",
                    onclick: move || tab.set(1), "Org chart" }
            }
            div { style: "flex: 1; min-height: 0; position: relative;",
                if tab.get() == 0 {
                    ShaderDemo {}
                } else {
                    OrgDemo {}
                }
            }
        }
    }
}

#[component]
#[allow(non_snake_case)]
fn ShaderDemo() -> NodeHandle {
    shader::demo(__scope)
}

#[component]
#[allow(non_snake_case)]
fn OrgDemo() -> NodeHandle {
    org::demo(__scope)
}

fn main() {
    App::new(app).title("Node graphs").size(1200, 800).run();
}
