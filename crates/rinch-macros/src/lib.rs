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

/// Attribute macro for defining components.
///
/// # Two modes based on function name casing:
///
/// ## lowercase functions — simple scope injection
///
/// For lowercase function names, the macro just injects `__scope: &mut RenderScope`
/// as the first parameter. Use this for the top-level app function and helper components.
///
/// ```ignore
/// #[component]
/// fn app() -> NodeHandle {
///     rsx! { div { "Hello" } }
/// }
/// // becomes: fn app(__scope: &mut RenderScope) -> NodeHandle { ... }
/// ```
///
/// ## PascalCase functions — Component struct generation
///
/// For PascalCase function names, the macro generates a struct + `Component` trait impl,
/// making the component usable in RSX with prop syntax (just like built-in components):
///
/// ```ignore
/// #[component]
/// fn TodoItem(
///     todo: Todo,
///     on_delete: Option<Callback>,
///     children: &[NodeHandle],
/// ) -> NodeHandle {
///     rsx! { div { {todo.name.clone()} } }
/// }
///
/// // Generates:
/// // - pub struct TodoItem { pub todo: Todo, pub on_delete: Option<Callback> }
/// // - impl Component for TodoItem { ... }
/// //
/// // Use in RSX like any component:
/// // TodoItem { todo: my_todo, on_delete: || remove(id) }
/// ```
///
/// ### Rules for PascalCase components:
///
/// - Parameters become public struct fields (must be owned types, not references)
/// - `children: &[NodeHandle]` is special — wired to Component::render's children, not a field
/// - A manual `Default` impl is generated with per-field defaults for known types
///   (String, bool, Option, Vec, numeric types). Unknown types fall back to `Default::default()`.
/// - `Component`, `RenderScope`, and `NodeHandle` must be in scope (via `use rinch::prelude::*`)
#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = syn::parse_macro_input!(item as syn::ItemFn);
    let name_str = func.sig.ident.to_string();

    if name_str.starts_with(|c: char| c.is_uppercase()) {
        generate_component(func)
    } else {
        // lowercase: just inject __scope
        let mut func = func;
        let scope_param: syn::FnArg = syn::parse_quote!(__scope: &mut RenderScope);
        func.sig.inputs.insert(0, scope_param);
        quote::quote!(#func).into()
    }
}

/// Generate a Component struct + impl from a PascalCase `#[component]` function.
fn generate_component(func: syn::ItemFn) -> TokenStream {
    use quote::quote;

    let vis = &func.vis;
    let name = &func.sig.ident;
    let attrs = &func.attrs;
    let stmts = &func.block.stmts;

    // Collect struct fields and detect children parameter
    let mut fields: Vec<(Vec<syn::Attribute>, syn::Ident, Box<syn::Type>)> = Vec::new();
    let mut has_children = false;

    for param in &func.sig.inputs {
        if let syn::FnArg::Typed(pat_type) = param {
            if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                let param_name = pat_ident.ident.to_string();

                // children: &[NodeHandle] is special — wired to Component::render, not a field
                if param_name == "children" {
                    has_children = true;
                    continue;
                }

                // Reject reference types (Phase 1b) — component params must be owned
                if let syn::Type::Reference(_) = &*pat_type.ty {
                    return syn::Error::new_spanned(
                        &pat_type.ty,
                        format!(
                            "Component parameter `{}` uses a reference type. \
                             Component parameters must be owned types \
                             (e.g., `String` instead of `&str`) because they are stored in a struct.",
                            param_name
                        ),
                    )
                    .to_compile_error()
                    .into();
                }

                fields.push((
                    pat_type.attrs.clone(),
                    pat_ident.ident.clone(),
                    pat_type.ty.clone(),
                ));
            }
        }
    }

    // Struct field definitions
    let field_defs: Vec<_> = fields
        .iter()
        .map(|(attrs, ident, ty)| {
            quote! { #(#attrs)* pub #ident: #ty }
        })
        .collect();

    // Generate per-field default expressions for manual Default impl.
    // Known types get explicit defaults. Unknown types fall back to Default::default().
    let field_defaults: Vec<_> = fields
        .iter()
        .map(|(_, ident, ty)| {
            let default_expr = type_default_expr(ty);
            quote! { #ident: #default_expr }
        })
        .collect();

    // Clone self fields into local variables so the body can use param names directly
    let clone_stmts: Vec<_> = fields
        .iter()
        .map(|(_, ident, _)| {
            quote! { #[allow(unused)] let #ident = self.#ident.clone(); }
        })
        .collect();

    // Name the children param in Component::render based on whether the component uses it
    let children_param = if has_children {
        quote! { children }
    } else {
        quote! { _children }
    };

    quote! {
        #(#attrs)*
        #vis struct #name {
            #(#field_defs,)*
        }

        impl ::std::default::Default for #name {
            fn default() -> Self {
                Self {
                    #(#field_defaults,)*
                }
            }
        }

        impl ::std::fmt::Debug for #name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct(stringify!(#name)).finish_non_exhaustive()
            }
        }

        impl Component for #name {
            fn render(&self, __scope: &mut RenderScope, #children_param: &[NodeHandle]) -> NodeHandle {
                #(#clone_stmts)*
                #(#stmts)*
            }
        }
    }
    .into()
}

/// Get a default expression for a type based on token-level analysis.
///
/// Known types get explicit defaults:
/// - `String` → `String::new()`
/// - `bool` → `false`
/// - `Option<T>` → `None`
/// - `Vec<T>` → `Vec::new()`
/// - Integer types → `0`
/// - Float types → `0.0`
///
/// Unknown types fall back to `Default::default()`.
fn type_default_expr(ty: &syn::Type) -> proc_macro2::TokenStream {
    use quote::quote;

    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let type_name = segment.ident.to_string();
            match type_name.as_str() {
                "String" => return quote! { String::new() },
                "bool" => return quote! { false },
                "Option" => return quote! { None },
                "Vec" => return quote! { Vec::new() },
                "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
                | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" => {
                    return quote! { 0 }
                }
                "f32" => return quote! { 0.0f32 },
                "f64" => return quote! { 0.0f64 },
                _ => {}
            }
        }
    }

    // Unknown type: fall back to Default trait.
    // If the type doesn't impl Default and the user doesn't provide this prop,
    // they'll get a compile error pointing to this specific field.
    quote! { ::std::default::Default::default() }
}
