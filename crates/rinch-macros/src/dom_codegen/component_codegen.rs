//! Component DOM code generation.
//!
//! Handles code generation for PascalCase components, including
//! static components and reactive components that re-render when signals change.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::helpers::{expand_style_shorthand, get_closure_expr, is_literal_expr};
use crate::prop::RsxProp;

use super::captures::{
    collect_body_captures, collect_capture_idents, is_move_closure, shadow_clones, wrap_site,
};
use super::html::{
    generate_class_code, generate_shorthand_code, generate_shorthand_code_reactive,
    generate_style_code,
};
use super::{DomCodegenContext, no_siblings};

/// Check if an element is a component with reactive (closure) props that needs
/// statement-based insertion (like control flow) rather than expression-based.
pub fn has_reactive_component_props(element: &RsxElement) -> bool {
    if !element.is_rinch_component() {
        return false;
    }
    element.props.iter().any(|p| {
        let name = p.name.to_string();
        if name == "key" || name == "style" || name == "class" {
            return false;
        }
        if name.starts_with("on") || name.ends_with("_fn") {
            return false;
        }
        if expand_style_shorthand(&name).is_some() {
            return false;
        }
        get_closure_expr(&p.value).is_some()
    })
}

/// Generate reactive component code as a statement that inserts directly into parent_var.
///
/// This is the preferred path when we know the parent (inside `generate_child_code`).
/// The component's marker and content are appended directly to the actual parent,
/// avoiding layout issues with wrapper divs.
pub fn generate_reactive_component_stmt(
    element: &RsxElement,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let comp_name = &element.name;

    // Separate style/class/shorthand props from component struct props
    let mut style_prop = None;
    let mut class_prop = None;
    let mut shorthand_props = Vec::new();
    let mut comp_props = Vec::new();

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if name_str == "key" {
            continue;
        } else if name_str == "style" {
            style_prop = Some(prop);
        } else if name_str == "class" {
            class_prop = Some(prop);
        } else if expand_style_shorthand(&name_str).is_some() {
            shorthand_props.push(prop);
        } else {
            comp_props.push(prop);
        }
    }

    let field_assignments: Vec<TokenStream2> =
        generate_component_field_assignments(&comp_props, true);

    let children_var = ctx.next_var("children");
    let temp_var = ctx.next_var("temp");
    let comp_var = ctx.next_var("comp");
    let result_var = ctx.next_var("result");

    // The render closure runs again on every prop change, so what it names is
    // captured from the body it was built in and moved out of it (issue #223).
    ctx.push_closure_frame(HashSet::new());
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &temp_var, ctx))
        .collect();
    ctx.pop_closure_frame();

    // Style/class inside the reactive closure use simple set (no separate effects needed)
    let style_code = if let Some(prop) = style_prop {
        let value = &prop.value;
        if is_literal_expr(value) {
            let value_str = crate::helpers::expr_to_string(value);
            quote! { #result_var.set_attribute("style", #value_str); }
        } else if let Some(closure) = get_closure_expr(value) {
            quote! { #result_var.set_attribute("style", &::std::string::ToString::to_string(&(#closure)())); }
        } else {
            quote! { #result_var.set_attribute("style", &::std::string::ToString::to_string(&#value)); }
        }
    } else {
        quote! {}
    };

    let class_code = if let Some(prop) = class_prop {
        let value = &prop.value;
        if is_literal_expr(value) {
            let value_str = crate::helpers::expr_to_string(value);
            quote! { #result_var.add_class(#value_str); }
        } else if let Some(closure) = get_closure_expr(value) {
            quote! {
                let __cls = ::std::string::ToString::to_string(&(#closure)());
                if !__cls.is_empty() { for __c in __cls.split_whitespace() { #result_var.add_class(__c); } }
            }
        } else {
            quote! {
                let __cls = ::std::string::ToString::to_string(&#value);
                if !__cls.is_empty() { for __c in __cls.split_whitespace() { #result_var.add_class(__c); } }
            }
        }
    } else {
        quote! {}
    };

    let shorthand_code = generate_shorthand_code_reactive(&shorthand_props, &result_var);

    // Pass the actual parent directly to reactive_component_dom — no wrapper div needed.
    // This is the statement path: no return value, the function handles insertion.
    let body = quote! {
        let __scope = __child_scope;

        #[allow(clippy::needless_update)]
        let #comp_var = #comp_name {
            #(#field_assignments,)*
            ..Default::default()
        };

        // The subtree render runs untracked (issue #390), the way `show_dom`,
        // `match_dom` and `for_each_dom` render their branches: a signal the
        // component body or a child reads while rendering must not subscribe
        // the re-render effect — nested control flow and `{|| expr}` closures
        // create their own effects for that, and a subscription here would
        // rebuild the whole subtree (resetting its component-local state) on
        // every inner change. The prop closures above and the
        // style/class/shorthand closures below stay in the tracked region:
        // their signal reads are what schedule a re-render.
        //
        // `untracked` pops exactly ONE observer, so a read inside it
        // subscribes the next observer down the stack. The leak worked
        // through that: a nested `match_dom`'s own untracked popped the
        // match effect and exposed this re-render effect underneath. The
        // invariant is that every effect-creating render boundary wraps its
        // user render in exactly one `untracked` — this wrap included — so
        // the stack stays balanced at any nesting depth.
        let #result_var = rinch::core::reactive::untracked(|| {
            let #temp_var = __scope.create_element("template");
            #(#children_code)*
            let #children_var: Vec<rinch::core::NodeHandle> = #temp_var.children();

            rinch::core::Component::render(&#comp_var, __scope, &#children_var)
        });
        #style_code
        #class_code
        #shorthand_code
        #result_var
    };
    let shadows = ctx.site_shadows(&collect_body_captures(&body), &no_siblings());
    let render = wrap_site(&shadows, quote! { move |__child_scope| { #body } });
    quote! {
        rinch::core::reactive_component_dom(__scope, &#parent_var, #render);
    }
}

/// Generate DOM code for a component (direct construction without Element::Component).
pub fn element_to_dom_component(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let comp_name = &element.name;

    // Separate style/class/shorthand props from component struct props.
    // style: and class: are applied to the rendered NodeHandle AFTER Component::render(),
    // not as fields on the component struct. Shorthands become set_style() calls.
    let mut style_prop = None;
    let mut class_prop = None;
    let mut shorthand_props = Vec::new();
    let mut comp_props = Vec::new();

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if name_str == "key" {
            // key: is handled by the parent for-loop, not the component struct
            continue;
        } else if name_str == "style" {
            style_prop = Some(prop);
        } else if name_str == "class" {
            class_prop = Some(prop);
        } else if expand_style_shorthand(&name_str).is_some() {
            shorthand_props.push(prop);
        } else {
            comp_props.push(prop);
        }
    }

    // Check if any non-event, non-style/class, non-_fn prop is a closure.
    // If so, we wrap the entire component in reactive_component_dom for re-rendering.
    // Reactive shorthand closures also trigger this.
    let has_reactive_props = comp_props.iter().any(|p| {
        let name = p.name.to_string();
        !name.starts_with("on") && !name.ends_with("_fn") && get_closure_expr(&p.value).is_some()
    });

    if has_reactive_props {
        return element_to_dom_component_reactive(
            element,
            ctx,
            &comp_props,
            &shorthand_props,
            style_prop,
            class_prop,
        );
    }

    // Static path: no reactive component props
    let comp_var = ctx.next_var("comp");
    let result_var = ctx.next_var("result");

    // Generate field assignments only for component props (not style/class)
    let field_assignments: Vec<TokenStream2> =
        generate_component_field_assignments(&comp_props, false);

    // Generate children rendering code - Show/For use marker-based insertion
    let children_var = ctx.next_var("children");
    let temp_var = ctx.next_var("temp");

    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &temp_var, ctx))
        .collect();

    // Generate post-render style/class/shorthand application code
    let style_code = generate_style_code(style_prop, &result_var, ctx);
    let class_code = generate_class_code(class_prop, &result_var, ctx);
    let shorthand_code = generate_shorthand_code(&shorthand_props, &result_var, ctx);

    quote! {
        {
            // Construct component
            #[allow(clippy::needless_update)]
            let #comp_var = #comp_name {
                #(#field_assignments,)*
                ..Default::default()
            };

            // Render children to NodeHandles
            let #temp_var = __scope.create_element("template");
            #(#children_code)*
            let #children_var: Vec<rinch::core::NodeHandle> = #temp_var.children();

            // Render component directly
            let #result_var = rinch::core::Component::render(&#comp_var, __scope, &#children_var);

            // Apply style/class/shorthand props to the rendered NodeHandle
            #style_code
            #class_code
            #shorthand_code

            #result_var
        }
    }
}

/// Generate DOM code for a component with reactive props (wrapped in reactive_component_dom).
///
/// When any component prop is a closure (e.g., `variant: {|| if active.get() { "filled" } else { "light" }}`),
/// the entire component is reconstructed whenever those signals change.
///
/// Uses a `display:contents` wrapper div as the parent for `reactive_component_dom`, so the
/// returned node can be appended to any parent by the caller. This avoids using
/// `__scope.parent()` which would misroute to the scope root (body).
pub fn element_to_dom_component_reactive(
    element: &RsxElement,
    ctx: &mut DomCodegenContext,
    comp_props: &[&RsxProp],
    shorthand_props: &[&RsxProp],
    style_prop: Option<&RsxProp>,
    class_prop: Option<&RsxProp>,
) -> TokenStream2 {
    let comp_name = &element.name;
    let wrapper_var = ctx.next_var("reactive_wrapper");

    // Inside the render closure, closure props are called (tracking signal deps),
    // and the result is used as a static value for the component struct.
    let field_assignments: Vec<TokenStream2> =
        generate_component_field_assignments(comp_props, true);

    // Generate children code - children are re-rendered each time too
    let children_var = ctx.next_var("children");
    let temp_var = ctx.next_var("temp");
    let comp_var = ctx.next_var("comp");
    let result_var = ctx.next_var("result");

    // The render closure runs again on every prop change, so what it names is
    // captured from the body it was built in and moved out of it (issue #223).
    ctx.push_closure_frame(HashSet::new());
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &temp_var, ctx))
        .collect();
    ctx.pop_closure_frame();

    // Style/class inside the reactive closure use simple set (no separate effects needed)
    let style_code = if let Some(prop) = style_prop {
        let value = &prop.value;
        if is_literal_expr(value) {
            let value_str = crate::helpers::expr_to_string(value);
            quote! { #result_var.set_attribute("style", #value_str); }
        } else if let Some(closure) = get_closure_expr(value) {
            quote! { #result_var.set_attribute("style", &::std::string::ToString::to_string(&(#closure)())); }
        } else {
            quote! { #result_var.set_attribute("style", &::std::string::ToString::to_string(&#value)); }
        }
    } else {
        quote! {}
    };

    let class_code = if let Some(prop) = class_prop {
        let value = &prop.value;
        if is_literal_expr(value) {
            let value_str = crate::helpers::expr_to_string(value);
            quote! { #result_var.add_class(#value_str); }
        } else if let Some(closure) = get_closure_expr(value) {
            quote! {
                let __cls = ::std::string::ToString::to_string(&(#closure)());
                if !__cls.is_empty() { for __c in __cls.split_whitespace() { #result_var.add_class(__c); } }
            }
        } else {
            quote! {
                let __cls = ::std::string::ToString::to_string(&#value);
                if !__cls.is_empty() { for __c in __cls.split_whitespace() { #result_var.add_class(__c); } }
            }
        }
    } else {
        quote! {}
    };

    // Shorthands inside reactive closure invoke closures directly (no separate effects)
    let shorthand_code = generate_shorthand_code_reactive(shorthand_props, &result_var);

    // Use a display:contents wrapper div as the parent for reactive_component_dom.
    // This ensures the component content is placed inside the wrapper, which the caller
    // then appends to the actual parent — avoiding __scope.parent() misrouting.
    let body = quote! {
        let __scope = __child_scope;

        #[allow(clippy::needless_update)]
        let #comp_var = #comp_name {
            #(#field_assignments,)*
            ..Default::default()
        };

        // The subtree render runs untracked (issue #390), the way `show_dom`,
        // `match_dom` and `for_each_dom` render their branches: a signal the
        // component body or a child reads while rendering must not subscribe
        // the re-render effect — nested control flow and `{|| expr}` closures
        // create their own effects for that, and a subscription here would
        // rebuild the whole subtree (resetting its component-local state) on
        // every inner change. The prop closures above and the
        // style/class/shorthand closures below stay in the tracked region:
        // their signal reads are what schedule a re-render.
        //
        // `untracked` pops exactly ONE observer, so a read inside it
        // subscribes the next observer down the stack. The leak worked
        // through that: a nested `match_dom`'s own untracked popped the
        // match effect and exposed this re-render effect underneath. The
        // invariant is that every effect-creating render boundary wraps its
        // user render in exactly one `untracked` — this wrap included — so
        // the stack stays balanced at any nesting depth.
        let #result_var = rinch::core::reactive::untracked(|| {
            let #temp_var = __scope.create_element("template");
            #(#children_code)*
            let #children_var: Vec<rinch::core::NodeHandle> = #temp_var.children();

            rinch::core::Component::render(&#comp_var, __scope, &#children_var)
        });
        #style_code
        #class_code
        #shorthand_code
        #result_var
    };
    let shadows = ctx.site_shadows(&collect_body_captures(&body), &no_siblings());
    let render = wrap_site(&shadows, quote! { move |__child_scope| { #body } });
    quote! {
        {
            let #wrapper_var = __scope.create_element("div");
            #wrapper_var.set_attribute("style", "display:contents");
            rinch::core::reactive_component_dom(__scope, &#wrapper_var, #render);
            #wrapper_var
        }
    }
}

/// Generate field assignment tokens for component props.
///
/// When `invoke_closures` is true (reactive mode), closure props are invoked to get
/// their current value (tracking signals), then wrapped like static values.
pub fn generate_component_field_assignments(
    comp_props: &[&RsxProp],
    invoke_closures: bool,
) -> Vec<TokenStream2> {
    comp_props
        .iter()
        .map(|prop| {
            let name = prop.name_as_ident();
            let name_str = prop.name.clone();
            let value = &prop.value;

            if name_str == "oninput" || name_str.starts_with("on") {
                // Use IntoEventHandler trait so the field can be either T or Option<T>
                // (e.g., Callback, Option<Callback>, ValueCallback<T>, Option<ValueCallback<T>>).
                quote! { #name: IntoEventHandler::into_event_handler(#value) }
            } else if name_str == "icon" || name_str.ends_with("_icon") {
                quote! { #name: Some(#value) }
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
