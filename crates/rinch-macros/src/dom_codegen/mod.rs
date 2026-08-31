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

pub(crate) mod captures;
mod component;
pub mod component_codegen;
mod control_flow;
pub mod helpers;
pub mod html;

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::helpers::{get_closure_expr, is_literal_expr};
use crate::node::RsxNode;

use captures::{collect_capture_idents, is_move_closure, shadow_clones};

/// Context for DOM code generation.
pub struct DomCodegenContext {
    /// Counter for generating unique variable names.
    var_counter: usize,
    /// Whether we're inside an effect (for nested reactive expressions).
    #[allow(dead_code)]
    in_effect: bool,
    /// One frame per enclosing generated closure body, innermost last, each
    /// holding the names that body binds for itself (`let` statements, the
    /// `for` pattern, the `if let` pattern, a `match` arm's pattern).
    ///
    /// `closure_frames[0]` is the `rsx!` root: code there runs once, so nothing
    /// it constructs can move out of a repeated capture. Every deeper frame is
    /// a body that runs more than once — a branch, an arm, a per-item view, an
    /// effect — which is what makes a name from *outside* it contested
    /// (issue #223). See [`captures`].
    closure_frames: Vec<HashSet<String>>,
}

impl DomCodegenContext {
    pub fn new() -> Self {
        Self {
            var_counter: 0,
            in_effect: false,
            closure_frames: vec![HashSet::new()],
        }
    }

    /// Enter a generated closure body that may run more than once. `bound` is
    /// what the body binds for itself before any of its RSX is generated.
    pub(crate) fn push_closure_frame(&mut self, bound: HashSet<String>) {
        self.closure_frames.push(bound);
    }

    pub(crate) fn pop_closure_frame(&mut self) {
        self.closure_frames.pop();
    }

    /// Record the names a `let` statement of the current body binds, so a
    /// closure built later in that body may still move them: they are a fresh
    /// local on every run, not a capture from outside.
    pub(crate) fn bind_statement(&mut self, stmt: &syn::Stmt) {
        let syn::Stmt::Local(local) = stmt else {
            return;
        };
        if let Some(frame) = self.closure_frames.last_mut() {
            captures::collect_pat_idents(&local.pat, frame);
        }
    }

    /// Whether `name` reaches this site from outside the repeatable body that
    /// encloses it — i.e. building a `'static move` closure from it here would
    /// move out of an `Fn`/`FnMut` capture, which only compiles today when the
    /// value is `Copy`.
    fn is_outer_capture(&self, name: &str) -> bool {
        self.closure_frames.len() > 1
            && !self
                .closure_frames
                .last()
                .is_some_and(|frame| frame.contains(name))
    }

    /// The shadow-clone prologue for one closure-construction site.
    ///
    /// `caps` is what the site's closure captures; `shared` is what sibling
    /// sites of the same construct also capture. A name is cloned when it is
    /// contested either way, and left alone otherwise — see [`captures`] for
    /// why both cases are provably `Copy` today.
    pub(crate) fn site_shadows(
        &self,
        caps: &[syn::Ident],
        shared: &HashSet<String>,
    ) -> TokenStream2 {
        shadow_clones(caps.iter().filter(|id| {
            let name = id.to_string();
            shared.contains(&name) || self.is_outer_capture(&name)
        }))
    }

    /// Generate a unique variable name.
    pub(crate) fn next_var(&mut self, prefix: &str) -> syn::Ident {
        let name = format!("__{}{}", prefix, self.var_counter);
        self.var_counter += 1;
        syn::Ident::new(&name, proc_macro2::Span::call_site())
    }
}

/// Nothing is contested with a sibling site — used where a construct has only
/// one closure to build.
pub(crate) fn no_siblings() -> HashSet<String> {
    HashSet::new()
}

