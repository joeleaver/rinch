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

use super::captures::{collect_capture_idents, is_move_closure, shadow_clones};
use super::{DomCodegenContext, no_siblings};

/// The two shadow-clone prologues a reactive binding needs (issue #223).
///
/// An effect is a `move` closure built where the binding sits, and it rebuilds
/// the user's closure on every fire — so it competes for that closure's values
/// twice: once with the body it is built in (`site`, emitted before
/// `create_effect`), and once with its own next fire (`fire`, emitted inside the
/// effect). Only a `move` closure is rebuilt destructively, so a borrowing one
/// takes the `site` prologue — the effect still captures by value — but no
/// `fire` prologue: it reads through the effect's own capture.
fn reactive_shadows(
    value: &syn::Expr,
    closure: Option<&syn::Expr>,
    ctx: &DomCodegenContext,
) -> (TokenStream2, TokenStream2) {
    let caps = collect_capture_idents(closure.unwrap_or(value));
    let site = ctx.site_shadows(&caps, &no_siblings());
    let fire = match closure {
        Some(closure) if is_move_closure(closure) => shadow_clones(caps.iter()),
        _ => quote! {},
    };
    (site, fire)
}

/// Generate DOM code for an HTML element.
pub fn element_to_dom_html(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let tag = element.name.to_string();
    let elem_var = ctx.next_var("elem");

    // Separate event handlers, shorthands, and regular attributes
    let mut event_props = Vec::new();
    let mut shorthand_props = Vec::new();
    let mut attr_props = Vec::new();

    // `style:` is pulled out of the attribute list: it is not a string written
    // over whatever is there, it is a set of declarations laid over them
    // (issue #647). A style shorthand is a second author on the same attribute,
    // and so is anything the runtime writes through `set_style`.
    //
    // A side effect worth knowing, since #154 makes effect registration order a
    // contract: a reactive `style:`'s effect is now always created after every
    // reactive *attribute* effect on the same element, where it used to follow
    // prop order. Nothing observable rides on it today — they write different
    // attributes — but it is an ordering change, not a no-op.
    let mut style_prop = None;
    // `ref:` hands the app this element's handle (issue #1779).
    let mut ref_prop = None;

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if prop.quoted {
            // A quoted name is the attribute of that name (issue #1653): never
            // an event, a style shorthand or `style:`'s merge — the parser
            // refuses the names those would need (`"style"`, `"on…"`).
            attr_props.push(prop);
        } else if name_str == "key" {
            // Skip key prop — used by for-loop reconciliation, not a DOM attribute
            continue;
        } else if name_str == "ref" {
            ref_prop = Some(prop);
        } else if is_event_prop(&name_str) {
            event_props.push(prop);
        } else if name_str == "style" {
            style_prop = Some(prop);
        } else if expand_style_shorthand(&name_str).is_some() {
            shorthand_props.push(prop);
        } else {
            attr_props.push(prop);
        }
    }

    // An `on*` name with no handler attribute, or a component-only `*_fn`
    // prop, is a compile error rather than a silent click handler or phantom
    // attribute (issues #1590, #1591).
    let mut errors: Option<syn::Error> = None;
    let mut push_error = |e: syn::Error| match &mut errors {
        Some(all) => all.combine(e),
        None => errors = Some(e),
    };
    for prop in &event_props {
        if crate::events::event_attribute(&prop.name).is_none() {
            push_error(crate::events::unknown_event_error(
                &prop.name,
                prop.name_span(),
            ));
        }
    }
    for prop in &attr_props {
        if !prop.quoted && !prop.name.contains('-') && prop.name.ends_with("_fn") {
            push_error(crate::events::fn_prop_error(&prop.name, prop.name_span()));
        }
    }
    if let Some(errors) = errors {
        return errors.to_compile_error();
    }

    let attr_code = generate_attr_code(&attr_props, &elem_var, ctx);

    // Generate event handler registration
    let has_event = |name: &str| event_props.iter().any(|prop| prop.name == name);
    // The DESKTOP runtime recognizes a text input by `data-oninput`, so an
    // element with a commit or submit handler and no `oninput` gets a no-op
    // input handler to stay focusable and receive typed text. `onchange` has
    // always had it; an `onsubmit` gets it only on a text control (#1590),
    // and so do `onkeydown` / `onkeyup` (#1678 — a raw composer read through
    // `live_value()` on Enter has no `oninput`), because these are also
    // written on a wrapper around a field, and a
    // `div` carrying `data-oninput` would itself become a text field. The shim
    // is not emitted for wasm32: the browser owns focus/typing natively there,
    // and the extra attribute would shadow an ancestor's `[data-oninput]` in
    // the web backend's document-level input delegation walk (#244 review).
    let text_field_shim = if !has_event("oninput")
        && (has_event("onchange")
            || ((has_event("onsubmit") || has_event("onkeydown") || has_event("onkeyup"))
                && matches!(tag.as_str(), "input" | "textarea")))
    {
        quote! {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let __noop_id = __scope.register_input_handler(|_: String| {});
                #elem_var.set_attribute("data-oninput", &__noop_id.0.to_string());
            }
        }
    } else {
        quote! {}
    };
    let event_code: Vec<TokenStream2> = event_props
        .iter()
        .map(|prop| {
            let handler = &prop.value;
            let event_name = prop.name.to_string();
            // A handler is built fresh on every run of the body it sits in, and
            // it is `'static`, so it moves what it names out of that body's
            // captures — one iteration's handler competing with the next
            // iteration's for an ancestor-scope value (issue #223). Only a
            // `move` closure captures by value; anything else is left alone.
            let handler_shadows = if is_move_closure(handler) {
                ctx.site_shadows(&collect_capture_idents(handler), &no_siblings())
            } else {
                quote! {}
            };
            if event_name == "oninput" {
                // Input events use register_input_handler with Fn(String)
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_input_handler(#handler);
                        #elem_var.set_attribute("data-oninput", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onchange" {
                // Commit boundary (issue #226): fires once when the typed
                // gesture ends (blur after modification, Enter, a <select>
                // pick) with the final value — HTML `change` semantics, no
                // longer an alias for the per-keystroke `oninput`. An
                // onchange-only element gets `text_field_shim` (above).
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_input_handler(#handler);
                        #elem_var.set_attribute("data-onchange", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onscroll" {
                // Scroll events use register_scroll_handler with Fn(ScrollEvent)
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_scroll_handler(#handler);
                        #elem_var.set_attribute("data-onscroll", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onkeydown" || event_name == "onkeyup" {
                // Key events (issue #1678) take `Fn(&KeyEventData) -> bool`:
                // `true` consumes the key — its default handling (a line
                // break, a submit, a focus move) does not happen and nothing
                // after the element is offered it.
                let data_attr = crate::events::event_attribute(&event_name)
                    .expect("both key events are in HTML_EVENTS");
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_key_handler(#handler);
                        #elem_var.set_attribute(#data_attr, &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onfiledrop" {
                // OS file-drop events use register_file_drop_handler with Fn(Vec<PathBuf>)
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_file_drop_handler(#handler);
                        #elem_var.set_attribute("data-onfiledrop", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "ondropfiles" {
                // Files dropped from the OS, handed as `LocalFile`s on the
                // desktop and the web (issue #1771).
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_files_drop_handler(#handler);
                        #elem_var.set_attribute("data-ondropfiles", &__handler_id.0.to_string());
                    }
                }
            } else {
                // Every other supported event is a `Fn()` handler written to
                // its own `data-` attribute, so the runtime can tell them apart
                // during walk-up dispatch (`onclick` is `data-rid`). An
                // unsupported name was rejected above.
                let data_attr = crate::events::event_attribute(&event_name)
                    .expect("unsupported events are rejected before codegen");
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = rinch::core::register_handler(std::rc::Rc::new(#handler));
                        #elem_var.set_attribute(#data_attr, &__handler_id.0.to_string());
                    }
                }
            }
        })
        .collect();

    // Emitted where the `style` attribute used to sit in the attribute loop, so
    // a shorthand prop still wins a collision with it.
    let style_code = generate_style_code(style_prop, &elem_var, ctx);

    // Generate shorthand style code (e.g., p: "md" -> set_style("padding", ...))
    let shorthand_code = generate_shorthand_code(&shorthand_props, &elem_var, ctx);

    // Generate children - Show/For use marker-based insertion directly into parent
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &elem_var, ctx))
        .collect();

    // After the element's attributes and children, before the caller appends it.
    let ref_code = generate_ref_code(ref_prop, &elem_var);

    // Check if void element
    if is_void_element(&tag) {
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#attr_code)*
                #style_code
                #(#event_code)*
                #text_field_shim
                #shorthand_code
                #ref_code
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
                #style_code
                #(#event_code)*
                #text_field_shim
                #shorthand_code
                #ref_code
                #elem_var
            }
        }
    } else {
        quote! {
            {
                let #elem_var = __scope.create_element(#tag);
                #(#attr_code)*
                #style_code
                #(#event_code)*
                #text_field_shim
                #shorthand_code
                #(#children_code)*
                #ref_code
                #elem_var
            }
        }
    }
}

