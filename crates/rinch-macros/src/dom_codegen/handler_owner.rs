//! Which owner a handler written in `rsx!` runs under (issues #3027, #3030).
//!
//! A re-rendering component — one with a reactive struct prop — is rebuilt,
//! struct and children, inside one render's scope, which the next render
//! disposes. The handlers the **caller** wrote there are not that render's: its
//! `on*` props, and the element handlers and component props among its
//! children down to the next `for`/`if`/`match` body (the caller-owned region,
//! `DomCodegenContext::caller_region`). Only the macro knows which code that
//! is, so it wraps each such handler in `rinch_core::reactive::CallerHandler`,
//! which runs it under `__rinch_caller_owner` (the caller's owner, bound by the
//! render body) — unless its expression created something under the render
//! while it was evaluated (`CallerHandler::begin` takes the render owner's
//! footprint first): then it holds what that render made, and stays the
//! render's. (`let` is accepted only in control-flow bodies, which end the
//! region, so no name bound among the region's children can reach a handler.)
//!
//! A `for` row, an `if` branch and a `match` arm are each the creator of what
//! is written inside them, so a handler there runs under the row's owner, its
//! registration owner, as it always has.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::DomCodegenContext;
use super::component_props::PropMode;

/// An element handler as it is registered: `CallerHandler::<method>(handler)`
/// when it is the caller's, otherwise `handler` as written. `method` names the
/// handler's argument shape (`plain`, `input`, `scroll`, `key`, `wheel`,
/// `file_drop`, `files_drop`).
pub(super) fn element_handler(
    ctx: &DomCodegenContext,
    method: &str,
    handler: &syn::Expr,
) -> TokenStream2 {
    if !ctx.in_caller_region() {
        return quote! { #handler };
    }
    let method = syn::Ident::new(method, proc_macro2::Span::call_site());
    quote! {
        rinch::core::reactive::CallerHandler::begin(&__rinch_caller_owner).#method(#handler)
    }
}

/// A component's `on*` prop value, converted to the field's callback type
/// (`IntoEventHandler`), under the caller's owner when it is the caller's.
///
/// Only the conversion runs under that owner (it is where the callback
/// captures it); the value is evaluated where it is written, so what it
/// creates is not handed to the caller on every render.
pub(super) fn component_handler_prop(
    ctx: &DomCodegenContext,
    mode: PropMode,
    value: &syn::Expr,
) -> TokenStream2 {
    let callers = match mode {
        // A re-rendering component's own props, built in its render.
        PropMode::PerRender => true,
        // Built once where written: the caller's only inside a region.
        PropMode::Once => ctx.in_caller_region(),
    };
    if callers {
        quote! { rinch::core::reactive::CallerHandler::begin(&__rinch_caller_owner).prop(#value) }
    } else {
        quote! { IntoEventHandler::into_event_handler(#value) }
    }
}

#[cfg(test)]
mod tests {
    use crate::dom_codegen::{DomCodegenContext, element_to_dom};
    use crate::element::RsxElement;

    fn codegen(src: &str) -> String {
        let element: RsxElement = syn::parse_str(src).expect("parses");
        element_to_dom(&element, &mut DomCodegenContext::new()).to_string()
    }

    fn wrapped(out: &str) -> usize {
        out.matches("CallerHandler :: begin").count()
    }

    #[test]
    fn outside_a_rerendering_component_nothing_is_wrapped() {
        let out = codegen("div { button { onclick: || {} } Button { onclick: || {} } }");
        assert_eq!(wrapped(&out), 0, "{out}");
    }

    #[test]
    fn a_rerendering_components_props_and_children_are_the_callers() {
        let out = codegen(
            "Paper { shadow: {|| s()}, onclick: || {}, \
               button { onclick: || {}, oninput: |_| {}, onkeydown: |_| false } \
               Button { onclick: || {} } }",
        );
        assert_eq!(wrapped(&out), 5, "{out}");
        for method in [". plain (", ". input (", ". key (", ". prop ("] {
            assert!(out.contains(method), "{method}: {out}");
        }
    }

    #[test]
    fn a_nested_rerendering_component_is_placed_for_the_outer_caller() {
        let out = codegen("Paper { shadow: {|| s()}, Paper { shadow: {|| s()}, \"x\" } }");
        assert_eq!(
            out.matches("reactive_component_dom_for_caller").count(),
            1,
            "{out}"
        );
    }

    #[test]
    fn a_control_flow_body_ends_the_region() {
        for body in [
            "for i in v() { button { key: i, onclick: || {} } }",
            "if c() { button { onclick: || {} } }",
            "match m() { _ => { button { onclick: || {} } } }",
        ] {
            let out = codegen(&format!("Paper {{ shadow: {{|| s()}}, {body} }}"));
            assert_eq!(wrapped(&out), 0, "{body}: {out}");
            assert!(!out.contains("reactive_component_dom_for_caller"), "{out}");
        }
    }
}