impl Default for DomCodegenContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate child code for a parent element.
///
/// Control flow elements use marker-based rendering and insert directly
/// into the parent. Other children are appended normally.
pub(crate) fn generate_child_code(
    child: &RsxNode,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    match child {
        RsxNode::IfBlock(if_block) => {
            // Native if/else inserts marker + content directly into parent
            control_flow::generate_if_block(if_block, parent_var, ctx)
        }
        RsxNode::ForLoop(for_loop) => {
            // Native for loop inserts marker + items directly into parent
            control_flow::generate_for_loop(for_loop, parent_var, ctx)
        }
        RsxNode::MatchBlock(match_block) => {
            // Native match inserts marker + content directly into parent
            control_flow::generate_match_block(match_block, parent_var, ctx)
        }
        RsxNode::Statement(stmt) => {
            // Emit statement directly (e.g., `let x = ...;`). Its bindings are
            // locals of the body being generated, so a closure built later in
            // that body may move them (issue #223).
            ctx.bind_statement(stmt);
            quote! { #stmt }
        }
        RsxNode::Element(element) if component_codegen::has_reactive_component_props(element) => {
            // Reactive components insert directly into parent (like control flow)
            // to avoid display:contents wrapper divs that Taffy can't layout
            component_codegen::generate_reactive_component_stmt(element, parent_var, ctx)
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

    // Special handling for Fragment - just wraps children in a span
    if name == "Fragment" {
        return control_flow::element_to_dom_fragment(element, ctx);
    }

    // Special handling for ThemeProvider - wraps children with reactive theme
    if name == "ThemeProvider" {
        return component::element_to_dom_theme_provider(element, ctx);
    }

    // Check if this is a rinch component (handled differently)
    if element.is_rinch_component() {
        return component::element_to_dom_component(element, ctx);
    }

    // Generate HTML element
    html::element_to_dom_html(element, ctx)
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
                // Reactive closure: create an EMPTY text node and let an effect
                // fill it. `create_effect` runs the effect immediately (Effect::new),
                // so the initial value is set right away — the same single-emission
                // pattern the attribute codegen uses. Emitting the user closure
                // once (not once for the initial value and again in the effect)
                // avoids running its body twice on mount (double-firing side
                // effects) and keeps reactive text consistent with reactive
                // attributes (issue #102).
                let text_var = ctx.next_var("text");
                // The effect is a `move` closure that rebuilds `#closure` on
                // every fire, so it competes twice for what that closure names:
                // once with the body it is built in (the site shadow), and once
                // with itself on the next fire (the per-fire shadow) — issue
                // #223. A borrowing closure only reads through the effect's own
                // capture and needs neither.
                let caps = collect_capture_idents(closure);
                let site_shadows = ctx.site_shadows(&caps, &no_siblings());
                let fire_shadows = if is_move_closure(closure) {
                    shadow_clones(caps.iter())
                } else {
                    quote! {}
                };
                quote! {
                    {
                        let #text_var = __scope.create_text("");
                        let __text_clone = #text_var.clone();
                        #site_shadows
                        __scope.create_effect(move || {
                            #fire_shadows
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
                    rinch::core::IntoNode::into_node(#expr, __scope)
                }
            }
        }
        // Native control flow at top level: wrap in display:contents div
        // since these need a parent to insert into.
        RsxNode::IfBlock(if_block) => {
            let wrapper_var = ctx.next_var("cf_wrapper");
            let inner = control_flow::generate_if_block(if_block, &wrapper_var, ctx);
            quote! {
                {
                    let #wrapper_var = __scope.create_element("div");
                    #wrapper_var.set_attribute("style", "display:contents");
                    #inner
                    #wrapper_var
                }
            }
        }
        RsxNode::ForLoop(for_loop) => {
            let wrapper_var = ctx.next_var("cf_wrapper");
            let inner = control_flow::generate_for_loop(for_loop, &wrapper_var, ctx);
            quote! {
                {
                    let #wrapper_var = __scope.create_element("div");
                    #wrapper_var.set_attribute("style", "display:contents");
                    #inner
                    #wrapper_var
                }
            }
        }
        RsxNode::MatchBlock(match_block) => {
            let wrapper_var = ctx.next_var("cf_wrapper");
            let inner = control_flow::generate_match_block(match_block, &wrapper_var, ctx);
            quote! {
                {
                    let #wrapper_var = __scope.create_element("div");
                    #wrapper_var.set_attribute("style", "display:contents");
                    #inner
                    #wrapper_var
                }
            }
        }
        RsxNode::Statement(stmt) => {
            // Statements at top level just emit the code (unusual but valid)
            quote! { { #stmt __scope.create_element("div") } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::helpers::is_likely_reactive;
    use syn::{Expr, parse_quote};

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
