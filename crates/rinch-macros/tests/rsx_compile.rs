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
    let count = use_signal(|| 0);
    rsx! { div { {|| count.get().to_string()} } }
}

#[component]
fn test_reactive_attribute() -> NodeHandle {
    let count = use_signal(|| 0);
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
    let count = use_signal(|| 0);
    rsx! {
        button {
            onclick: move || count.update(|n| *n += 1),
            "Click me"
        }
    }
}

// ============================================================
// Show component
// ============================================================

#[component]
fn test_show_eager() -> NodeHandle {
    let visible = use_signal(|| true);
    rsx! {
        Show {
            when: {move || visible.get()},
            div { "Visible!" }
        }
    }
}

#[component]
fn test_show_lazy_then() -> NodeHandle {
    let visible = use_signal(|| true);
    rsx! {
        Show {
            when: {move || visible.get()},
            then: |__scope: &mut RenderScope| rsx! { div { "Visible!" } },
        }
    }
}

#[component]
fn test_show_with_fallback() -> NodeHandle {
    let visible = use_signal(|| true);
    rsx! {
        Show {
            when: {move || visible.get()},
            then: |__scope: &mut RenderScope| rsx! { div { "Visible!" } },
            fallback: |__scope: &mut RenderScope| rsx! { div { "Hidden" } },
        }
    }
}

#[component]
fn test_show_eager_with_fallback() -> NodeHandle {
    let visible = use_signal(|| true);
    rsx! {
        Show {
            when: {move || visible.get()},
            fallback: |__scope: &mut RenderScope| rsx! { div { "Hidden" } },
            div { "Visible!" }
        }
    }
}

// ============================================================
// For component
// ============================================================

#[component]
fn test_for_basic() -> NodeHandle {
    let items = use_signal(|| vec!["a".to_string(), "b".to_string()]);
    rsx! {
        div {
            For {
                each: {move || items.get().into_iter().enumerate().map(|(i, item)| {
                    ForItem::new(i.to_string(), item)
                }).collect()},
                |item: &ForItem| {
                    let text = item.downcast::<String>().unwrap().clone();
                    rsx! { p { {text} } }
                }
            }
        }
    }
}

#[component]
fn test_for_with_braced_closure() -> NodeHandle {
    let items = use_signal(|| vec!["a".to_string(), "b".to_string()]);
    rsx! {
        div {
            For {
                each: {move || items.get().into_iter().enumerate().map(|(i, item)| {
                    ForItem::new(i.to_string(), item)
                }).collect()},
                {|item: &ForItem| {
                    let text = item.downcast::<String>().unwrap().clone();
                    rsx! { p { {text} } }
                }}
            }
        }
    }
}

// ============================================================
// Bare closure children (For view function)
// ============================================================

#[component]
fn test_bare_closure_child() -> NodeHandle {
    let items = use_signal(|| vec![1u32, 2, 3]);
    rsx! {
        div {
            For {
                each: {move || items.get().into_iter().map(|n| {
                    ForItem::new(n.to_string(), n)
                }).collect()},
                |item: &ForItem| {
                    let n = *item.downcast::<u32>().unwrap();
                    rsx! { span { {n.to_string()} } }
                }
            }
        }
    }
}

// ============================================================
// Widget tests (requires rinch-widgets feature)
// ============================================================

#[cfg(feature = "widgets")]
mod widget_tests {
    use super::*;

    #[component]
    fn test_button_widget() -> NodeHandle {
        rsx! {
            Button {
                variant: "filled",
                onclick: || {},
                "Click"
            }
        }
    }

    #[component]
    fn test_widget_with_style() -> NodeHandle {
        rsx! {
            Button {
                variant: "filled",
                style: "margin: 10px",
                "Styled"
            }
        }
    }

    #[component]
    fn test_widget_with_class() -> NodeHandle {
        rsx! {
            Button {
                variant: "filled",
                class: "my-button",
                "Classed"
            }
        }
    }

    #[component]
    fn test_widget_with_reactive_style() -> NodeHandle {
        let active = use_signal(|| false);
        rsx! {
            Button {
                variant: "filled",
                style: {|| if active.get() { "background: red" } else { "" }},
                "Dynamic"
            }
        }
    }

    #[component]
    fn test_widget_with_reactive_class() -> NodeHandle {
        let active = use_signal(|| false);
        rsx! {
            Button {
                variant: "filled",
                class: {|| if active.get() { "active" } else { "" }},
                "Dynamic Class"
            }
        }
    }

    #[component]
    fn test_text_widget() -> NodeHandle {
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
    fn test_for_inside_widget() -> NodeHandle {
        let items = use_signal(|| vec!["a".to_string(), "b".to_string()]);
        rsx! {
            Stack {
                gap: "sm",
                For {
                    each: {move || items.get().into_iter().enumerate().map(|(i, item)| {
                        ForItem::new(i.to_string(), item)
                    }).collect()},
                    |item: &ForItem| {
                        let text = item.downcast::<String>().unwrap().clone();
                        rsx! { Text { {text} } }
                    }
                }
            }
        }
    }

    #[component]
    fn test_show_inside_widget() -> NodeHandle {
        let visible = use_signal(|| true);
        rsx! {
            Stack {
                Show {
                    when: {move || visible.get()},
                    then: |__scope: &mut RenderScope| rsx! { Text { "Visible!" } },
                    fallback: |__scope: &mut RenderScope| rsx! { Text { "Hidden" } },
                }
            }
        }
    }
}

// Main function not needed for test binary, but required for compilation.
// If this file compiles, all RSX patterns work correctly.
fn main() {}
