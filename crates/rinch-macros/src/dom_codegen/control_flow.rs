//! Control flow DOM code generation.
//!
//! Handles `Fragment`, `Show` (conditional rendering), `For` (list rendering),
//! and native `if`/`for`/`match` control flow in RSX.

use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};

use crate::element::RsxElement;
use crate::node::{RsxElseBranch, RsxForLoop, RsxIfBlock, RsxMatchBlock, RsxNode};

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

// ============================================================================
// Native Control Flow Codegen (if / for / match)
// ============================================================================

/// Generate DOM code for a native `if` / `else if` / `else` block in RSX.
///
/// Desugars to `show_dom()` calls. The condition is auto-wrapped in a `move ||` closure
/// to make it reactive. `else if` chains become nested `show_dom` calls.
pub fn generate_if_block(
    if_block: &RsxIfBlock,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let condition = &if_block.condition;

    // Build the condition closure
    let when_closure = if if_block.is_if_let {
        // if let Some(x) = expr { ... }
        // Condition: move || matches!(expr, Pattern)
        let pattern = if_block.pattern.as_ref().unwrap();
        quote! { move || matches!(#condition, #pattern) }
    } else {
        // Plain if: move || condition
        quote! { move || { #condition } }
    };

    // Build then closure from children
    let then_closure = generate_branch_closure(&if_block.then_children, if_block, ctx);

    // Build else closure
    let else_option = match &if_block.else_branch {
        Some(RsxElseBranch::Else(children)) => {
            let else_closure = generate_children_closure(children, ctx);
            quote! { Some(#else_closure) }
        }
        Some(RsxElseBranch::ElseIf(inner_if)) => {
            // Nested else-if: the else branch renders a display:contents wrapper
            // containing a nested show_dom
            let wrapper_var = ctx.next_var("elif_wrap");
            let nested = generate_if_block(inner_if, &wrapper_var, ctx);
            quote! {
                Some(move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    let #wrapper_var = __scope.create_element("div");
                    #wrapper_var.set_attribute("style", "display:contents");
                    #nested
                    #wrapper_var
                })
            }
        }
        None => {
            quote! { None::<fn(&mut ::rinch::core::dom::RenderScope) -> ::rinch::core::dom::NodeHandle> }
        }
    };

    quote! {
        {
            ::rinch::core::show_dom(
                __scope,
                &#parent_var,
                #when_closure,
                #then_closure,
                #else_option
            );
        }
    }
}

/// Generate a render closure for `if` then-branch children.
///
/// For `if let`, the closure re-destructures the expression to bind variables.
fn generate_branch_closure(
    children: &[RsxNode],
    if_block: &RsxIfBlock,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let body = generate_children_body(children, ctx);

    if if_block.is_if_let {
        // Re-destructure to bind variables in the then branch
        let pattern = if_block.pattern.as_ref().unwrap();
        let condition = &if_block.condition;
        quote! {
            move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                let __scope = __child_scope;
                #[allow(unreachable_patterns)]
                let #pattern = #condition else { unreachable!() };
                #body
            }
        }
    } else {
        quote! {
            move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                let __scope = __child_scope;
                #body
            }
        }
    }
}

/// Generate a render closure from a list of RSX children.
fn generate_children_closure(
    children: &[RsxNode],
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let body = generate_children_body(children, ctx);
    quote! {
        move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
            let __scope = __child_scope;
            #body
        }
    }
}

/// Generate the body code for a list of RSX children, returning a single NodeHandle.
///
/// If there's one child, it's returned directly. Multiple children are wrapped
/// in a `display:contents` div.
fn generate_children_body(
    children: &[RsxNode],
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    if children.is_empty() {
        quote! { __scope.create_element("div") }
    } else if children.len() == 1 {
        // Single child — check if it's a control flow node that needs a parent
        match &children[0] {
            RsxNode::IfBlock(_) | RsxNode::ForLoop(_) | RsxNode::MatchBlock(_) => {
                // Control flow nodes insert into a parent, so we need a wrapper
                let wrapper = ctx.next_var("cf_wrap");
                let child_code = super::generate_child_code(&children[0], &wrapper, ctx);
                quote! {
                    {
                        let #wrapper = __scope.create_element("div");
                        #wrapper.set_attribute("style", "display:contents");
                        #child_code
                        #wrapper
                    }
                }
            }
            _ => {
                let child_code = super::node_to_dom(&children[0], ctx);
                quote! { #child_code }
            }
        }
    } else {
        let wrapper = ctx.next_var("branch_wrap");
        let children_code: Vec<TokenStream2> = children
            .iter()
            .map(|child| super::generate_child_code(child, &wrapper, ctx))
            .collect();
        quote! {
            {
                let #wrapper = __scope.create_element("div");
                #wrapper.set_attribute("style", "display:contents");
                #(#children_code)*
                #wrapper
            }
        }
    }
}

/// Generate DOM code for a native `for` loop in RSX.
///
/// Desugars to `for_each_dom_typed()`. The iterator expression is auto-wrapped
/// in a `move ||` closure. If a `key:` prop is found on the first child element,
/// it's extracted as the key function.
pub fn generate_for_loop(
    for_loop: &RsxForLoop,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let pattern = &for_loop.pattern;
    let iter_expr = &for_loop.iter_expr;

    // Try to extract key from first child element's `key:` prop
    let key_fn = extract_key_expr(for_loop);

    // Build the view closure body from children
    let body = generate_children_body(&for_loop.children, ctx);

    // Collection closure: move || iter_expr.into_iter().collect::<Vec<_>>()
    let collection = quote! {
        move || (#iter_expr).into_iter().collect::<Vec<_>>()
    };

    quote! {
        {
            ::rinch::core::for_each_dom_typed(
                __scope,
                &#parent_var,
                #collection,
                |#pattern| ::std::string::ToString::to_string(&#key_fn),
                |#pattern, __child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    #body
                }
            );
        }
    }
}

/// Extract a `key:` expression from the first child element of a for loop.
///
/// Returns the key expression token stream. If no `key:` prop is found,
/// falls back to Debug formatting the item.
///
/// Note: The `key:` prop is left on the element. HTML codegen treats `key`
/// as a special attribute and skips it (it would just become a harmless
/// `set_attribute("key", ...)` otherwise).
fn extract_key_expr(for_loop: &RsxForLoop) -> TokenStream2 {
    // Look for key: prop on the first child element
    if let Some(RsxNode::Element(el)) = for_loop.children.first()
        && let Some(key_prop) = el.props.iter().find(|p| p.name == "key")
    {
        let key_expr = &key_prop.value;
        return quote! { #key_expr };
    }

    // No key prop found — use debug format of the item as key (fallback)
    let pattern = &for_loop.pattern;
    quote! { format!("{:?}", #pattern) }
}

/// Generate DOM code for a native `match` block in RSX.
///
/// Desugars to `match_dom()`. The scrutinee is evaluated in a discriminant closure
/// that returns a branch index. Each arm becomes a boxed render closure.
pub fn generate_match_block(
    match_block: &RsxMatchBlock,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let scrutinee = &match_block.scrutinee;
    let num_arms = match_block.arms.len();

    // Build the discriminant closure: move || match scrutinee { pat => 0, pat => 1, ... }
    let discriminant_arms: Vec<TokenStream2> = match_block
        .arms
        .iter()
        .enumerate()
        .map(|(i, arm)| {
            let pat = &arm.pattern;
            let idx = i;
            if let Some(ref guard) = arm.guard {
                quote! { #pat if #guard => #idx, }
            } else {
                quote! { #pat => #idx, }
            }
        })
        .collect();

    let discriminant = quote! {
        move || -> usize {
            #[allow(unreachable_patterns)]
            match #scrutinee {
                #(#discriminant_arms)*
                _ => #num_arms, // out-of-range = no branch rendered
            }
        }
    };

    // Build branch closures
    let branch_closures: Vec<TokenStream2> = match_block
        .arms
        .iter()
        .map(|arm| {
            let pat = &arm.pattern;
            let guard_check = arm.guard.as_ref().map(|g| quote! { if #g });
            let body = generate_children_body(&arm.children, ctx);

            // Each branch re-evaluates the scrutinee to bind pattern variables.
            // We use `match` with the specific pattern + a catch-all unreachable.
            quote! {
                Box::new(move |__child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    #[allow(unreachable_patterns, unused_variables, irrefutable_let_patterns)]
                    match #scrutinee {
                        #pat #guard_check => { #body }
                        _ => unreachable!()
                    }
                }) as Box<dyn Fn(&mut ::rinch::core::dom::RenderScope) -> ::rinch::core::dom::NodeHandle>
            }
        })
        .collect();

    quote! {
        {
            ::rinch::core::match_dom(
                __scope,
                &#parent_var,
                #discriminant,
                vec![#(#branch_closures),*]
            );
        }
    }
}
