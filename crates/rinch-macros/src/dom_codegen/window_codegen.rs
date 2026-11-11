//! `Window { … }` in `rsx!` (#2416): a window of the app, open while the
//! element is mounted.
//!
//! Its props are [`WindowProps`] fields, read when the window opens, plus
//! `on_open` (called with its `WindowHandle`) and `on_closed` (called once
//! it has closed for any cause but the element going). Its children are the
//! window's content: they are not built here, in the document the element
//! sits in, but wrapped in the same render closure an `if` branch is, which
//! the window runs in its own document.
//!
//! [`WindowProps`]: https://docs.rs/rinch-core/latest/rinch_core/element/struct.WindowProps.html

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};

use super::DomCodegenContext;
use super::captures::wrap_site;
use crate::element::RsxElement;

/// What changes a prop after the window has opened, for the error a
/// reactive value gets.
fn changed_by(field: &str) -> &'static str {
    match field {
        "title" => "`WindowHandle::set_title`",
        "width" | "height" => "`WindowHandle::set_size`",
        "x" | "y" => "`WindowHandle::set_position`",
        "output" => "`WindowHandle::set_output`",
        "wayland_layer" => "`WindowHandle::set_layer`",
        "click_through" => "`WindowHandle::set_click_through`",
        "visible" => "`WindowHandle::show` and `WindowHandle::hide`",
        _ => "nothing: close the window and open it again with the new value",
    }
}

pub fn element_to_dom_window(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let mut fields = Vec::new();
    let mut on_open = quote! { |_: rinch::WindowHandle| {} };
    let mut on_closed = quote! { || {} };
    for prop in &element.props {
        let name = prop.name.as_str();
        let value = &prop.value;
        let span = prop.name_span();
        if name == "key" {
            // Read by an enclosing `for`.
            continue;
        }
        if prop.quoted || name.contains('-') {
            let msg = format!(
                "`Window` takes `WindowProps` fields, `on_open` and `on_closed`; `{name}` is none of them"
            );
            return quote_spanned! { span => compile_error!(#msg) };
        }
        match name {
            "on_open" => on_open = quote! { #value },
            "on_closed" => on_closed = quote! { #value },
            "on_close_requested" => {
                fields.push(quote! { on_close_requested: Some(::std::sync::Arc::new(#value)) });
            }
            _ => {
                if crate::helpers::get_closure_expr(value).is_some() {
                    let msg = format!(
                        "`Window`'s props are read when it opens, so `{name}` cannot follow a \
                         signal; what changes it later is {}",
                        changed_by(name)
                    );
                    return quote_spanned! { span => compile_error!(#msg) };
                }
                if crate::helpers::is_literal_bool(value)
                    || crate::helpers::is_literal_int(value)
                    || crate::helpers::is_literal_float(value)
                {
                    // Into the field as written: its type decides the
                    // literal's (`height: 32` is a `u32`); an `Option` field
                    // takes `Some(…)`, as `WindowProps` itself does.
                    let field = prop.name_as_ident();
                    fields.push(quote! { #field: #value });
                } else {
                    fields.extend(
                        super::component_codegen::generate_component_field_assignments(
                            &[prop],
                            false,
                        ),
                    );
                }
            }
        }
    }

    let (content, caps) = super::control_flow::generate_children_closure(&element.children, ctx);
    let content = wrap_site(&ctx.site_shadows(&caps, &HashSet::new()), content);
    quote! {
        rinch::__window_component(
            __scope,
            rinch::WindowProps {
                #(#fields,)*
                ..::core::default::Default::default()
            },
            #on_open,
            #on_closed,
            #content,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DomCodegenContext, element_to_dom};
    use crate::element::RsxElement;
    use syn::parse_str;

    fn codegen(src: &str) -> String {
        let element: RsxElement = parse_str(src).expect("parses");
        let mut ctx = DomCodegenContext::new();
        element_to_dom(&element, &mut ctx).to_string()
    }

    #[test]
    fn a_window_s_children_are_its_content_closure_not_built_here() {
        let out = codegen(r#"Window { title: "panel", height: 32, div { "inside" } }"#);
        assert!(out.contains("__window_component"), "{out}");
        assert!(out.contains("WindowProps"), "{out}");
        // The child is created inside the content closure, with its own scope.
        let closure = out.find("__child_scope").expect("a content closure");
        let child = out.find("\"inside\"").expect("the child");
        assert!(child > closure, "the child is built in the closure: {out}");
        assert!(!out.contains("compile_error"), "{out}");
    }

    #[test]
    fn a_reactive_prop_is_a_compile_error_naming_what_changes_it() {
        let out = codegen("Window { title: {|| name.get()} }");
        assert!(out.contains("compile_error"), "{out}");
        assert!(out.contains("read when it opens"), "{out}");
        assert!(out.contains("WindowHandle::set_title"), "{out}");
        let out = codegen("Window { borderless: {|| flag.get()} }");
        assert!(out.contains("close the window and open it again"), "{out}");
    }

    #[test]
    fn on_open_and_on_closed_are_the_window_s_and_take_closures() {
        let out =
            codegen("Window { on_closed: move || shown.set(false), on_open: move |h| keep(h) }");
        assert!(!out.contains("compile_error"), "{out}");
        assert!(
            !out.contains("on_closed :"),
            "not a WindowProps field: {out}"
        );
    }

    #[test]
    fn an_attribute_is_not_a_window_prop() {
        let out = codegen(r#"Window { "data-x": "1" }"#);
        assert!(out.contains("compile_error"), "{out}");
    }
}
