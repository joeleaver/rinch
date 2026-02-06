//! Procedural macros for rinch - RSX syntax and component attributes.
//!
//! Provides the `rsx!` macro and `#[component]` attribute macro for declarative
//! UI definition with fine-grained reactive rendering.
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
/// - A `__scope: &mut RenderScope` must be in scope.
///   Use `#[component]` on your function to inject this automatically.
/// - Returns a `NodeHandle`
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// #[component]
/// fn counter() -> NodeHandle {
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

/// Attribute macro that injects `__scope: &mut RenderScope` as the first parameter.
///
/// This eliminates the need to manually write the `__scope` parameter in every
/// component function. The macro transforms:
///
/// ```ignore
/// #[component]
/// fn app() -> NodeHandle {
///     rsx! { div { "Hello" } }
/// }
/// ```
///
/// Into:
///
/// ```ignore
/// fn app(__scope: &mut RenderScope) -> NodeHandle {
///     rsx! { div { "Hello" } }
/// }
/// ```
///
/// Functions with existing parameters get `__scope` prepended:
///
/// ```ignore
/// #[component]
/// fn card(title: &str) -> NodeHandle { ... }
/// // becomes: fn card(__scope: &mut RenderScope, title: &str) -> NodeHandle { ... }
/// ```
#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut func = syn::parse_macro_input!(item as syn::ItemFn);

    // Build `__scope: &mut RenderScope`
    let scope_param: syn::FnArg = syn::parse_quote!(__scope: &mut RenderScope);

    // Prepend as first parameter
    func.sig.inputs.insert(0, scope_param);

    quote::quote!(#func).into()
}