/// `ref: target` (issue #1779): set `target` — a
/// `Signal<Option<NodeHandle>>` — to the element, and back to `None` when the
/// building scope is disposed (`rinch_core::dom::bind_node_ref`). Also used for
/// a component's root.
pub fn generate_ref_code(ref_prop: Option<&RsxProp>, elem_var: &syn::Ident) -> TokenStream2 {
    match ref_prop {
        Some(prop) => {
            let value = &prop.value;
            quote! {
                rinch::core::dom::bind_node_ref(__scope, #value, &#elem_var);
            }
        }
        None => quote! {},
    }
}

/// Generate attribute-writing code for `attr_props` on the **stable** node
/// `elem_var`: an HTML element's own attributes, and the hyphenated attributes
/// a caller writes on a component, which land on its root after
/// `Component::render` (issue #433).
///
/// Every path here goes through `NodeHandle::write_attribute`, not
/// `set_attribute`: an HTML boolean attribute's value is a *shape* and not a
/// string, so a rendered `false` has to remove the attribute rather than
/// write `disabled="false"` — which HTML reads as *present*, and therefore
/// true (issue #551). The list of such attributes lives in `rinch-core`; the
/// macro deliberately does not carry a copy, and deliberately keys on the
/// attribute **name** rather than the value's type, because `draggable`,
/// `aria-*` and rinch's own `data-viewport-ready` all take a meaningful
/// `"false"`.
///
/// A literal is written once; a closure, or any other expression, becomes an
/// effect that rewrites the attribute whenever a signal it reads changes.
pub fn generate_attr_code(
    attr_props: &[&RsxProp],
    elem_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> Vec<TokenStream2> {
    attr_props
        .iter()
        .map(|prop| {
            let name = prop.name.to_string();
            let value = &prop.value;

            if is_literal_expr(value) {
                // Static attribute - set once
                let value_str = crate::helpers::expr_to_string(value);
                quote! {
                    #elem_var.write_attribute(#name, #value_str);
                }
            } else if let Some(closure) = get_closure_expr(value) {
                // Closure expression - call it and use result
                let handle_var = ctx.next_var("attr_handle");
                let (site, fire) = reactive_shadows(value, Some(closure), ctx);
                quote! {
                    {
                        let #handle_var = #elem_var.clone();
                        #site
                        __scope.create_effect(move || {
                            #fire
                            #handle_var.write_attribute(#name, &::std::string::ToString::to_string(&(#closure)()));
                        });
                    }
                }
            } else {
                // Dynamic expression (not a closure) - wrap in effect
                let handle_var = ctx.next_var("attr_handle");
                let (site, _) = reactive_shadows(value, None, ctx);
                quote! {
                    {
                        let #handle_var = #elem_var.clone();
                        #site
                        __scope.create_effect(move || {
                            #handle_var.write_attribute(#name, &::std::string::ToString::to_string(&#value));
                        });
                    }
                }
            }
        })
        .collect()
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
                        let (site, fire) = reactive_shadows(value, Some(closure), ctx);
                        quote! {
                            {
                                let #handle_var = #result_var.clone();
                                #site
                                __scope.create_effect(move || {
                                    #fire
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

/// Generate style application code for a `style:` prop on a **stable** node.
///
/// Used for a component's root after `Component::render` (the static component
/// path; a re-rendering component's root takes `component_codegen::RootBindings`,
/// issue #1190) and for a plain HTML element. Both are laid *over* whatever the node
/// already carries rather than replacing it (issue #647): a component publishes
/// its props as inline declarations on its root — every overlay's `z_index` is
/// a custom property written there — and a style shorthand (`p:`, `mt:` …) is a
/// second author on the same attribute, so a write of the whole `style`
/// attribute erased one or both with nothing warning.
///
/// The node is stable here, so the reactive arms carry a [`StyleProp`] across
/// fires: it takes its own previous declarations off before laying the new ones
/// on, which is what stops a binding from accumulating properties it no longer
/// declares — and restores whatever value each one displaced.
///
/// [`StyleProp`]: rinch_core::StyleProp
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
            #result_var.merge_style(#value_str);
        }
    } else if let Some(closure) = get_closure_expr(value) {
        let handle_var = ctx.next_var("style_handle");
        let binding_var = ctx.next_var("style_prop");
        let (site, fire) = reactive_shadows(value, Some(closure), ctx);
        quote! {
            {
                let #handle_var = #result_var.clone();
                let mut #binding_var = rinch::core::StyleProp::default();
                #site
                __scope.create_effect(move || {
                    #fire
                    #binding_var.apply(&#handle_var, &::std::string::ToString::to_string(&(#closure)()));
                });
            }
        }
    } else {
        let handle_var = ctx.next_var("style_handle");
        let binding_var = ctx.next_var("style_prop");
        let (site, _) = reactive_shadows(value, None, ctx);
        quote! {
            {
                let #handle_var = #result_var.clone();
                let mut #binding_var = rinch::core::StyleProp::default();
                #site
                __scope.create_effect(move || {
                    #binding_var.apply(&#handle_var, &::std::string::ToString::to_string(&#value));
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
        let (site, fire) = reactive_shadows(value, Some(closure), ctx);
        quote! {
            {
                let #handle_var = #result_var.clone();
                let #prev_var = ::std::cell::RefCell::new(String::new());
                #site
                __scope.create_effect(move || {
                    #fire
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
        let (site, _) = reactive_shadows(value, None, ctx);
        quote! {
            {
                let #handle_var = #result_var.clone();
                let #prev_var = ::std::cell::RefCell::new(String::new());
                #site
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

#[cfg(test)]
mod event_and_fn_prop_tests {
    //! What an `on*` or `*_fn` prop on an HTML element becomes (#1590, #1591).
    use crate::dom_codegen::{DomCodegenContext, element_to_dom};
    use crate::element::RsxElement;
    use syn::parse_str;

    fn codegen(src: &str) -> String {
        let element: RsxElement = parse_str(src).expect("parses");
        let mut ctx = DomCodegenContext::new();
        element_to_dom(&element, &mut ctx).to_string()
    }

    fn is_error(out: &str) -> bool {
        out.contains("compile_error")
    }

    /// #1590: `onsubmit` is the Enter handler, read by both backends from
    /// `data-onsubmit`; it fell through to `data-rid`, the click handler.
    #[test]
    fn onsubmit_is_written_as_data_onsubmit_not_as_a_click() {
        let out = codegen("input { onsubmit: || {} }");
        assert!(!is_error(&out), "{out}");
        assert!(out.contains("\"data-onsubmit\""), "{out}");
        assert!(!out.contains("\"data-rid\""), "{out}");
    }

    /// An `onsubmit`-only `<input>`/`<textarea>` gets desktop's text-field
    /// shim (a no-op `data-oninput`), as an `onchange`-only one does; any other
    /// element does not, since a `div` carrying `data-oninput` is a text field.
    #[test]
    fn the_text_field_shim_is_given_to_controls_only() {
        for tag in ["input", "textarea"] {
            let out = codegen(&format!("{tag} {{ onsubmit: || {{}} }}"));
            assert!(out.contains("\"data-oninput\""), "{tag}: {out}");
            // #1678: a key handler makes a raw control a text field too.
            for ev in ["onkeydown", "onkeyup"] {
                let out = codegen(&format!("{tag} {{ {ev}: |_| false }}"));
                assert!(out.contains("\"data-oninput\""), "{tag} {ev}: {out}");
            }
        }
        let out = codegen("div { onsubmit: || {} }");
        assert!(!out.contains("\"data-oninput\""), "{out}");
        let out = codegen("div { onkeydown: |_| false }");
        assert!(!out.contains("\"data-oninput\""), "{out}");
        // Beside a real oninput no shim is added (one data-oninput write).
        let out = codegen("input { oninput: |_: String| {}, onsubmit: || {} }");
        assert_eq!(out.matches("\"data-oninput\"").count(), 1, "{out}");
    }

    /// `onclick` stays the click handler.
    #[test]
    fn onclick_is_still_data_rid() {
        let out = codegen("button { onclick: || {} }");
        assert!(!is_error(&out) && out.contains("\"data-rid\""), "{out}");
    }

    /// Every event the macro supports still generates its handler — without
    /// this, rejecting every `on*` name would pass the tests below.
    #[test]
    fn every_supported_event_generates_a_handler() {
        for (name, attr) in [
            ("onclick", "data-rid"),
            ("oninput", "data-oninput"),
            ("onchange", "data-onchange"),
            ("onsubmit", "data-onsubmit"),
            ("onscroll", "data-onscroll"),
            ("onfiledrop", "data-onfiledrop"),
            ("ondropfiles", "data-ondropfiles"),
            ("onfiledragenter", "data-onfiledragenter"),
            ("onfiledragleave", "data-onfiledragleave"),
            ("ondragstart", "data-ondragstart"),
            ("ondragmove", "data-ondragmove"),
            ("ondragend", "data-ondragend"),
            ("ondrop", "data-ondrop"),
            ("ondragenter", "data-ondragenter"),
            ("ondragover", "data-ondragover"),
            ("ondragleave", "data-ondragleave"),
            ("oncontextmenu", "data-oncontextmenu"),
            ("onmouseenter", "data-onenter"),
            ("onmouseleave", "data-onleave"),
            ("onmousedown", "data-onmousedown"),
            ("onmouseup", "data-onmouseup"),
            ("onmousemove", "data-onmousemove"),
        ] {
            let out = codegen(&format!("div {{ {name}: || {{}} }}"));
            assert!(!is_error(&out), "{name}: {out}");
            assert!(
                out.contains(&format!("\"{attr}\"")),
                "{name} -> {attr}: {out}"
            );
        }
    }

    /// #1590: an `on*` name the macro does not support was silently a click
    /// handler. It is a compile error now, naming the supported events.
    #[test]
    fn an_unsupported_event_is_a_compile_error_not_a_click() {
        for name in ["onfocus", "onblur", "ondblclick", "onkeypress"] {
            let out = codegen(&format!("div {{ {name}: || {{}} }}"));
            assert!(is_error(&out), "{name}: {out}");
            assert!(!out.contains("\"data-rid\""), "{name}: {out}");
            assert!(
                out.contains("onclick") && out.contains("onsubmit"),
                "{name} must list the supported events: {out}"
            );
        }
    }

    /// A typo gets a suggestion, like an unknown tag (#528).
    #[test]
    fn a_misspelt_event_suggests_the_real_one() {
        let out = codegen("div { onclik: || {} }");
        assert!(
            is_error(&out) && out.contains("did you mean `onclick`"),
            "{out}"
        );
        let out = codegen("input { onsumbit: || {} }");
        assert!(
            is_error(&out) && out.contains("did you mean `onsubmit`"),
            "{out}"
        );
        // And a name close to nothing suggests nothing.
        let out = codegen("div { onfocus: || {} }");
        assert!(!out.contains("did you mean"), "{out}");
    }

    /// #1678: `onkeydown` / `onkeyup` are element key handlers, registered
    /// as `Fn(&KeyEventData) -> bool` and written to their own attributes —
    /// not click handlers, not `Fn()`.
    #[test]
    fn the_key_events_register_key_handlers() {
        for (name, attr) in [("onkeydown", "data-onkeydown"), ("onkeyup", "data-onkeyup")] {
            let out = codegen(&format!("textarea {{ {name}: |k| k.shift }}"));
            assert!(!is_error(&out), "{name}: {out}");
            assert!(out.contains("register_key_handler"), "{name}: {out}");
            assert!(out.contains(&format!("\"{attr}\"")), "{name}: {out}");
            assert!(!out.contains("\"data-rid\""), "{name}: {out}");
        }
    }

    /// #1591: `value_fn:` / `checked_fn:` are component prop conventions; on a
    /// raw element they used to become a `value_fn` attribute nothing reads.
    #[test]
    fn a_fn_prop_on_an_html_element_is_a_compile_error_naming_the_attribute() {
        for (tag, name, attr) in [
            ("input", "value_fn", "value"),
            ("textarea", "value_fn", "value"),
            ("select", "value_fn", "value"),
            ("input", "checked_fn", "checked"),
            ("div", "opened_fn", "opened"),
        ] {
            let out = codegen(&format!("{tag} {{ {name}: move || String::new() }}"));
            assert!(is_error(&out), "{tag} {name}: {out}");
            assert!(
                !out.contains(&format!("\"{name}\"")),
                "no phantom attribute: {out}"
            );
            assert!(
                out.contains(&format!("`{attr}: {{|| …}}`")),
                "{tag} {name} must name `{attr}:`: {out}"
            );
        }
    }

    /// The other half: a reactive `value:` / `checked:` is still an attribute.
    #[test]
    fn a_reactive_value_or_checked_is_still_an_attribute() {
        for name in ["value", "checked"] {
            let out = codegen(&format!("input {{ {name}: {{|| true}} }}"));
            assert!(
                !is_error(&out) && out.contains(&format!("\"{name}\"")),
                "{out}"
            );
        }
    }
}
