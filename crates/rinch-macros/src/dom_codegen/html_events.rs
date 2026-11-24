//! Event handler registration for HTML elements (split out of `html.rs`).
//!
//! Each `on*` prop on an element becomes a registration in the building scope
//! and a `data-` attribute naming the handler id. Among a re-rendering
//! component's children, a handler the caller wrote is wrapped to run under
//! the caller's owner (issue #3030, see `handler_owner`).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::prop::RsxProp;

use super::captures::{collect_capture_idents, is_move_closure};
use super::handler_owner::element_handler;
use super::{DomCodegenContext, no_siblings};

/// The registration code for `event_props` on `elem_var` (a `<tag>`), and the
/// no-op `data-oninput` shim a commit-only text control needs.
pub(super) fn generate_event_code(
    event_props: &[&RsxProp],
    tag: &str,
    elem_var: &syn::Ident,
    ctx: &DomCodegenContext,
) -> (Vec<TokenStream2>, TokenStream2) {
    let has_event = |name: &str| event_props.iter().any(|prop| prop.name == name);
    // The DESKTOP runtime recognizes a custom text control by `data-oninput`,
    // so an element with a commit or submit handler and no `oninput` gets a
    // no-op input handler to stay focusable and receive typed text. (A text
    // `<input>`/`<textarea>` takes typed text by its element since #2585,
    // with or without this; the shim is kept for the custom case.) `onchange` has
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
                && matches!(tag, "input" | "textarea")))
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
            // Among a re-rendering component's children, the caller's handler
            // runs under the caller's owner (issue #3030).
            let owned = owned_handler(ctx, &event_name, handler);
            if event_name == "oninput" {
                // Input events use register_input_handler with Fn(String)
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_input_handler(#owned);
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
                        let __handler_id = __scope.register_input_handler(#owned);
                        #elem_var.set_attribute("data-onchange", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onscroll" {
                // Scroll events use register_scroll_handler with Fn(ScrollEvent)
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_scroll_handler(#owned);
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
                        let __handler_id = __scope.register_key_handler(#owned);
                        #elem_var.set_attribute(#data_attr, &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onwheel" {
                // A wheel over the element takes `Fn(&WheelEvent) -> bool`:
                // `true` consumes it — no ancestor's `onwheel` is offered it
                // and no scroll container scrolls.
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_wheel_handler(#owned);
                        #elem_var.set_attribute("data-onwheel", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "onfiledrop" {
                // OS file-drop events use register_file_drop_handler with Fn(Vec<PathBuf>)
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_file_drop_handler(#owned);
                        #elem_var.set_attribute("data-onfiledrop", &__handler_id.0.to_string());
                    }
                }
            } else if event_name == "ondropfiles" {
                // Files dropped from the OS, handed as `LocalFile`s on the
                // desktop and the web (issue #1771).
                quote! {
                    {
                        #handler_shadows
                        let __handler_id = __scope.register_files_drop_handler(#owned);
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
                        let __handler_id = rinch::core::register_handler(std::rc::Rc::new(#owned));
                        #elem_var.set_attribute(#data_attr, &__handler_id.0.to_string());
                    }
                }
            }
        })
        .collect();

    (event_code, text_field_shim)
}

/// `handler` as registered: wrapped by `CallerHandler` under the method for
/// its argument shape when it is the caller's, otherwise as written.
fn owned_handler(ctx: &DomCodegenContext, event_name: &str, handler: &syn::Expr) -> TokenStream2 {
    let method = match event_name {
        "oninput" | "onchange" => "input",
        "onscroll" => "scroll",
        "onkeydown" | "onkeyup" => "key",
        "onwheel" => "wheel",
        "onfiledrop" => "file_drop",
        "ondropfiles" => "files_drop",
        _ => "plain",
    };
    element_handler(ctx, method, handler)
}

#[cfg(test)]
mod focus_event_tests {
    //! #2731: `onfocus` / `onblur` / `onfocusin` / `onfocusout` are element
    //! event props, `Fn()` handlers written to their own attributes.
    use crate::dom_codegen::{DomCodegenContext, element_to_dom};
    use crate::element::RsxElement;
    use syn::parse_str;

    fn codegen(src: &str) -> String {
        let element: RsxElement = parse_str(src).expect("parses");
        let mut ctx = DomCodegenContext::new();
        element_to_dom(&element, &mut ctx).to_string()
    }

    /// Each is registered as a plain handler and written to its attribute —
    /// not a click (`data-rid`), and not a compile error (#1590's list had
    /// `onfocus` and `onblur` in it).
    #[test]
    fn the_focus_events_are_handlers_of_their_own() {
        for (name, attr) in [
            ("onfocus", "data-onfocus"),
            ("onblur", "data-onblur"),
            ("onfocusin", "data-onfocusin"),
            ("onfocusout", "data-onfocusout"),
        ] {
            let out = codegen(&format!("input {{ {name}: || {{}} }}"));
            assert!(!out.contains("compile_error"), "{name}: {out}");
            assert!(out.contains("register_handler"), "{name}: {out}");
            assert!(out.contains(&format!("\"{attr}\"")), "{name}: {out}");
            assert!(!out.contains("\"data-rid\""), "{name}: {out}");
            // Not a text field shim: a focus handler makes no element editable.
            assert!(!out.contains("\"data-oninput\""), "{name}: {out}");
        }
    }

    /// `onerror` (#2202/#2203) is a plain handler written to `data-onerror`:
    /// not a click, and not a compile error.
    #[test]
    fn onerror_is_a_handler_of_its_own() {
        let out = codegen("img { src: \"a.png\", onerror: || {} }");
        assert!(!out.contains("compile_error"), "{out}");
        assert!(out.contains("register_handler"), "{out}");
        assert!(out.contains("\"data-onerror\""), "{out}");
        assert!(!out.contains("\"data-rid\""), "{out}");
    }

    /// A near miss is a compile error that names the real one.
    #[test]
    fn a_misspelt_focus_event_suggests_the_real_one() {
        let out = codegen("input { onfocsu: || {} }");
        assert!(
            out.contains("compile_error") && out.contains("did you mean `onfocus`"),
            "{out}"
        );
    }
}
