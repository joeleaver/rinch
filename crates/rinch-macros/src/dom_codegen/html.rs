//! HTML element DOM code generation.
//!
//! Handles code generation for standard HTML elements (`div`, `p`, `input`, etc.)
//! including attribute binding, event handlers, style shorthands, and children.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::helpers::{
    expand_style_shorthand, get_closure_expr, is_event_prop, is_literal_expr, is_void_element,
    resolve_spacing_value,
};
use crate::prop::RsxProp;

use super::DomCodegenContext;

/// Generate DOM code for an HTML element.
pub fn element_to_dom_html(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let tag = element.name.to_string();
    let elem_var = ctx.next_var("elem");

    // Separate event handlers, shorthands, and regular attributes
    let mut event_props = Vec::new();
    let mut shorthand_props = Vec::new();
    let mut attr_props = Vec::new();

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if name_str == "key" {
            // Skip key prop — used by for-loop reconciliation, not a DOM attribute
            continue;
        } else if is_event_prop(&name_str) {
            event_props.push(prop);
        } else if expand_style_shorthand(&name_str).is_some() {
            shorthand_props.push(prop);
        } else {
            attr_props.push(prop);
        }
    }

    // Generate attribute setting code
    let attr_code: Vec<TokenStream2> = attr_props
        .iter()
        .map(|prop| {
            let name = prop.name.to_string();
            let value = &prop.value;

            if is_literal_expr(value) {
                // Static attribute - set once
                let value_str = crate::helpers::expr_to_string(value);
                quote! {
                    #elem_var.set_attribute(#name, #value_str);
                }
            } else if let Some(closure) = get_closure_expr(value) {
                // Closure expression - call it and use result
                let handle_var = ctx.next_var("attr_handle");
                quote! {
                    {
                        let #handle_var = #elem_var.clone();
                        __scope.create_effect(move || {
                            #handle_var.set_attribute(#name, &::std::string::ToString::to_string(&(#closure)()));
                        });
                    }
                }
            } else {
                // Dynamic expression (not a closure) - wrap in effect
                let handle_var = ctx.next_var("attr_handle");
                quote! {
                    {
                        let #handle_var = #elem_var.clone();
                        __scope.create_effect(move || {
                            #handle_var.set_attribute(#name, &::std::string::ToString::to_string(&#value));
                        });
                    }
                }
            }
        })
        .collect();

    // Generate event handler registration
    let event_code: Vec<TokenStream2> = event_props
        .iter()
        .map(|prop| {
            let handler = &prop.value;
            let event_name = prop.name.to_string();
            if event_name == "oninput" || event_name == "onchange" {
                // Input events use register_input_handler with Fn(String)
                quote! {
                    {
                        let __handler_id = __scope.register_input_handler(#handler);
                        #elem_var.set_attribute("data-oninput", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onscroll" {
                // Scroll events use register_scroll_handler with Fn(f64)
                quote! {
                    {
                        let __handler_id = __scope.register_scroll_handler(#handler);
                        #elem_var.set_attribute("data-onscroll", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onfiledrop" {
                // OS file-drop events use register_file_drop_handler with Fn(Vec<PathBuf>)
                quote! {
                    {
                        let __handler_id = __scope.register_file_drop_handler(#handler);
                        #elem_var.set_attribute("data-onfiledrop", &__handler_id.0.to_string());
                    }
                }
            } else {
                // Drag-and-drop and context menu events get their own data-
                // attributes so the runtime can distinguish them during
                // walk-up dispatch.
                let data_attr = match event_name.as_str() {
                    "ondragstart" => "data-ondragstart",
                    "ondragmove" => "data-ondragmove",
                    "ondragend" => "data-ondragend",
                    "ondrop" => "data-ondrop",
                    "ondragenter" => "data-ondragenter",
                    "ondragover" => "data-ondragover",
                    "ondragleave" => "data-ondragleave",
                    "onfiledragenter" => "data-onfiledragenter",
                    "onfiledragleave" => "data-onfiledragleave",
                    "oncontextmenu" => "data-oncontextmenu",
                    "onmouseenter" => "data-onenter",
                    "onmouseleave" => "data-onleave",
                    "onmousedown" => "data-onmousedown",
                    "onmouseup" => "data-onmouseup",
                    "onmousemove" => "data-onmousemove",
                    _ => "data-rid", // onclick and all other events
                };
                quote! {
                    {
                        let __handler_id = rinch::core::register_handler(std::rc::Rc::new(#handler));
                        #elem_var.set_attribute(#data_attr, &__handler_id.0.to_string());
                    }
                }
            }
        })
        .collect();

    // Generate shorthand style code (e.g., p: "md" -> set_style("padding", ...))
    let shorthand_code = generate_shorthand_code(&shorthand_props, &elem_var, ctx);

    // Generate children - Show/For use marker-based insertion directly into parent
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &elem_var, ctx))
        .collect();

    // Check if void element
    if is_void_element(&tag) {
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#attr_code)*
                #(#event_code)*
                #shorthand_code
                #elem_var
            }
        }
    } else if tag == "select" {
        // A `<select>`'s `value` is a live property that only takes effect once
        // its `<option>` children exist, and the reactive `value:` effect runs at
        // creation — so append the children BEFORE the attribute effects, else the
        // initial selected value is dropped on the web backend (issue #100).
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#children_code)*
                #(#attr_code)*
                #(#event_code)*
                #shorthand_code
                #elem_var
            }
        }
    } else {
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#attr_code)*
                #(#event_code)*
                #shorthand_code
                #(#children_code)*
                #elem_var
            }
        }
    }
}

