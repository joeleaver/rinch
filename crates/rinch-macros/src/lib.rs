//! Procedural macros for rinch - RSX syntax.
//!
//! Provides the `rsx!` macro for declarative UI definition with fine-grained
//! reactive rendering.
//!
//! The `rsx!` macro generates DOM construction code that creates nodes directly
//! with Effects for reactive expressions. This enables surgical DOM updates -
//! only the affected nodes are updated when signals change.

mod dom_codegen;
mod element;
mod helpers;
mod node;
mod prop;

use proc_macro::TokenStream;

use node::RsxNode;

/// RSX macro for fine-grained reactive DOM construction.
///
/// This generates DOM construction code that creates nodes directly with
/// Effects for reactive expressions. Only the affected DOM nodes are updated
/// when signals change.
///
/// # Requirements
///
/// - A `__scope: &mut RenderScope` must be in scope
/// - Returns a `NodeHandle`
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// fn counter(__scope: &mut RenderScope) -> NodeHandle {
///     let count = use_signal(|| 0);
///     let count_inc = count.clone();
///
///     rsx! {
///         div {
///             // Closure syntax {|| ...} creates reactive Effects
///             p { "Count: " {|| count.get().to_string()} }
///             button { onclick: move || count_inc.update(|n| *n += 1),
///                 "Increment"
///             }
///         }
///     }
/// }
/// ```
///
/// # Reactive Expressions
///
/// Use closure syntax `{|| expr}` for reactive text that updates when signals change:
///
/// ```ignore
/// // Static - captured once, never updates
/// p { "Count: " {count.get()} }
///
/// // Reactive - creates Effect, updates on signal change
/// p { "Count: " {|| count.get().to_string()} }
/// ```
///
/// # How It Works
///
/// The macro transforms:
/// ```ignore
/// div { p { "Count: " {|| count.get().to_string()} } }
/// ```
///
/// Into:
/// ```ignore
/// {
///     let __elem0 = __scope.create_element("div");
///     let __child0 = {
///         let __elem1 = __scope.create_element("p");
///         let __child1 = __scope.create_text("Count: ");
///         __elem1.append_child(&__child1);
///         let __text0 = __scope.create_text("");
///         let __handle = __text0.clone();
///         __scope.create_effect(move || {
///             __handle.set_text(&(|| count.get().to_string())());
///         });
///         __elem1.append_child(&__text0);
///         __elem1
///     };
///     __elem0.append_child(&__child0);
///     __elem0
/// }
/// ```
#[proc_macro]
pub fn rsx(input: TokenStream) -> TokenStream {
    let node = syn::parse_macro_input!(input as RsxNode);
    let mut ctx = dom_codegen::DomCodegenContext::new();
    dom_codegen::node_to_dom(&node, &mut ctx).into()
}
