//! Component DOM code generation.
//!
//! Handles code generation for PascalCase components, including
//! static components and reactive components that re-render when signals change.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::helpers::{
    expand_style_shorthand, get_closure_expr, is_literal_expr, resolve_spacing_value,
};
use crate::prop::RsxProp;

use super::captures::{
    collect_body_captures, collect_capture_idents, is_move_closure, shadow_clones, wrap_site,
};
use super::html::{
    generate_attr_code, generate_class_code, generate_shorthand_code, generate_style_code,
};
use super::{DomCodegenContext, no_siblings};

/// Whether a prop written on a component is an attribute for the component's
/// **root DOM element** rather than a field of its struct (issue #433).
///
/// Exactly the hyphenated names — `data-nofocus`, `aria-label`,
/// `data-viewport` — because a hyphen is not legal in a Rust identifier, so no
/// component can ever declare a field of that name and the routing cannot
/// shadow a prop. Such a prop is applied after `Component::render`, the way
/// `style:` and `class:` are: through `NodeHandle::write_attribute`, so a
/// boolean attribute (`data-nofocus`, `data-disabled`, `data-trap-focus`,
/// `data-backdrop`, `hidden`, …) is a presence for a truthy value and removed
/// for a falsey one (#551), exactly as on an HTML element.
pub fn is_root_attribute(name: &str) -> bool {
    name.contains('-')
}

