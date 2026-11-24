//! A component's struct fields from its `rsx!` props (split out of
//! `component_codegen.rs`).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::helpers::get_closure_expr;
use crate::prop::RsxProp;

use super::DomCodegenContext;
use super::captures::{collect_capture_idents, is_move_closure, shadow_clones};
use super::handler_owner::component_handler_prop;

/// How a component's props are built.
#[derive(Clone, Copy)]
pub enum PropMode {
    /// Once, where the component is written.
    Once,
    /// On every render of a re-rendering component (one with a reactive
    /// struct prop), inside that render, where `__rinch_caller_owner` is bound.
    PerRender,
}

/// Generate field assignment tokens for component props.
///
/// In [`PropMode::PerRender`] (a re-rendering component), closure props are
/// invoked to get their current value (tracking signals), then wrapped like
/// static values. `on*` handler props are converted under the caller's owner
/// where they are the caller's (issues #3027, #3030; see `handler_owner`).
pub fn generate_component_field_assignments(
    comp_props: &[&RsxProp],
    mode: PropMode,
    ctx: &DomCodegenContext,
) -> Vec<TokenStream2> {
    let invoke_closures = matches!(mode, PropMode::PerRender);
    comp_props
        .iter()
        .map(|prop| {
            let name = prop.name_as_ident();
            let name_str = prop.name.clone();
            let value = &prop.value;

            if name_str == "oninput" || name_str.starts_with("on") {
                // Use IntoEventHandler trait so the field can be either T or Option<T>
                // (e.g., Callback, Option<Callback>, ValueCallback<T>, Option<ValueCallback<T>>).
                let handler = component_handler_prop(ctx, mode, value);
                quote! { #name: #handler }
            } else if name_str == "icon" || name_str.ends_with("_icon") {
                // #718: on the reactive path a closure must be INVOKED before
                // being wrapped in `Some(...)` — the icon field is
                // `Option<TablerIcon>`, never `Option<impl Fn() -> TablerIcon>`.
                // `has_reactive_component_props`/`has_reactive_props` route a
                // component with a closure-valued `icon`/`*_icon` prop onto the
                // reactive path (`invoke_closures == true`); the static path
                // (`invoke_closures == false`) is only reached when no comp
                // prop is a closure, so `value` here is never a closure there
                // and the plain `Some(#value)` wrap is unchanged.
                if invoke_closures {
                    if let Some(closure) = get_closure_expr(value) {
                        if is_move_closure(closure) {
                            let fire = shadow_clones(collect_capture_idents(closure).iter());
                            quote! { #name: Some({ #fire (#closure)() }) }
                        } else {
                            quote! { #name: Some((#closure)()) }
                        }
                    } else {
                        quote! { #name: Some(#value) }
                    }
                } else {
                    quote! { #name: Some(#value) }
                }
            } else if name_str.ends_with("_fn") {
                quote! { #name: Some(std::rc::Rc::new(#value)) }
            } else if crate::helpers::is_literal_bool(value)
                || crate::helpers::is_literal_int(value)
                || crate::helpers::is_literal_float(value)
            {
                // Literal bool/int/float fall through to `.into()` so the same expression
                // works whether the destination field is `T` or `Option<T>`:
                //   - `T` field: `From<T> for T` (trivial blanket) gives identity.
                //   - `Option<T>` field: `From<T> for Option<T>` in std gives `Some(v)`.
                // The destination field type drives inference, so unsuffixed literals
                // like `12.0` resolve correctly against the field type.
                quote! { #name: (#value).into() }
            } else if crate::helpers::is_literal_string(value) {
                quote! { #name: String::from(#value) }
            } else if crate::helpers::is_option_expr(value) {
                // Some(...) and None pass through directly to preserve
                // unsizing coercion (e.g. Some(Rc::new(|| ...)) for Option<Rc<dyn Fn()>>).
                quote! { #name: #value }
            } else if invoke_closures {
                if let Some(closure) = get_closure_expr(value) {
                    // Invoke the closure to get current value, using .into() so
                    // &str → String, bool → bool, etc. all work correctly.
                    // A `move` closure rebuilt here on every render would move
                    // what it names out of the render closure's captures, so it
                    // gets a fresh shadow per render (issue #223).
                    if is_move_closure(closure) {
                        let fire = shadow_clones(collect_capture_idents(closure).iter());
                        quote! { #name: ({ #fire ((#closure)()).into() }) }
                    } else {
                        quote! { #name: ((#closure)()).into() }
                    }
                } else {
                    // Use .into() so T auto-wraps into Option<T> via From<T>,
                    // while same-type assignments stay identity conversions.
                    quote! { #name: (#value).into() }
                }
            } else {
                // Use .into() so T auto-wraps into Option<T> via From<T>,
                // while same-type assignments stay identity conversions.
                quote! { #name: (#value).into() }
            }
        })
        .collect()
}
