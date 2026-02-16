//! Control flow DOM code generation.
//!
//! Handles `Fragment` and native `if`/`for`/`match` control flow in RSX.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::node::{RsxElseBranch, RsxForLoop, RsxIfBlock, RsxMatchBlock, RsxNode};

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
/// in a `display:contents` div. Leading `let` statements are emitted before
/// the RSX content.
fn generate_children_body(
    children: &[RsxNode],
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    // Partition into leading statements and trailing RSX nodes
    let mut statements = Vec::new();
    let mut rsx_children = Vec::new();
    let mut past_statements = false;

    for child in children {
        if !past_statements {
            if let RsxNode::Statement(stmt) = child {
                statements.push(stmt);
                continue;
            }
            past_statements = true;
        }
        rsx_children.push(child);
    }

    let stmt_code: Vec<TokenStream2> = statements.iter().map(|stmt| quote! { #stmt }).collect();

    if rsx_children.is_empty() {
        quote! {
            #(#stmt_code)*
            __scope.create_element("div")
        }
    } else if rsx_children.len() == 1 {
        // Single child — check if it's a control flow node that needs a parent
        match rsx_children[0] {
            RsxNode::IfBlock(_) | RsxNode::ForLoop(_) | RsxNode::MatchBlock(_) => {
                // Control flow nodes insert into a parent, so we need a wrapper
                let wrapper = ctx.next_var("cf_wrap");
                let child_code = super::generate_child_code(rsx_children[0], &wrapper, ctx);
                quote! {
                    {
                        #(#stmt_code)*
                        let #wrapper = __scope.create_element("div");
                        #wrapper.set_attribute("style", "display:contents");
                        #child_code
                        #wrapper
                    }
                }
            }
            _ => {
                let child_code = super::node_to_dom(rsx_children[0], ctx);
                quote! {
                    {
                        #(#stmt_code)*
                        #child_code
                    }
                }
            }
        }
    } else {
        let wrapper = ctx.next_var("branch_wrap");
        let children_code: Vec<TokenStream2> = rsx_children
            .iter()
            .map(|child| super::generate_child_code(child, &wrapper, ctx))
            .collect();
        quote! {
            {
                #(#stmt_code)*
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
                move |#pattern| ::std::string::ToString::to_string(&#key_fn),
                move |#pattern, __child_scope: &mut ::rinch::core::dom::RenderScope| -> ::rinch::core::dom::NodeHandle {
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
    // Look for key: prop on the first child element (skip leading let statements)
    let first_element = for_loop
        .children
        .iter()
        .find(|c| !matches!(c, RsxNode::Statement(_)));

    if let Some(RsxNode::Element(el)) = first_element
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
