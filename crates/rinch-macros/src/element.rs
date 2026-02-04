//! RSX element parsing.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, Result, Token};

use crate::node::RsxNode;
use crate::prop::RsxProp;

/// An element in RSX (component or HTML tag).
pub struct RsxElement {
    pub name: Ident,
    pub props: Vec<RsxProp>,
    pub children: Vec<RsxNode>,
}

impl Parse for RsxElement {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;

        let content;
        syn::braced!(content in input);

        let mut props = Vec::new();
        let mut children = Vec::new();

        while !content.is_empty() {
            // Try to parse as a prop (name: value)
            if content.peek(Ident) && content.peek2(Token![:]) && !content.peek2(Token![::]) {
                let prop: RsxProp = content.parse()?;
                props.push(prop);

                // Consume trailing comma if present
                if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            } else {
                // Parse as a child node
                let child: RsxNode = content.parse()?;
                children.push(child);

                // Consume trailing comma if present
                if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
        }

        Ok(RsxElement {
            name,
            props,
            children,
        })
    }
}

impl RsxElement {
    /// Core rinch components defined in rinch-core.
    fn is_core_component(&self) -> bool {
        let name = self.name.to_string();
        matches!(
            name.as_str(),
            "Window"
                | "ThemeProvider"
                | "AppMenu"
                | "Menu"
                | "MenuItem"
                | "MenuSeparator"
                | "Fragment"
                | "Portal"
                | "Show"
                | "For"
        )
    }

    /// Check if this is a widget (PascalCase name that isn't a core component).
    /// Widgets are third-party components that implement the Widget trait.
    fn is_widget(&self) -> bool {
        let name = self.name.to_string();
        // Must start with uppercase letter (PascalCase)
        let first_char = name.chars().next().unwrap_or('a');
        first_char.is_ascii_uppercase() && !self.is_core_component()
    }

    /// Check if this element is a rinch component (core component or widget).
    pub fn is_rinch_component(&self) -> bool {
        self.is_core_component() || self.is_widget()
    }
}
