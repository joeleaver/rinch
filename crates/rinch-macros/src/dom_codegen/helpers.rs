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
            |__scope: &mut rinch::core::RenderScope, __children: &[rinch::core::NodeHandle]| -> rinch::core::NodeHandle {
                #body
            }
        }
    } else {
        quote! {
            |__scope: &mut rinch::core::RenderScope| -> rinch::core::NodeHandle {
                #body
            }
        }
    }
}