/// Check if an element is a component with reactive (closure) props that needs
/// statement-based insertion (like control flow) rather than expression-based.
pub fn has_reactive_component_props(element: &RsxElement) -> bool {
    if !element.is_rinch_component() {
        return false;
    }
    element.props.iter().any(|p| {
        let name = p.name.to_string();
        // A hyphenated attribute is applied to the root, not the struct
        // (issue #433): a reactive one is an effect on a stable root, like a
        // reactive `style:`/`class:`, and re-renders nothing.
        if name == "key" || name == "style" || name == "class" || is_root_attribute(&name) {
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
    let mut attr_props = Vec::new();
    let mut comp_props = Vec::new();

    for prop in &element.props {
        let name_str = prop.name.to_string();
        if name_str == "key" {
            continue;
        } else if name_str == "style" {
            style_prop = Some(prop);
        } else if name_str == "class" {
            class_prop = Some(prop);
        } else if is_root_attribute(&name_str) {
            attr_props.push(prop);
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

    let bindings = RootBindings::plan(
        ctx,
        &attr_props,
        style_prop,
        class_prop,
        &shorthand_props,
        &result_var,
    );
    let root_bindings = &bindings.per_render;

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
        // every inner change. The prop closures above stay in the tracked
        // region: their signal reads are what schedule a re-render. The
        // caller's root bindings below are effects of their own (issue #1190).
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

            let __rendered = rinch::core::Component::render(&#comp_var, __scope, &#children_var);
            // The scratch container is attached to nothing, so no subtree walk
            // can ever reclaim it (issue #719). After the render: the children
            // the component adopted have been re-parented out by then.
            rinch::core::dom::release_scratch_container(__scope, &#temp_var);
            __rendered
        });
        #root_bindings
        #result_var
    };
    let (binding_fns, render) = bindings.finish(ctx, &body);
    quote! {
        {
            #binding_fns
            rinch::core::reactive_component_dom(__scope, &#parent_var, #render);
        }
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
    let mut attr_props = Vec::new();
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
        } else if is_root_attribute(&name_str) {
            attr_props.push(prop);
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
            &attr_props,
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

    // Generate post-render attribute/style/class/shorthand application code
    let attr_code = generate_attr_code(&attr_props, &result_var, ctx);
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
            // The scratch container is attached to nothing, so no subtree walk
            // can ever reclaim it (issue #719). After the render: the children
            // the component adopted have been re-parented out by then.
            rinch::core::dom::release_scratch_container(__scope, &#temp_var);

            // Apply hyphenated attributes and style/class/shorthand props to
            // the rendered NodeHandle. The caller writes after the component,
            // so a collision with an attribute the component wrote on its own
            // root goes to the caller (issue #433).
            #(#attr_code)*
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
    attr_props: &[&RsxProp],
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

    let bindings = RootBindings::plan(
        ctx,
        attr_props,
        style_prop,
        class_prop,
        shorthand_props,
        &result_var,
    );
    let root_bindings = &bindings.per_render;

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
        // every inner change. The prop closures above stay in the tracked
        // region: their signal reads are what schedule a re-render. The
        // caller's root bindings below are effects of their own (issue #1190).
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

            let __rendered = rinch::core::Component::render(&#comp_var, __scope, &#children_var);
            // The scratch container is attached to nothing, so no subtree walk
            // can ever reclaim it (issue #719). After the render: the children
            // the component adopted have been re-parented out by then.
            rinch::core::dom::release_scratch_container(__scope, &#temp_var);
            __rendered
        });
        #root_bindings
        #result_var
    };
    let (binding_fns, render) = bindings.finish(ctx, &body);
    quote! {
        {
            #binding_fns
            let #wrapper_var = __scope.create_element("div");
            #wrapper_var.set_attribute("style", "display:contents");
            rinch::core::reactive_component_dom(__scope, &#wrapper_var, #render);
            #wrapper_var
        }
    }
}

/// The caller's root bindings — hyphenated attributes, `style:`, `class:` and
/// style shorthands — on a component that **re-renders** for a reactive struct
/// prop (issue #1190).
///
/// Each reactive binding is an effect of its own and never a read of the render
/// closure. The render closure runs tracked — that is how a struct prop
/// re-renders — so a binding invoked there subscribed the re-render: a change to
/// the signal a `style: {|| …}` reads rebuilt the whole component and reset its
/// local state. The effect is created inside the render closure, where `__scope`
/// is the per-render child scope, so it is owned by that render and disposed
/// with it by `reactive_component_dom` on the next re-render; the new root gets
/// fresh effects, and with them a fresh reactive `style:` / `class:` memory
/// (#647, #717), since nothing of the caller's is on a root built a moment ago.
///
/// The user's expressions are **not** rebuilt per render, and they are not
/// split off into closures of their own either: that captured a name a struct
/// prop (or a child) and a binding both named twice, which needs `Clone` — and
/// an analysis that cannot see inside `format!` could not even tell. Instead the
/// whole site is **one** `move` closure, a site bundle
/// (`rinch::core::dom::SiteFn`), that renders the component on
/// `SiteCall::Render` and evaluates binding `N` on `SiteCall::Binding(N)`; the
/// render closure handed to `reactive_component_dom` and every binding effect
/// hold it by `Rc`. So everything the caller's tokens name is captured exactly
/// once, as when the bindings were evaluated inside the render closure: a borrow
/// of a non-`Clone` value compiles, and state a binding keeps in a captured cell
/// lives as long as the site. A `move` binding closure keeps its per-call shadow
/// clones (`#fire`), since the bundle is an `Fn` that rebuilds it on every call.
/// A site with no reactive binding keeps the plain render closure.
struct RootBindings {
    /// Binding `N`'s user expression, evaluated to a `String`.
    evals: Vec<TokenStream2>,
    /// The name the render arm binds the bundle to (`&Rc<SiteFn>`).
    site_self: syn::Ident,
    /// What the render closure runs after `Component::render`.
    per_render: TokenStream2,
}

impl RootBindings {
    fn plan(
        ctx: &mut DomCodegenContext,
        attr_props: &[&RsxProp],
        style_prop: Option<&RsxProp>,
        class_prop: Option<&RsxProp>,
        shorthand_props: &[&RsxProp],
        result_var: &syn::Ident,
    ) -> Self {
        let mut evals = Vec::new();
        let mut code = Vec::new();
        let site_self = ctx.next_var("site_self");

        // A binding fn for a reactive value (a closure or a non-literal
        // expression); `None` for a literal, which is written once inline.
        let mut binding_fn = |value: &syn::Expr| {
            if is_literal_expr(value) {
                return None;
            }
            let eval = if let Some(closure) = get_closure_expr(value) {
                let fire = if is_move_closure(closure) {
                    shadow_clones(collect_capture_idents(closure).iter())
                } else {
                    quote! {}
                };
                quote! { { #fire ::std::string::ToString::to_string(&(#closure)()) } }
            } else {
                quote! { ::std::string::ToString::to_string(&(#value)) }
            };
            let index = proc_macro2::Literal::u32_unsuffixed(evals.len() as u32);
            evals.push(eval);
            // A closure over the bundle that evaluates this binding.
            Some(quote! {
                {
                    let __s = ::std::rc::Rc::clone(#site_self);
                    move || (__s)(rinch::core::dom::SiteCall::Binding(#index)).into_string()
                }
            })
        };

        for prop in attr_props {
            let name = prop.name.to_string();
            match binding_fn(&prop.value) {
                None => {
                    let v = crate::helpers::expr_to_string(&prop.value);
                    code.push(quote! { #result_var.write_attribute(#name, #v); });
                }
                Some(f) => code.push(quote! {
                    {
                        let __h = #result_var.clone();
                        let __f = #f;
                        __scope.create_effect(move || {
                            __h.write_attribute(#name, &__f());
                        });
                    }
                }),
            }
        }

        if let Some(prop) = style_prop {
            match binding_fn(&prop.value) {
                None => {
                    let v = crate::helpers::expr_to_string(&prop.value);
                    code.push(quote! { #result_var.merge_style(#v); });
                }
                Some(f) => code.push(quote! {
                    {
                        let __h = #result_var.clone();
                        let __f = #f;
                        let mut __sp = rinch::core::StyleProp::default();
                        __scope.create_effect(move || {
                            __sp.apply(&__h, &__f());
                        });
                    }
                }),
            }
        }

        if let Some(prop) = class_prop {
            match binding_fn(&prop.value) {
                None => {
                    let v = crate::helpers::expr_to_string(&prop.value);
                    code.push(quote! { #result_var.add_class(#v); });
                }
                Some(f) => code.push(quote! {
                    {
                        let __h = #result_var.clone();
                        let __f = #f;
                        let __prev = ::std::cell::RefCell::new(String::new());
                        __scope.create_effect(move || {
                            let __old = __prev.borrow().clone();
                            for __c in __old.split_whitespace() {
                                __h.remove_class(__c);
                            }
                            let __new_class = __f();
                            for __c in __new_class.split_whitespace() {
                                __h.add_class(__c);
                            }
                            *__prev.borrow_mut() = __new_class;
                        });
                    }
                }),
            }
        }

        for prop in shorthand_props {
            let css_props = expand_style_shorthand(&prop.name.to_string()).unwrap();
            let value = &prop.value;
            if get_closure_expr(value).is_none() {
                // A literal or a plain expression: resolved as its source text
                // at compile time, exactly as the static path does.
                let resolved = resolve_spacing_value(&crate::helpers::expr_to_string(value));
                for css_prop in css_props {
                    code.push(quote! { #result_var.set_style(#css_prop, #resolved); });
                }
                continue;
            }
            let f = binding_fn(value).expect("a closure is not a literal");
            for css_prop in css_props {
                code.push(quote! {
                    {
                        let __h = #result_var.clone();
                        let __f = #f;
                        __scope.create_effect(move || {
                            let __resolved = rinch::core::resolve_spacing(&__f());
                            __h.set_style(#css_prop, &__resolved);
                        });
                    }
                });
            }
        }

        Self {
            evals,
            site_self,
            per_render: quote! { #(#code)* },
        }
    }

    /// What to emit at the component site before `reactive_component_dom`,
    /// and the render closure to hand it: the plain render closure over `body`
    /// when there is no reactive binding, else a site bundle and a render
    /// closure calling it.
    fn finish(self, ctx: &DomCodegenContext, body: &TokenStream2) -> (TokenStream2, TokenStream2) {
        if self.evals.is_empty() {
            let shadows = ctx.site_shadows(&collect_body_captures(body), &no_siblings());
            let render = wrap_site(&shadows, quote! { move |__child_scope| { #body } });
            return (quote! {}, render);
        }
        let site_self = &self.site_self;
        let arms = self.evals.iter().enumerate().map(|(i, eval)| {
            let index = proc_macro2::Literal::u32_unsuffixed(i as u32);
            quote! {
                rinch::core::dom::SiteCall::Binding(#index) => rinch::core::dom::SiteOut::Str(#eval),
            }
        });
        let site_body = quote! {
            match __call {
                rinch::core::dom::SiteCall::Render(__child_scope, #site_self) => {
                    rinch::core::dom::SiteOut::Node({ #body })
                }
                #(#arms)*
                rinch::core::dom::SiteCall::Binding(_) => {
                    ::std::unreachable!("no such root binding")
                }
            }
        };
        let shadows = ctx.site_shadows(&collect_body_captures(&site_body), &no_siblings());
        let bundle = wrap_site(
            &shadows,
            quote! {
                ::std::rc::Rc::new(
                    move |__call: rinch::core::dom::SiteCall<'_>| -> rinch::core::dom::SiteOut {
                        #site_body
                    },
                )
            },
        );
        let lets = quote! {
            let __site: ::std::rc::Rc<rinch::core::dom::SiteFn> = #bundle;
        };
        let render = quote! {
            move |__child_scope: &mut rinch::core::RenderScope| {
                (__site)(rinch::core::dom::SiteCall::Render(__child_scope, &__site)).into_node()
            }
        };
        (lets, render)
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
