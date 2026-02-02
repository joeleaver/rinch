//! RSX property parsing.

use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, Result, Token};

/// A property in RSX (name: value).
pub struct RsxProp {
    pub name: Ident,
    pub value: Expr,
}

impl Parse for RsxProp {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let value: Expr = input.parse()?;
        Ok(RsxProp { name, value })
    }
}
