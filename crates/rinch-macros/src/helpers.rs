//! Utility functions for RSX code generation.

use quote::ToTokens;
use syn::Expr;

/// Check if a property name is an event handler.
pub fn is_event_prop(name: &str) -> bool {
    name.starts_with("on")
}

/// Check if an expression is a literal (can be evaluated at compile time).
pub fn is_literal_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(_))
}

/// Extract the closure expression from an expression.
/// Returns the closure if the expression is a closure or a block containing a single closure.
pub fn get_closure_expr(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Closure(_) => Some(expr),
        Expr::Block(block) => {
            // Check if block contains exactly one statement that is a closure expression
            if block.block.stmts.len() == 1
                && let syn::Stmt::Expr(inner, None) = &block.block.stmts[0]
                && matches!(inner, Expr::Closure(_))
            {
                return Some(inner);
            }
            None
        }
        _ => None,
    }
}

/// Check if an expression is a boolean literal.
pub fn is_literal_bool(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Bool(_)))
}

/// Check if an expression is an integer literal.
pub fn is_literal_int(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Int(_)))
}

/// Check if an expression is a string literal.
pub fn is_literal_string(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Str(_)))
}

/// Convert an expression to a string (for HTML attribute values).
pub fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Str(s) => s.value(),
            syn::Lit::Int(i) => i.base10_digits().to_string(),
            syn::Lit::Float(f) => f.base10_digits().to_string(),
            syn::Lit::Bool(b) => b.value.to_string(),
            _ => expr.to_token_stream().to_string(),
        },
        _ => expr.to_token_stream().to_string(),
    }
}

/// Check if an HTML tag is a void element (self-closing).
pub fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
