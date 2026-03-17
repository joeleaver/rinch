//! Component DOM code generation.
//!
//! Handles PascalCase component dispatch (shell element detection) and
//! ThemeProvider reactive wrapper generation.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;

use super::DomCodegenContext;

/// Generate DOM code for a rinch component.
///
/// For components (PascalCase), generates direct construction and render call.
/// Shell elements (Window, Menu, etc.) are NOT supported inside rsx! - use run() for windows
/// and the menu API for menus.
pub fn element_to_dom_component(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
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

    // For components, generate direct construction and render call
    super::component_codegen::element_to_dom_component(element, ctx)
}

/// Generate DOM code for ThemeProvider (reactive theme wrapper).
///
/// ThemeProvider accepts reactive props for theme values and creates an Effect
/// that updates the theme CSS when those values change.
pub fn element_to_dom_theme_provider(
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
                rinch::core::Effect::new(move || {
                    let __color = (__pc_fn)();
                    let __dark = (__dm_fn)();
                    rinch::fine_grained::update_theme(&rinch::core::element::ThemeProviderProps {
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
        .map(|child| super::generate_child_code(child, &container_var, ctx))
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
