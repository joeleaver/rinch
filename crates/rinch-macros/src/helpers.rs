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

/// Map a style shorthand name to its CSS property name(s).
///
/// Returns `Some(&[&str])` with the CSS properties this shorthand expands to,
/// or `None` if the name is not a recognized shorthand.
pub fn expand_style_shorthand(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "w" => Some(&["width"]),
        "h" => Some(&["height"]),
        "miw" => Some(&["min-width"]),
        "maw" => Some(&["max-width"]),
        "mih" => Some(&["min-height"]),
        "mah" => Some(&["max-height"]),
        "p" => Some(&["padding"]),
        "pt" => Some(&["padding-top"]),
        "pb" => Some(&["padding-bottom"]),
        "pl" => Some(&["padding-left"]),
        "pr" => Some(&["padding-right"]),
        "px" => Some(&["padding-left", "padding-right"]),
        "py" => Some(&["padding-top", "padding-bottom"]),
        "m" => Some(&["margin"]),
        "mt" => Some(&["margin-top"]),
        "mb" => Some(&["margin-bottom"]),
        "ml" => Some(&["margin-left"]),
        "mr" => Some(&["margin-right"]),
        "mx" => Some(&["margin-left", "margin-right"]),
        "my" => Some(&["margin-top", "margin-bottom"]),
        "display" => Some(&["display"]),
        "pos" => Some(&["position"]),
        "top" => Some(&["top"]),
        "bottom" => Some(&["bottom"]),
        "left" => Some(&["left"]),
        "right" => Some(&["right"]),
        _ => None,
    }
}

/// Resolve a spacing scale value at compile time.
///
/// Maps `xs`, `sm`, `md`, `lg`, `xl` to `var(--rinch-spacing-{value})`.
/// Other values are passed through unchanged.
pub fn resolve_spacing_value(value: &str) -> String {
    match value {
        "xs" | "sm" | "md" | "lg" | "xl" => format!("var(--rinch-spacing-{})", value),
        _ => value.to_string(),
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
