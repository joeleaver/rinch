//! RSX node types.

use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Result, token};

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
        } else {
            Ok(RsxNode::Element(input.parse()?))
        }
    }
}
