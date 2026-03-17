//! RSX property parsing.

use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, Result, Token};

/// A property in RSX (name: value).
///
/// Supports both simple (`class`) and hyphenated (`data-viewport`, `aria-label`,
/// `stroke-width`) attribute names.
pub struct RsxProp {
    /// The attribute name as a string (unrawified for matching).
    /// E.g., "class", "data-viewport", "type" (even if written as `r#type`).
    pub name: String,
    /// The first ident token (preserved for codegen of component struct fields).
    first_ident: Ident,
    pub value: Expr,
}

impl RsxProp {
    /// Convert the name back to an `Ident` for use in `quote!` codegen
    /// (e.g., component struct field assignment). For simple names this is
    /// straightforward; for `r#type` the raw prefix is preserved.
    /// Panics for hyphenated names — those aren't valid struct fields.
    pub fn name_as_ident(&self) -> Ident {
        // For non-hyphenated names, return the original ident (preserves r# prefix)
        if !self.name.contains('-') {
            self.first_ident.clone()
        } else {
            panic!(
                "hyphenated attribute '{}' cannot be used as a struct field",
                self.name
            )
        }
    }
}

impl Parse for RsxProp {
    fn parse(input: ParseStream) -> Result<Self> {
        let first: Ident = Ident::parse_any(input)?;
        let mut name = first.unraw().to_string();

        // Greedily consume `-ident` segments for hyphenated names
        while input.peek(Token![-]) {
            let fork = input.fork();
            fork.parse::<Token![-]>().ok();
            if fork.peek(Ident) {
                input.parse::<Token![-]>()?;
                let seg: Ident = input.parse()?;
                name.push('-');
                name.push_str(&seg.to_string());
            } else {
                break;
            }
        }

        input.parse::<Token![:]>()?;
        let value: Expr = input.parse()?;
        Ok(RsxProp {
            name,
            first_ident: first,
            value,
        })
    }
}