/// Generate `set_style()` calls for style shorthand props.
///
/// Handles both static values (compile-time spacing resolution) and reactive
/// closures (runtime `resolve_spacing()`). Used by HTML, component, and reactive
/// component codegen paths.
pub fn generate_shorthand_code(
    shorthand_props: &[&RsxProp],
    result_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let code: Vec<TokenStream2> = shorthand_props
        .iter()
        .flat_map(|prop| {
            let name_str = prop.name.to_string();
            let css_props = expand_style_shorthand(&name_str).unwrap();
            let value = &prop.value;

            css_props
                .iter()
                .map(|css_prop| {
                    if is_literal_expr(value) {
                        // Static value - resolve spacing at compile time
                        let raw_value = crate::helpers::expr_to_string(value);
                        let resolved = resolve_spacing_value(&raw_value);
                        quote! {
                            #result_var.set_style(#css_prop, #resolved);
                        }
                    } else if let Some(closure) = get_closure_expr(value) {
                        // Reactive closure - use runtime resolve_spacing
                        let handle_var = ctx.next_var("sh_handle");
                        quote! {
                            {
                                let #handle_var = #result_var.clone();
                                __scope.create_effect(move || {
                                    let __val = ::std::string::ToString::to_string(&(#closure)());
                                    let __resolved = rinch::core::resolve_spacing(&__val);
                                    #handle_var.set_style(#css_prop, &__resolved);
                                });
                            }
                        }
                    } else {
                        // Dynamic expression - evaluate once with runtime resolve
                        let resolved_val = crate::helpers::expr_to_string(value);
                        let resolved = resolve_spacing_value(&resolved_val);
                        quote! {
                            #result_var.set_style(#css_prop, #resolved);
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    quote! { #(#code)* }
}

/// Generate `set_style()` calls for shorthand props inside a reactive component closure.
///
/// Similar to `generate_shorthand_code` but closures are invoked directly (tracking
/// signals) rather than creating separate effects, since the entire component re-renders.
pub fn generate_shorthand_code_reactive(
    shorthand_props: &[&RsxProp],
    result_var: &syn::Ident,
) -> TokenStream2 {
    let code: Vec<TokenStream2> = shorthand_props
        .iter()
        .flat_map(|prop| {
            let name_str = prop.name.to_string();
            let css_props = expand_style_shorthand(&name_str).unwrap();
            let value = &prop.value;

            css_props
                .iter()
                .map(|css_prop| {
                    if is_literal_expr(value) {
                        let raw_value = crate::helpers::expr_to_string(value);
                        let resolved = resolve_spacing_value(&raw_value);
                        quote! {
                            #result_var.set_style(#css_prop, #resolved);
                        }
                    } else if let Some(closure) = get_closure_expr(value) {
                        // Inside reactive component, invoke closure directly (tracks signals)
                        quote! {
                            {
                                let __val = ::std::string::ToString::to_string(&(#closure)());
                                let __resolved = rinch::core::resolve_spacing(&__val);
                                #result_var.set_style(#css_prop, &__resolved);
                            }
                        }
                    } else {
                        let resolved_val = crate::helpers::expr_to_string(value);
                        let resolved = resolve_spacing_value(&resolved_val);
                        quote! {
                            #result_var.set_style(#css_prop, #resolved);
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    quote! { #(#code)* }
}

/// Generate post-render style application code for a component.
pub fn generate_style_code(
    style_prop: Option<&RsxProp>,
    result_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let Some(prop) = style_prop else {
        return quote! {};
    };
    let value = &prop.value;
    if is_literal_expr(value) {
        let value_str = crate::helpers::expr_to_string(value);
        quote! {
            #result_var.set_attribute("style", #value_str);
        }
    } else if let Some(closure) = get_closure_expr(value) {
        let handle_var = ctx.next_var("style_handle");
        quote! {
            {
                let #handle_var = #result_var.clone();
                __scope.create_effect(move || {
                    #handle_var.set_attribute("style", &::std::string::ToString::to_string(&(#closure)()));
                });
            }
        }
    } else {
        let handle_var = ctx.next_var("style_handle");
        quote! {
            {
                let #handle_var = #result_var.clone();
                __scope.create_effect(move || {
                    #handle_var.set_attribute("style", &::std::string::ToString::to_string(&#value));
                });
            }
        }
    }
}

/// Generate post-render class application code for a component.
pub fn generate_class_code(
    class_prop: Option<&RsxProp>,
    result_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let Some(prop) = class_prop else {
        return quote! {};
    };
    let value = &prop.value;
    if is_literal_expr(value) {
        let value_str = crate::helpers::expr_to_string(value);
        quote! {
            #result_var.add_class(#value_str);
        }
    } else if let Some(closure) = get_closure_expr(value) {
        let handle_var = ctx.next_var("class_handle");
        let prev_var = ctx.next_var("prev_class");
        quote! {
            {
                let #handle_var = #result_var.clone();
                let #prev_var = ::std::cell::RefCell::new(String::new());
                __scope.create_effect(move || {
                    let __old = #prev_var.borrow().clone();
                    if !__old.is_empty() {
                        for __c in __old.split_whitespace() {
                            #handle_var.remove_class(__c);
                        }
                    }
                    let __new_class = ::std::string::ToString::to_string(&(#closure)());
                    if !__new_class.is_empty() {
                        for __c in __new_class.split_whitespace() {
                            #handle_var.add_class(__c);
                        }
                    }
                    *#prev_var.borrow_mut() = __new_class;
                });
            }
        }
    } else {
        let handle_var = ctx.next_var("class_handle");
        let prev_var = ctx.next_var("prev_class");
        quote! {
            {
                let #handle_var = #result_var.clone();
                let #prev_var = ::std::cell::RefCell::new(String::new());
                __scope.create_effect(move || {
                    let __old = #prev_var.borrow().clone();
                    if !__old.is_empty() {
                        for __c in __old.split_whitespace() {
                            #handle_var.remove_class(__c);
                        }
                    }
                    let __new_class = ::std::string::ToString::to_string(&#value);
                    if !__new_class.is_empty() {
                        for __c in __new_class.split_whitespace() {
                            #handle_var.add_class(__c);
                        }
                    }
                    *#prev_var.borrow_mut() = __new_class;
                });
            }
        }
    }
}
