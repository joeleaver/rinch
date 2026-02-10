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

use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::Expr;

use crate::element::RsxElement;
use crate::helpers::{get_closure_expr, is_event_prop, is_literal_expr, is_void_element};
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
    fn next_var(&mut self, prefix: &str) -> syn::Ident {
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
fn generate_child_code(
    child: &RsxNode,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    match child {
        RsxNode::Element(el) if el.name == "Show" => {
            // Show inserts marker + content directly into parent
            element_to_dom_show(el, ctx, parent_var)
        }
        RsxNode::Element(el) if el.name == "For" => {
            // For inserts marker + items directly into parent
            element_to_dom_for(el, ctx, parent_var)
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
        let inner = element_to_dom_show(element, ctx, &wrapper_var);
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
        let inner = element_to_dom_for(element, ctx, &wrapper_var);
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
        return element_to_dom_fragment(element, ctx);
    }

    // Special handling for ThemeProvider - wraps children with reactive theme
    if name == "ThemeProvider" {
        return element_to_dom_theme_provider(element, ctx);
    }

    // Check if this is a rinch component (handled differently)
    if element.is_rinch_component() {
        return element_to_dom_component(element, ctx);
    }

    // Generate HTML element
    element_to_dom_html(element, ctx)
}

/// Generate DOM code for a Fragment (just renders children in an invisible wrapper).
fn element_to_dom_fragment(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let container_var = ctx.next_var("fragment");

    // Generate children - Show/For use marker-based insertion
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| generate_child_code(child, &container_var, ctx))
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

/// Generate DOM code for ThemeProvider (reactive theme wrapper).
///
/// ThemeProvider accepts reactive props for theme values and creates an Effect
/// that updates the theme CSS when those values change.
fn element_to_dom_theme_provider(
    element: &RsxElement,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let container_var = ctx.next_var("theme_container");

    // Find reactive props
    let primary_color_fn = element.props.iter().find(|p| p.name == "primary_color_fn");
    let dark_mode_fn = element.props.iter().find(|p| p.name == "dark_mode_fn");
    let default_radius = element.props.iter().find(|p| p.name == "default_radius");

    // Generate Effect for theme updates
    let effect_code = if primary_color_fn.is_some() || dark_mode_fn.is_some() {
        let pc_expr = primary_color_fn.map(|p| &p.value);
        let dm_expr = dark_mode_fn.map(|p| &p.value);
        let radius_expr = default_radius.map(|p| &p.value);

        let pc_let = if let Some(expr) = pc_expr {
            quote! { let __pc_fn: ::std::rc::Rc<dyn Fn() -> &'static str> = #expr; }
        } else {
            quote! { let __pc_fn: ::std::rc::Rc<dyn Fn() -> &'static str> = ::std::rc::Rc::new(|| "blue"); }
        };

        let dm_let = if let Some(expr) = dm_expr {
            quote! { let __dm_fn: ::std::rc::Rc<dyn Fn() -> bool> = #expr; }
        } else {
            quote! { let __dm_fn: ::std::rc::Rc<dyn Fn() -> bool> = ::std::rc::Rc::new(|| false); }
        };

        let radius_value = if let Some(expr) = radius_expr {
            quote! { Some((#expr).to_string()) }
        } else {
            quote! { Some("md".to_string()) }
        };

        quote! {
            {
                #pc_let
                #dm_let
                ::rinch::core::Effect::new(move || {
                    let __color = (__pc_fn)();
                    let __dark = (__dm_fn)();
                    ::rinch::fine_grained::update_theme(&::rinch::core::element::ThemeProviderProps {
                        primary_color: Some(__color.to_string()),
                        dark_mode: __dark,
                        default_radius: #radius_value,
                        ..Default::default()
                    });
                });
            }
        }
    } else {
        quote! {}
    };

    // Generate children - Show/For use marker-based insertion
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| generate_child_code(child, &container_var, ctx))
        .collect();

    // Use a div with flex column layout to fill parent and pass size to children
    quote! {
        {
            #effect_code
            let #container_var = __scope.create_element("div");
            #container_var.set_attribute("data-theme-provider", "true");
            #container_var.set_attribute("style", "display: flex; flex-direction: column; width: 100%; height: 100%");
            #(#children_code)*
            #container_var
        }
    }
}

/// Generate DOM code for an HTML element.
fn element_to_dom_html(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let tag = element.name.to_string();
    let elem_var = ctx.next_var("elem");

    // Separate event handlers from regular attributes
    let (event_props, attr_props): (Vec<_>, Vec<_>) = element
        .props
        .iter()
        .partition(|p| is_event_prop(&p.name.to_string()));

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
            } else {
                // Click and other events use register_handler with Fn()
                quote! {
                    {
                        let __handler_id = ::rinch::core::register_handler(std::rc::Rc::new(#handler));
                        #elem_var.set_attribute("data-rid", &__handler_id.0.to_string());
                    }
                }
            }
        })
        .collect();

    // Generate children - Show/For use marker-based insertion directly into parent
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| generate_child_code(child, &elem_var, ctx))
        .collect();

    // Check if void element
    if is_void_element(&tag) {
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#attr_code)*
                #(#event_code)*
                #elem_var
            }
        }
    } else {
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#attr_code)*
                #(#event_code)*
                #(#children_code)*
                #elem_var
            }
        }
    }
}

/// Generate DOM code for Show component (conditional rendering).
///
/// Generates a call to `show_dom()` which inserts a comment marker and content
/// directly into the parent element. No wrapper span is created.
fn element_to_dom_show(
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
            .map(|child| node_to_dom(child, ctx))
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
fn element_to_dom_for(
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
        RsxNode::Expr(expr) => {
            // Check if it's a closure expression
            if let Some(closure) = get_closure_expr(expr) {
                closure.clone()
            } else {
                // Not a closure, use the expression as-is
                expr.clone()
            }
        }
        _ => {
            return quote_spanned! {span=>
                compile_error!("For component view must be a closure: |item| rsx! { ... }")
            };
        }
    };

    // Generate call to for_each_dom - inserts marker + items into parent directly
    // The view function is constructed inside the callback so it captures the child scope
    // Use quote_spanned to point errors at the For keyword
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

/// Generate DOM code for a rinch component.
///
/// For widgets (PascalCase components), generates direct widget construction and render call.
/// Shell elements (Window, Menu, etc.) are NOT supported inside rsx! - use run() for windows
/// and the menu API for menus.
fn element_to_dom_component(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let name = element.name.to_string();

    // Shell elements should not be used inside rsx! - they're handled at runtime level
    let is_shell_element = matches!(
        name.as_str(),
        "Window" | "AppMenu" | "Menu" | "MenuItem" | "MenuSeparator" | "Portal"
    );

    if is_shell_element {
        let error_msg = format!(
            "Shell element `{}` cannot be used inside rsx!. Use run() for windows and the menu API for menus.",
            name
        );
        return quote! {
            compile_error!(#error_msg)
        };
    }

    // For widgets, generate direct construction and render call
    element_to_dom_widget(element, ctx)
}

/// Generate DOM code for a widget (direct construction without Element::Widget).
fn element_to_dom_widget(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let widget_name = &element.name;
    let widget_var = ctx.next_var("widget");
    let result_var = ctx.next_var("result");

    // Separate style/class props from widget struct props.
    // style: and class: are applied to the rendered NodeHandle AFTER Widget::render(),
    // not as fields on the widget struct.
    let mut style_prop = None;
    let mut class_prop = None;
    let mut widget_props = Vec::new();

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if name_str == "style" {
            style_prop = Some(prop);
        } else if name_str == "class" {
            class_prop = Some(prop);
        } else {
            widget_props.push(prop);
        }
    }

    // Generate field assignments only for widget props (not style/class)
    let field_assignments: Vec<TokenStream2> = widget_props
        .iter()
        .map(|prop| {
            let name = &prop.name;
            let name_str = prop.name.to_string();
            let value = &prop.value;

            // Handle different prop types
            if name_str == "oninput" {
                quote! { #name: Some(InputCallback::new(#value)) }
            } else if name_str.starts_with("on") {
                quote! { #name: Some((#value).into()) }
            } else if name_str == "icon" || name_str.ends_with("_icon") {
                quote! { #name: Some(#value) }
            } else if name_str.ends_with("_fn") {
                // Auto-wrap _fn reactive props: closure → Some(Rc::new(closure))
                // Rust's type coercion handles Rc<closure> → Rc<dyn Fn() -> T>
                quote! { #name: Some(std::rc::Rc::new(#value)) }
            } else if crate::helpers::is_literal_bool(value) {
                quote! { #name: #value }
            } else if crate::helpers::is_literal_int(value) {
                quote! { #name: Some(#value) }
            } else if crate::helpers::is_literal_string(value) {
                quote! { #name: Some(String::from(#value)) }
            } else {
                quote! { #name: #value }
            }
        })
        .collect();

    // Generate children rendering code - Show/For use marker-based insertion
    let children_var = ctx.next_var("children");
    let temp_var = ctx.next_var("temp");

    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| generate_child_code(child, &temp_var, ctx))
        .collect();

    // Generate post-render style application code
    let style_code = if let Some(prop) = style_prop {
        let value = &prop.value;
        if is_literal_expr(value) {
            // Static style string - set once
            let value_str = crate::helpers::expr_to_string(value);
            quote! {
                #result_var.set_attribute("style", #value_str);
            }
        } else if let Some(closure) = get_closure_expr(value) {
            // Reactive closure - create effect
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
            // Dynamic expression - wrap in effect
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
    } else {
        quote! {}
    };

    // Generate post-render class application code.
    // Uses add_class to merge with any classes the widget itself sets.
    let class_code = if let Some(prop) = class_prop {
        let value = &prop.value;
        if is_literal_expr(value) {
            // Static class string - add once
            let value_str = crate::helpers::expr_to_string(value);
            quote! {
                #result_var.add_class(#value_str);
            }
        } else if let Some(closure) = get_closure_expr(value) {
            // Reactive closure - create effect that updates class.
            // We track the previous extra class to remove it before adding the new one.
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
            // Dynamic expression - wrap in effect with tracking
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
    } else {
        quote! {}
    };

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

            // Apply style/class props to the rendered NodeHandle
            #style_code
            #class_code

            #result_var
        }
    }
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

/// Check if an expression likely contains reactive reads (Signal.get(), etc.).
///
/// This is a heuristic - it looks for method calls that might be reactive.
#[allow(dead_code)]
pub fn is_likely_reactive(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCall(call) => {
            let method = call.method.to_string();
            // Common reactive method names
            matches!(method.as_str(), "get" | "with" | "value" | "read")
                || is_likely_reactive(&call.receiver)
        }
        Expr::Call(call) => {
            // Check if it's a function that might be reactive
            call.args.iter().any(is_likely_reactive)
        }
        Expr::Binary(bin) => is_likely_reactive(&bin.left) || is_likely_reactive(&bin.right),
        Expr::Unary(un) => is_likely_reactive(&un.expr),
        Expr::Paren(p) => is_likely_reactive(&p.expr),
        Expr::Reference(r) => is_likely_reactive(&r.expr),
        Expr::Lit(_) => false,
        Expr::Path(_) => false, // Variable references are not inherently reactive
        _ => true,              // Assume unknown expressions might be reactive
    }
}

/// Generate a fine-grained component function signature.
///
/// Components in fine-grained mode take a RenderScope and return a NodeHandle:
/// ```ignore
/// fn my_component(__scope: &mut RenderScope) -> NodeHandle {
///     // ...
/// }
/// ```
#[allow(dead_code)]
pub fn generate_component_wrapper(body: TokenStream2, has_children: bool) -> TokenStream2 {
    if has_children {
        quote! {
            |__scope: &mut ::rinch::core::RenderScope, __children: &[::rinch::core::NodeHandle]| -> ::rinch::core::NodeHandle {
                #body
            }
        }
    } else {
        quote! {
            |__scope: &mut ::rinch::core::RenderScope| -> ::rinch::core::NodeHandle {
                #body
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

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
