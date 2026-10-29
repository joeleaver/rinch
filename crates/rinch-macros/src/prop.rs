//! RSX property parsing.

use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Result, Token};

/// A property in RSX (name: value).
///
/// Supports simple (`class`), hyphenated (`data-viewport`, `aria-label`,
/// `stroke-width`) and quoted (`"title"`, issue #1653) attribute names.
pub struct RsxProp {
    /// The attribute name as a string (unrawified for matching).
    /// E.g., "class", "data-viewport", "type" (even if written as `r#type`).
    pub name: String,
    /// The name was written as a string literal — `"title": …`. A quoted name
    /// is always the HTML attribute of that name, on any element (issue
    /// #1653): on a component it goes to the root element the way a
    /// hyphenated name does (#433), never to a struct field; on an HTML
    /// element it is never an event, a `_fn` prop or a style shorthand.
    pub quoted: bool,
    /// The first ident token (preserved for codegen of component struct
    /// fields); `None` for a quoted name, which is never a field.
    first_ident: Option<Ident>,
    name_span: proc_macro2::Span,
    pub value: Expr,
}

impl RsxProp {
    /// The span of the prop's name, for a diagnostic about the name itself.
    pub fn name_span(&self) -> proc_macro2::Span {
        self.name_span
    }

    /// Convert the name back to an `Ident` for use in `quote!` codegen
    /// (e.g., component struct field assignment). For simple names this is
    /// straightforward; for `r#type` the raw prefix is preserved.
    /// Panics for hyphenated and quoted names — those aren't struct fields. A
    /// component never asks for one: `component_codegen::is_root_attribute`
    /// routes either to the component's root element instead (issues #433,
    /// #1653).
    pub fn name_as_ident(&self) -> Ident {
        match &self.first_ident {
            // For non-hyphenated names, return the original ident (preserves r# prefix)
            Some(ident) if !self.name.contains('-') => ident.clone(),
            _ => panic!("attribute '{}' cannot be used as a struct field", self.name),
        }
    }
}

/// Whether `name` is an attribute name every backend can write.
///
/// The ASCII subset of XML's `Name` production: a letter, `_` or `:`, then
/// letters, digits, `-`, `_`, `.` or `:`. It is the set `setAttribute`
/// accepts in every browser (the newer, looser DOM rule is not in all of
/// them), so a name that passes here never throws on the web backend, and
/// HTML has no attribute outside it.
fn is_attribute_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// Refuse a quoted name that is not an attribute, or that is one `rsx!` gives
/// a meaning of its own, where the quote would silently drop that meaning.
fn check_quoted_name(lit: &LitStr) -> Result<String> {
    let name = lit.value();
    if !is_attribute_name(&name) {
        return Err(syn::Error::new(
            lit.span(),
            format!(
                "`{name:?}` is not an attribute name\n\
                 note: a quoted name is written as an HTML attribute; it starts with an \
                 ASCII letter, `_` or `:`, followed by ASCII letters, digits, `-`, `_`, \
                 `.` or `:`"
            ),
        ));
    }
    let lower = name.to_ascii_lowercase();
    if matches!(lower.as_str(), "style" | "class" | "key" | "ref") {
        let what = match lower.as_str() {
            "style" => {
                "is merged over the declarations already on the element or the component's root"
            }
            "class" => {
                "is that attribute on an element, and on a component is merged with the component's own classes"
            }
            "key" => "keys a `for` loop's rows; it is not written to the DOM",
            _ => "binds the element's handle; it is not written to the DOM",
        };
        return Err(syn::Error::new(
            lit.span(),
            format!(
                "a quoted name is always a plain HTML attribute, and `{lower}` is one `rsx!` \
                 gives its own meaning\n\
                 help: write `{lower}:` unquoted, which {what}"
            ),
        ));
    }
    if lower.starts_with("on") {
        return Err(syn::Error::new(
            lit.span(),
            format!(
                "`{name}` is an event handler content attribute, a string of script, \
                 which rinch never runs\n\
                 help: write the handler unquoted: `{lower}: move || …`"
            ),
        ));
    }
    Ok(name)
}

impl Parse for RsxProp {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(LitStr) {
            let lit: LitStr = input.parse()?;
            let name = check_quoted_name(&lit)?;
            input.parse::<Token![:]>()?;
            let value: Expr = input.parse()?;
            return Ok(RsxProp {
                name,
                quoted: true,
                first_ident: None,
                name_span: lit.span(),
                value,
            });
        }

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
            quoted: false,
            name_span: first.span(),
            first_ident: Some(first),
            value,
        })
    }
}
