//! RSX node types.

use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Result, Token, token};

use crate::element::RsxElement;

/// A node in the RSX tree.
pub enum RsxNode {
    /// A component or HTML element with optional props and children.
    Element(RsxElement),
    /// A text literal.
    Text(LitStr),
    /// A Rust expression in braces.
    Expr(Expr),
}

impl Parse for RsxNode {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(LitStr) {
            Ok(RsxNode::Text(input.parse()?))
        } else if input.peek(token::Brace) {
            let content;
            syn::braced!(content in input);
            Ok(RsxNode::Expr(content.parse()?))
        } else if input.peek(Token![|]) || input.peek(Token![move]) {
            // Parse bare closure as expression (for For component view functions)
            Ok(RsxNode::Expr(input.parse()?))
        } else {
            Ok(RsxNode::Element(input.parse()?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_str;

    /// Helper to check what variant was parsed.
    fn parse_variant(input: &str) -> &'static str {
        match parse_str::<RsxNode>(input) {
            Ok(RsxNode::Text(_)) => "Text",
            Ok(RsxNode::Expr(_)) => "Expr",
            Ok(RsxNode::Element(_)) => "Element",
            Err(_) => "Error",
        }
    }

    // ── Text literal parsing ─────────────────────────────────────

    #[test]
    fn parse_text_literal() {
        assert_eq!(parse_variant(r#""hello world""#), "Text");
    }

    #[test]
    fn parse_text_empty_string() {
        assert_eq!(parse_variant(r#""""#), "Text");
    }

    #[test]
    fn parse_text_with_special_chars() {
        assert_eq!(parse_variant(r#""hello <b>world</b>""#), "Text");
    }

    // ── Braced expression parsing ────────────────────────────────

    #[test]
    fn parse_braced_expression_integer() {
        assert_eq!(parse_variant("{ 42 }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_variable() {
        assert_eq!(parse_variant("{ x }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_method_call() {
        assert_eq!(parse_variant("{ count.get() }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_to_string() {
        assert_eq!(parse_variant("{ count.get().to_string() }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_format() {
        assert_eq!(parse_variant(r#"{ format!("val: {}", x) }"#), "Expr");
    }

    // ── Braced closure parsing ───────────────────────────────────

    #[test]
    fn parse_braced_closure() {
        assert_eq!(parse_variant("{|| count.get()}"), "Expr");
    }

    #[test]
    fn parse_braced_closure_with_to_string() {
        assert_eq!(parse_variant("{|| count.get().to_string()}"), "Expr");
    }

    #[test]
    fn parse_braced_closure_with_if() {
        assert_eq!(
            parse_variant(r#"{|| if count.get() > 5 { "high" } else { "low" }}"#),
            "Expr"
        );
    }

    #[test]
    fn parse_braced_move_closure() {
        assert_eq!(parse_variant("{move || count.get()}"), "Expr");
    }

    // ── Bare closure parsing (For view functions) ────────────────

    #[test]
    fn parse_bare_closure_simple() {
        assert_eq!(parse_variant("|item| { item }"), "Expr");
    }

    #[test]
    fn parse_bare_closure_with_body() {
        assert_eq!(parse_variant("|item| { item.name.clone() }"), "Expr");
    }

    #[test]
    fn parse_bare_closure_with_type_annotation() {
        assert_eq!(
            parse_variant("|item: &ForItem| { item.name.clone() }"),
            "Expr"
        );
    }

    #[test]
    fn parse_move_closure() {
        assert_eq!(parse_variant("move |x| x + 1"), "Expr");
    }

    #[test]
    fn parse_move_closure_with_block() {
        assert_eq!(parse_variant("move |x| { x + 1 }"), "Expr");
    }

    // ── Element parsing ──────────────────────────────────────────

    #[test]
    fn parse_element_simple() {
        assert_eq!(parse_variant("div {}"), "Element");
    }

    #[test]
    fn parse_element_with_text() {
        assert_eq!(parse_variant(r#"div { "hello" }"#), "Element");
    }

    #[test]
    fn parse_element_with_props() {
        assert_eq!(parse_variant(r#"div { class: "foo" }"#), "Element");
    }

    #[test]
    fn parse_element_nested() {
        assert_eq!(parse_variant(r#"div { p { "hello" } }"#), "Element");
    }

    // ── Widget/component parsing (PascalCase → Element) ──────────

    #[test]
    fn parse_widget() {
        assert_eq!(parse_variant(r#"Button { variant: "filled" }"#), "Element");
    }

    #[test]
    fn parse_show_component() {
        assert_eq!(parse_variant("Show { when: {|| true} }"), "Element");
    }

    #[test]
    fn parse_for_component() {
        assert_eq!(
            parse_variant("For { each: {|| vec![]}, |item| { item } }"),
            "Element"
        );
    }

    // ── Error cases ──────────────────────────────────────────────

    #[test]
    fn parse_fails_on_empty_input() {
        assert!(parse_str::<RsxNode>("").is_err());
    }

    #[test]
    fn parse_fails_on_invalid_token() {
        // A lone number without braces is not valid RSX
        assert_eq!(parse_variant("42"), "Error");
    }
}
