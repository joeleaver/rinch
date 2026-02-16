//! Helper utilities for DOM code generation.
//!
//! Contains heuristic checks and closure extraction functions used
//! by other codegen submodules.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Expr;

/// Check if an expression likely contains reactive reads (Signal.get(), etc.).
///
/// This is a heuristic - it looks for method calls that might be reactive.
#[allow(dead_code)]
pub fn is_likely_reactive(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCall(call) => {
            let method = call.method.to_string();
            // Common reactive method names
            matches!(method.as_str(), "get" | "with" | "value" | "read")
                || is_likely_reactive(&call.receiver)
        }
        Expr::Call(call) => {
            // Check if it's a function that might be reactive
            call.args.iter().any(is_likely_reactive)
        }
        Expr::Binary(bin) => is_likely_reactive(&bin.left) || is_likely_reactive(&bin.right),
        Expr::Unary(un) => is_likely_reactive(&un.expr),
        Expr::Paren(p) => is_likely_reactive(&p.expr),
        Expr::Reference(r) => is_likely_reactive(&r.expr),
        Expr::Lit(_) => false,
        Expr::Path(_) => false, // Variable references are not inherently reactive
        _ => true,              // Assume unknown expressions might be reactive
    }
}

/// Generate a fine-grained component function signature.
///
/// Components in fine-grained mode take a RenderScope and return a NodeHandle:
/// ```ignore
/// fn my_component(__scope: &mut RenderScope) -> NodeHandle {
///     // ...
/// }
/// ```
#[allow(dead_code)]
pub fn generate_component_wrapper(body: TokenStream2, has_children: bool) -> TokenStream2 {
    if has_children {
        quote! {
            |__scope: &mut ::rinch::core::RenderScope, __children: &[::rinch::core::NodeHandle]| -> ::rinch::core::NodeHandle {
                #body
            }
        }
    } else {
        quote! {
            |__scope: &mut ::rinch::core::RenderScope| -> ::rinch::core::NodeHandle {
                #body
            }
        }
    }
}

/// Extract a typed parameter from a closure expression for For auto-downcast.
///
/// If the closure is `|item: &Todo| { ... }` where Todo != ForItem,
/// returns Some((item_ident, Todo_type)).
pub fn extract_closure_typed_param(expr: &Expr) -> Option<(syn::Ident, syn::Type)> {
    let closure = match expr {
        Expr::Closure(c) => c,
        _ => return None,
    };

    // Must have exactly one parameter
    if closure.inputs.len() != 1 {
        return None;
    }

    let param = &closure.inputs[0];
    // Must be a typed pattern: `item: &Type`
    let pat_type = match param {
        syn::Pat::Type(pt) => pt,
        _ => return None,
    };

    // Get the parameter name
    let param_name = match &*pat_type.pat {
        syn::Pat::Ident(pi) => pi.ident.clone(),
        _ => return None,
    };

    // Get the type - must be a reference type &T
    let inner_type = match &*pat_type.ty {
        syn::Type::Reference(r) => &*r.elem,
        _ => return None,
    };

    // Check if the type is ForItem - if so, don't auto-downcast
    if let syn::Type::Path(tp) = inner_type {
        let last_seg = tp.path.segments.last();
        if let Some(seg) = last_seg
            && seg.ident == "ForItem"
        {
            return None;
        }
    }

    Some((param_name, inner_type.clone()))
}

/// Extract the body of a closure expression.
pub fn extract_closure_body(expr: &Expr) -> &Expr {
    match expr {
        Expr::Closure(c) => &c.body,
        _ => expr,
    }
}
