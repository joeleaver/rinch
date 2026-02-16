//! DOM code generation for fine-grained reactive rendering.
//!
//! This module provides an alternative code generation path that creates
//! DOM nodes directly with Effects for reactive expressions.
//!
//! # Architecture
//!
//! Instead of generating HTML strings:
//! ```ignore
//! Element::Html(format!("<p>Count: {}</p>", count.get()))
//! ```
//!
//! This generates DOM construction code with Effects:
//! ```ignore
//! {
//!     let p = __scope.create_element("p");
//!     let text = __scope.create_text("Count: ");
//!     let value = __scope.create_text("");
//!
//!     // Reactive binding
//!     let value_handle = value.clone();
//!     let count_clone = count.clone();
//!     __scope.create_effect(move || {
//!         value_handle.set_text(&count_clone.get().to_string());
//!     });
//!
//!     p.append_child(&text);
//!     p.append_child(&value);
//!     p
//! }
//! ```
//!
//! # Usage
//!
//! This module is enabled when the `fine-grained` feature is active.
//! Components must accept a `&mut RenderScope` parameter and return a `NodeHandle`.

mod component;
mod control_flow;
pub mod helpers;
pub mod html;
pub mod widget;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::helpers::{get_closure_expr, is_literal_expr};
use crate::node::RsxNode;


/// Context for DOM code generation.
pub struct DomCodegenContext {
    /// Counter for generating unique variable names.
    var_counter: usize,
    /// Whether we're inside an effect (for nested reactive expressions).
    #[allow(dead_code)]
    in_effect: bool,
}

impl DomCodegenContext {
    pub fn new() -> Self {
        Self {
            var_counter: 0,
            in_effect: false,
        }
    }

    /// Generate a unique variable name.
    pub(crate) fn next_var(&mut self, prefix: &str) -> syn::Ident {
        let name = format!("__{}{}", prefix, self.var_counter);
        self.var_counter += 1;
        syn::Ident::new(&name, proc_macro2::Span::call_site())
    }
}

impl Default for DomCodegenContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate child code for a parent element.
///
/// Show and For elements use marker-based rendering and insert directly
/// into the parent. Other children are appended normally.
pub(crate) fn generate_child_code(
    child: &RsxNode,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    match child {
        RsxNode::Element(el) if el.name == "Show" => {
            // Show inserts marker + content directly into parent
            control_flow::element_to_dom_show(el, ctx, parent_var)
        }
        RsxNode::Element(el) if el.name == "For" => {
            // For inserts marker + items directly into parent
            control_flow::element_to_dom_for(el, ctx, parent_var)
        }
        _ => {
            let child_var = ctx.next_var("child");
            let child_dom = node_to_dom(child, ctx);
            quote! {
                let #child_var = #child_dom;
                #parent_var.append_child(&#child_var);
            }
        }
    }
}

/// Generate DOM construction code for an RSX element.
pub fn element_to_dom(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let name = element.name.to_string();

    // Show and For use marker-based rendering and need a parent context.
    // When nested inside an element, they're handled by generate_child_code.
    // At top level, create a display:contents wrapper.
    if name == "Show" {
        let wrapper_var = ctx.next_var("show_for_wrapper");
        let inner = control_flow::element_to_dom_show(element, ctx, &wrapper_var);
        return quote! {
            {
                let #wrapper_var = __scope.create_element("div");
                #wrapper_var.set_attribute("style", "display:contents");
                #inner
                #wrapper_var
            }
        };
    }
    if name == "For" {
        let wrapper_var = ctx.next_var("show_for_wrapper");
        let inner = control_flow::element_to_dom_for(element, ctx, &wrapper_var);
        return quote! {
            {
                let #wrapper_var = __scope.create_element("div");
                #wrapper_var.set_attribute("style", "display:contents");
                #inner
                #wrapper_var
            }
        };
    }

    // Special handling for Fragment - just wraps children in a span
    if name == "Fragment" {
        return control_flow::element_to_dom_fragment(element, ctx);
    }

    // Special handling for ThemeProvider - wraps children with reactive theme
    if name == "ThemeProvider" {
        return component::element_to_dom_theme_provider(element, ctx);
    }

    // Check if this is a rinch component (handled differently)
    if element.is_rinch_component() {
        return component::element_to_dom_component(element, ctx);
    }

    // Generate HTML element
    html::element_to_dom_html(element, ctx)
}

/// Generate DOM construction code for an RSX node.
pub fn node_to_dom(node: &RsxNode, ctx: &mut DomCodegenContext) -> TokenStream2 {
    match node {
        RsxNode::Element(element) => element_to_dom(element, ctx),
        RsxNode::Text(lit) => {
            // Text nodes contain raw text - no HTML escaping needed
            // (HTML escaping is only for raw HTML string generation)
            let text = lit.value();
            quote! {
                __scope.create_text(#text)
            }
        }
        RsxNode::Expr(expr) => {
            // Check if this is a simple literal expression
            if is_literal_expr(expr) {
                let text = crate::helpers::expr_to_string(expr);
                quote! {
                    __scope.create_text(#text)
                }
            } else if let Some(closure) = get_closure_expr(expr) {
                // Closure expression - create text node and effect that updates it directly
                let text_var = ctx.next_var("text");
                quote! {
                    {
                        // Create text node with initial value from closure
                        let #text_var = __scope.create_text(
                            &::std::string::ToString::to_string(&(#closure)())
                        );
                        // Effect updates text node directly via DOM API
                        let __text_clone = #text_var.clone();
                        __scope.create_effect(move || {
                            __text_clone.set_text(
                                &::std::string::ToString::to_string(&(#closure)())
                            );
                        });
                        #text_var
                    }
                }
            } else {
                // Non-closure expression - evaluate once and convert to NodeHandle via IntoNode
                // This handles both NodeHandle returns (from component functions) and text values
                quote! {
                    ::rinch::core::IntoNode::into_node(#expr, __scope)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::helpers::is_likely_reactive;
    use syn::{parse_quote, Expr};

    #[test]
    fn test_is_likely_reactive_method_call() {
        let expr: Expr = parse_quote!(count.get());
        assert!(is_likely_reactive(&expr));
    }

    #[test]
    fn test_is_likely_reactive_literal() {
        let expr: Expr = parse_quote!(42);
        assert!(!is_likely_reactive(&expr));
    }

    #[test]
    fn test_is_likely_reactive_binary() {
        let expr: Expr = parse_quote!(count.get() + 1);
        assert!(is_likely_reactive(&expr));
    }
}
