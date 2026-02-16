//! Widget DOM code generation.
//!
//! Handles code generation for PascalCase widget components, including
//! static widgets and reactive widgets that re-render when signals change.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::helpers::{expand_style_shorthand, get_closure_expr, is_literal_expr};
use crate::prop::RsxProp;

use super::html::{
    generate_class_code, generate_shorthand_code, generate_shorthand_code_reactive,
    generate_style_code,
};
use super::DomCodegenContext;

/// Generate DOM code for a widget (direct construction without Element::Widget).
pub fn element_to_dom_widget(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let widget_name = &element.name;

    // Separate style/class/shorthand props from widget struct props.
    // style: and class: are applied to the rendered NodeHandle AFTER Widget::render(),
    // not as fields on the widget struct. Shorthands become set_style() calls.
    let mut style_prop = None;
    let mut class_prop = None;
    let mut shorthand_props = Vec::new();
    let mut widget_props = Vec::new();

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if name_str == "style" {
            style_prop = Some(prop);
        } else if name_str == "class" {
            class_prop = Some(prop);
        } else if expand_style_shorthand(&name_str).is_some() {
            shorthand_props.push(prop);
        } else {
            widget_props.push(prop);
        }
    }

    // Check if any non-event, non-style/class, non-_fn prop is a closure.
    // If so, we wrap the entire widget in reactive_widget_dom for re-rendering.
    // Reactive shorthand closures also trigger this.
    let has_reactive_props = widget_props.iter().any(|p| {
        let name = p.name.to_string();
        !name.starts_with("on") && !name.ends_with("_fn") && get_closure_expr(&p.value).is_some()
    });

    if has_reactive_props {
        return element_to_dom_widget_reactive(
            element,
            ctx,
            &widget_props,
            &shorthand_props,
            style_prop,
            class_prop,
        );
    }

    // Static path: no reactive widget props
    let widget_var = ctx.next_var("widget");
    let result_var = ctx.next_var("result");

    // Generate field assignments only for widget props (not style/class)
    let field_assignments: Vec<TokenStream2> =
        generate_widget_field_assignments(&widget_props, false);

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
            // Construct widget
            #[allow(clippy::needless_update)]
            let #widget_var = #widget_name {
                #(#field_assignments,)*
                ..Default::default()
            };

            // Render children to NodeHandles
            let #temp_var = __scope.create_element("template");
            #(#children_code)*
            let #children_var: Vec<::rinch::core::NodeHandle> = #temp_var.children();

            // Render widget directly
            let #result_var = ::rinch::core::Widget::render(&#widget_var, __scope, &#children_var);

            // Apply style/class/shorthand props to the rendered NodeHandle
            #style_code
            #class_code
            #shorthand_code

            #result_var
        }
    }
}

/// Generate DOM code for a widget with reactive props (wrapped in reactive_widget_dom).
///
/// When any widget prop is a closure (e.g., `variant: {|| if active.get() { "filled" } else { "light" }}`),
/// the entire widget is reconstructed whenever those signals change.
pub fn element_to_dom_widget_reactive(
    element: &RsxElement,
    ctx: &mut DomCodegenContext,
    widget_props: &[&RsxProp],
    shorthand_props: &[&RsxProp],
    style_prop: Option<&RsxProp>,
    class_prop: Option<&RsxProp>,
) -> TokenStream2 {
    let widget_name = &element.name;
    let wrapper_var = ctx.next_var("reactive_wrapper");

    // Inside the render closure, closure props are called (tracking signal deps),
    // and the result is used as a static value for the widget struct.
    let field_assignments: Vec<TokenStream2> =
        generate_widget_field_assignments(widget_props, true);

    // Generate children code - children are re-rendered each time too
    let children_var = ctx.next_var("children");
    let temp_var = ctx.next_var("temp");
    let widget_var = ctx.next_var("widget");
    let result_var = ctx.next_var("result");

    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &temp_var, ctx))
        .collect();

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

    quote! {
        {
            let #wrapper_var = __scope.parent();
            ::rinch::core::reactive_widget_dom(__scope, &#wrapper_var, move |__child_scope| {
                let __scope = __child_scope;

                #[allow(clippy::needless_update)]
                let #widget_var = #widget_name {
                    #(#field_assignments,)*
                    ..Default::default()
                };

                let #temp_var = __scope.create_element("template");
                #(#children_code)*
                let #children_var: Vec<::rinch::core::NodeHandle> = #temp_var.children();

                let #result_var = ::rinch::core::Widget::render(&#widget_var, __scope, &#children_var);
                #style_code
                #class_code
                #shorthand_code
                #result_var
            })
        }
    }
}

/// Generate field assignment tokens for widget props.
///
/// When `invoke_closures` is true (reactive mode), closure props are invoked to get
/// their current value (tracking signals), then wrapped like static values.
pub fn generate_widget_field_assignments(
    widget_props: &[&RsxProp],
    invoke_closures: bool,
) -> Vec<TokenStream2> {
    widget_props
        .iter()
        .map(|prop| {
            let name = &prop.name;
            let name_str = prop.name.to_string();
            let value = &prop.value;

            if name_str == "oninput" {
                quote! { #name: Some(InputCallback::new(#value)) }
            } else if name_str.starts_with("on") {
                quote! { #name: Some((#value).into()) }
            } else if name_str == "icon" || name_str.ends_with("_icon") {
                quote! { #name: Some(#value) }
            } else if name_str.ends_with("_fn") {
                quote! { #name: Some(std::rc::Rc::new(#value)) }
            } else if crate::helpers::is_literal_bool(value) {
                quote! { #name: #value }
            } else if crate::helpers::is_literal_int(value) {
                quote! { #name: Some(#value) }
            } else if crate::helpers::is_literal_string(value) {
                quote! { #name: Some(String::from(#value)) }
            } else if invoke_closures {
                if let Some(closure) = get_closure_expr(value) {
                    // Invoke the closure to get current value, wrap as Option<String>
                    quote! { #name: Some(String::from(::std::string::ToString::to_string(&(#closure)()))) }
                } else {
                    quote! { #name: #value }
                }
            } else {
                quote! { #name: #value }
            }
        })
        .collect()
}
