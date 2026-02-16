//! Control flow DOM code generation.
//!
//! Handles `Fragment`, `Show` (conditional rendering), and `For` (list rendering).

use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};

use crate::element::RsxElement;
use crate::node::RsxNode;

use super::helpers::{extract_closure_body, extract_closure_typed_param};
use super::DomCodegenContext;

/// Generate DOM code for a Fragment (just renders children in an invisible wrapper).
pub fn element_to_dom_fragment(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let container_var = ctx.next_var("fragment");

    // Generate children - Show/For use marker-based insertion
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &container_var, ctx))
        .collect();

    // Use a span as a lightweight container
    quote! {
        {
            let #container_var = __scope.create_element("div");
            #container_var.set_attribute("data-fragment", "true");
            #(#children_code)*
            #container_var
        }
    }
}

/// Generate DOM code for Show component (conditional rendering).
///
/// Generates a call to `show_dom()` which inserts a comment marker and content
/// directly into the parent element. No wrapper span is created.
pub fn element_to_dom_show(
    element: &RsxElement,
    ctx: &mut DomCodegenContext,
    parent_var: &syn::Ident,
) -> TokenStream2 {
    // Get the span of the "Show" identifier for better error reporting
    let span = element.name.span();

    // Find the 'when' prop
    let when_prop = element.props.iter().find(|p| p.name == "when");

    let when_expr = match when_prop {
        Some(prop) => &prop.value,
        None => {
            return quote_spanned! {span=>
                compile_error!("Show component requires 'when' prop")
            };
        }
    };

    // Find the optional 'fallback' prop
    let fallback_prop = element.props.iter().find(|p| p.name == "fallback");

    // Find the optional 'then' prop for lazy evaluation
    let then_prop = element.props.iter().find(|p| p.name == "then");

    let wrapper_var = ctx.next_var("show_wrapper");

    // Build the then closure - either from explicit prop or from children
    let then_closure = if let Some(then_prop) = then_prop {
        // Lazy mode: pass the user's closure directly to show_dom
        // The user provides: |__scope| rsx! { ... } which already matches
        // the Fn(&mut RenderScope) -> NodeHandle signature
        let then_expr = &then_prop.value;
        quote! { #then_expr }
    } else {
        // Eager mode (backwards compatible): generate code from children
        let children_code: Vec<TokenStream2> = element
            .children
            .iter()
            .map(|child| super::node_to_dom(child, ctx))
            .collect();

        if children_code.is_empty() {
            quote! {
                |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                    __child_scope.create_element("div")
                }
            }
        } else if children_code.len() == 1 {
            let child = &children_code[0];
            quote! {
                move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                    // Temporarily swap in child scope as the active render scope
                    let __scope = __child_scope;
                    #child
                }
            }
        } else {
            quote! {
                move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                    // Temporarily swap in child scope as the active render scope
                    let __scope = __child_scope;
                    let #wrapper_var = __scope.create_element("div");
                    #wrapper_var.set_attribute("data-show-content", "true");
                    #(
                        {
                            let __child = #children_code;
                            #wrapper_var.append_child(&__child);
                        }
                    )*
                    #wrapper_var
                }
            }
        }
    };

    // Build the else closure if fallback prop exists
    let else_option = if let Some(fallback) = fallback_prop {
        let fallback_expr = &fallback.value;
        quote! { Some(#fallback_expr) }
    } else {
        quote! { None::<fn(&mut ::rinch::core::dom::RenderScope) -> ::rinch::core::dom::NodeHandle> }
    };

    // Generate call to show_dom - inserts marker + content into parent directly
    // Use quote_spanned to point errors at the Show keyword
    quote_spanned! {span=>
        {
            let __when_closure = #when_expr;
            ::rinch::core::show_dom(
                __scope,
                &#parent_var,
                __when_closure,
                #then_closure,
                #else_option
            );
        }
    }
}

/// Generate DOM code for For component (list rendering).
///
/// Generates a call to `for_each_dom()` which inserts a comment marker and items
/// directly into the parent element. No wrapper span is created.
pub fn element_to_dom_for(
    element: &RsxElement,
    _ctx: &mut DomCodegenContext,
    parent_var: &syn::Ident,
) -> TokenStream2 {
    // Get the span of the "For" identifier for better error reporting
    let span = element.name.span();

    // Find the 'each' prop (returns Vec<ForItem>)
    let each_prop = element.props.iter().find(|p| p.name == "each");

    let each_expr = match each_prop {
        Some(prop) => &prop.value,
        None => {
            return quote_spanned! {span=>
                compile_error!("For component requires 'each' prop")
            };
        }
    };

    // The child should be a single closure that takes &ForItem and returns Element
    // In RSX: For { each: ..., |item| rsx! { ... } }
    // This is represented as a child expression containing the closure
    if element.children.is_empty() {
        return quote_spanned! {span=>
            compile_error!("For component requires a view function as child: |item| rsx! { ... }")
        };
    }

    // Get the view closure from children
    // The view closure is the first child that is an expression containing a closure
    let view_closure = match &element.children[0] {
        RsxNode::Expr(expr) => expr.clone(),
        _ => {
            return quote_spanned! {span=>
                compile_error!("For component view must be a closure: |item| rsx! { ... }")
            };
        }
    };

    // Check if the view closure has a typed parameter for auto-downcast
    // If user writes |item: &Todo|, we generate automatic downcast
    let auto_downcast = extract_closure_typed_param(&view_closure);

    if let Some((param_name, param_type)) = auto_downcast {
        // Auto-downcast mode: user wrote |item: &Todo| { ... }
        // We need to extract the closure body
        let closure_body = extract_closure_body(&view_closure);
        quote_spanned! {span=>
            {
                let __each_closure = #each_expr;
                ::rinch::core::for_each_dom(
                    __scope,
                    &#parent_var,
                    __each_closure,
                    move |__item: &::rinch::core::ForItem, __child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                        let __scope = __child_scope;
                        let #param_name: &#param_type = __item.data.downcast_ref::<#param_type>()
                            .expect(concat!("ForItem type mismatch: expected ", stringify!(#param_type)));
                        #closure_body
                    }
                );
            }
        }
    } else {
        // Standard mode: pass through as-is
        quote_spanned! {span=>
            {
                let __each_closure = #each_expr;
                ::rinch::core::for_each_dom(
                    __scope,
                    &#parent_var,
                    __each_closure,
                    move |__item: &::rinch::core::ForItem, __child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                        let __scope = __child_scope;
                        let mut __view_fn = #view_closure;
                        __view_fn(__item)
                    }
                );
            }
        }
    }
}
