//! Integration compile tests for the RSX macro.
//!
//! These tests verify that the rsx! macro correctly generates compilable code
//! for all supported syntax patterns. If this file compiles, the tests pass.
//!
//! NOTE: This test requires the full `rinch` crate as a dev-dependency, which
//! pulls in desktop dependencies (winit, wgpu, vello). If these tests fail to
//! compile due to windowing/GPU dependencies in headless or CI environments,
//! the parser unit tests in `node.rs` and `element.rs` are the primary test
//! suite and do not require any external dependencies.

#![allow(dead_code, unused_imports, unexpected_cfgs)]

use rinch::prelude::*;
use std::rc::Rc;

// ============================================================
// Basic element tests
// ============================================================

#[component]
fn test_empty_div() -> NodeHandle {
    rsx! { div {} }
}

#[component]
fn test_div_with_text() -> NodeHandle {
    rsx! { div { "Hello, world!" } }
}

#[component]
fn test_div_with_multiple_text() -> NodeHandle {
    rsx! { div { "Hello, " "world!" } }
}

#[component]
fn test_div_with_attributes() -> NodeHandle {
    rsx! { div { class: "container", id: "main", "content" } }
}

#[component]
fn test_nested_elements() -> NodeHandle {
    rsx! {
        div {
            p { "Paragraph 1" }
            p { "Paragraph 2" }
            span { "Inline" }
        }
    }
}

#[component]
fn test_deeply_nested() -> NodeHandle {
    rsx! {
        div { div { div { p { "deep" } } } }
    }
}

// ============================================================
// Expression children
// ============================================================

#[component]
fn test_braced_expression() -> NodeHandle {
    let x = 42;
    rsx! { div { {x.to_string()} } }
}

#[component]
fn test_reactive_closure() -> NodeHandle {
    let count = Signal::new(0);
    rsx! { div { {|| count.get().to_string()} } }
}

#[component]
fn test_reactive_attribute() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            class: {|| if count.get() > 5 { "high" } else { "low" }},
            "value"
        }
    }
}

// ============================================================
// Event handlers
// ============================================================

#[component]
fn test_onclick() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        button {
            onclick: move || count.update(|n| *n += 1),
            "Click me"
        }
    }
}

// ============================================================
// Conditional rendering (native if/else)
// ============================================================

#[component]
fn test_if_basic() -> NodeHandle {
    let visible = Signal::new(true);
    rsx! {
        div {
            if visible.get() {
                div { "Visible!" }
            }
        }
    }
}

#[component]
fn test_if_else() -> NodeHandle {
    let visible = Signal::new(true);
    rsx! {
        div {
            if visible.get() {
                div { "Visible!" }
            } else {
                div { "Hidden" }
            }
        }
    }
}

// ============================================================
// List rendering (native for loops)
// ============================================================

#[derive(Clone, Debug, PartialEq)]
struct TestItem {
    id: u32,
    name: String,
}

#[component]
fn test_for_basic() -> NodeHandle {
    let items = Signal::new(vec![
        TestItem {
            id: 1,
            name: "Alice".into(),
        },
        TestItem {
            id: 2,
            name: "Bob".into(),
        },
    ]);
    rsx! {
        div {
            for item in items.get() {
                p { key: item.id, {item.name.clone()} }
            }
        }
    }
}

#[allow(unused_variables)]
#[component]
fn test_for_with_closures() -> NodeHandle {
    let items = Signal::new(vec![
        TestItem {
            id: 1,
            name: "Alice".into(),
        },
        TestItem {
            id: 2,
            name: "Bob".into(),
        },
    ]);
    rsx! {
        div {
            for item in items.get() {
                let id = item.id;
                div { key: item.id,
                    {item.name.clone()}
                    button {
                        onclick: move || {
                            items.update(|list| list.retain(|t| t.id != id));
                        },
                        "Delete"
                    }
                }
            }
        }
    }
}

// ============================================================
// Component tests (requires rinch-components feature)
// ============================================================

#[cfg(feature = "components")]
mod component_tests {
    use super::*;

    #[component]
    fn test_button_component() -> NodeHandle {
        rsx! {
            Button {
                variant: "filled",
                onclick: || {},
                "Click"
            }
        }
    }

    #[component]
    fn test_component_with_style() -> NodeHandle {
        rsx! {
            Button {
                variant: "filled",
                style: "margin: 10px",
                "Styled"
            }
        }
    }

    #[component]
    fn test_component_with_class() -> NodeHandle {
        rsx! {
            Button {
                variant: "filled",
                class: "my-button",
                "Classed"
            }
        }
    }

    #[component]
    fn test_component_with_reactive_style() -> NodeHandle {
        let active = Signal::new(false);
        rsx! {
            Button {
                variant: "filled",
                style: {|| if active.get() { "background: red" } else { "" }},
                "Dynamic"
            }
        }
    }

    #[component]
    fn test_component_with_reactive_class() -> NodeHandle {
        let active = Signal::new(false);
        rsx! {
            Button {
                variant: "filled",
                class: {|| if active.get() { "active" } else { "" }},
                "Dynamic Class"
            }
        }
    }

    #[component]
    fn test_text_component() -> NodeHandle {
        rsx! {
            Text { size: "lg", color: "dimmed", "Hello" }
        }
    }

    #[component]
    fn test_stack_with_children() -> NodeHandle {
        rsx! {
            Stack {
                gap: "md",
                div { "Item 1" }
                div { "Item 2" }
            }
        }
    }

    #[component]
    fn test_for_inside_component() -> NodeHandle {
        let items = Signal::new(vec![
            TestItem {
                id: 1,
                name: "Alice".into(),
            },
            TestItem {
                id: 2,
                name: "Bob".into(),
            },
        ]);
        rsx! {
            Stack {
                gap: "sm",
                for item in items.get() {
                    Text { key: item.id, {item.name.clone()} }
                }
            }
        }
    }

    #[component]
    fn test_if_inside_component() -> NodeHandle {
        let visible = Signal::new(true);
        rsx! {
            Stack {
                if visible.get() {
                    Text { "Visible!" }
                } else {
                    Text { "Hidden" }
                }
            }
        }
    }
}

// Main function not needed for test binary, but required for compilation.
// If this file compiles, all RSX patterns work correctly.
fn main() {}
