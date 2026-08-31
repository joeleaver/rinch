//! Control flow DOM code generation.
//!
//! Handles `Fragment` and native `if`/`for`/`match` control flow in RSX.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::element::RsxElement;
use crate::node::{RsxElseBranch, RsxForLoop, RsxIfBlock, RsxMatchBlock, RsxNode};

use super::DomCodegenContext;
use super::captures::{
    collect_body_captures, collect_capture_idents, collect_pat_idents, contested_names,
    shadow_clones, wrap_site,
};

/// Generate DOM code for a Fragment (just renders children in an invisible wrapper).
pub fn element_to_dom_fragment(element: &RsxElement, ctx: &mut DomCodegenContext) -> TokenStream2 {
    let container_var = ctx.next_var("fragment");

    // Generate children - Show/For use marker-based insertion
    let children_code: Vec<TokenStream2> = element
        .children
        .iter()
        .map(|child| super::generate_child_code(child, &container_var, ctx))
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

// ============================================================================
// Native Control Flow Codegen (if / for / match)
// ============================================================================

/// Generate DOM code for a native `if` / `else if` / `else` block in RSX.
///
/// Desugars to `show_dom()` calls. The condition is auto-wrapped in a `move ||` closure
/// to make it reactive. `else if` chains become nested `show_dom` calls.
///
/// All three closures — condition, then, else — are constructed as arguments to
/// one `show_dom` call, whichever way the condition goes, so a value two of them
/// name is moved twice. Each site therefore gets its own shadow clone of what it
/// shares (issue #223); see [`super::captures`].
pub fn generate_if_block(
    if_block: &RsxIfBlock,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let condition = &if_block.condition;

    // Build the condition closure
    let when_closure = if if_block.is_if_let {
        // if let Some(x) = expr { ... }
        // Condition: move || matches!(expr, Pattern)
        let pattern = if_block.pattern.as_ref().unwrap();
        quote! { move || matches!(#condition, #pattern) }
    } else {
        // Plain if: move || condition
        quote! { move || { #condition } }
    };
    let cond_caps = collect_capture_idents(condition);

    // Build then closure from children
    let (then_closure, then_caps) = generate_branch_closure(&if_block.then_children, if_block, ctx);

    // Build else closure
    let (else_closure, else_caps) = match &if_block.else_branch {
        Some(RsxElseBranch::Else(children)) => {
            let (closure, caps) = generate_children_closure(children, ctx);
            (Some(closure), caps)
        }
        Some(RsxElseBranch::ElseIf(inner_if)) => {
            // Nested else-if: the else branch renders a display:contents wrapper
            // containing a nested show_dom
            let wrapper_var = ctx.next_var("elif_wrap");
            ctx.push_closure_frame(HashSet::new());
            let nested = generate_if_block(inner_if, &wrapper_var, ctx);
            ctx.pop_closure_frame();
            let body = quote! {
                let #wrapper_var = __scope.create_element("div");
                #wrapper_var.set_attribute("style", "display:contents");
                #nested
                #wrapper_var
            };
            let caps = collect_body_captures(&body);
            let closure = quote! {
                move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    #body
                }
            };
            (Some(closure), caps)
        }
        None => (None, Vec::new()),
    };

    let shared = contested_names(&[&cond_caps, &then_caps, &else_caps]);
    let when_closure = wrap_site(&ctx.site_shadows(&cond_caps, &shared), when_closure);
    let then_closure = wrap_site(&ctx.site_shadows(&then_caps, &shared), then_closure);
    let else_option = match else_closure {
        Some(closure) => {
            let closure = wrap_site(&ctx.site_shadows(&else_caps, &shared), closure);
            quote! { Some(#closure) }
        }
        None => {
            quote! { None::<fn(&mut rinch::core::dom::RenderScope) -> rinch::core::dom::NodeHandle> }
        }
    };

    quote! {
        {
            rinch::core::show_dom(
                __scope,
                &#parent_var,
                #when_closure,
                #then_closure,
                #else_option
            );
        }
    }
}

/// Generate a render closure for `if` then-branch children, and the values it
/// captures from the enclosing scope.
///
/// For `if let`, the closure re-destructures the expression to bind variables —
/// which is why an `if let` scrutinee is captured by the then closure as well as
/// by the condition closure.
fn generate_branch_closure(
    children: &[RsxNode],
    if_block: &RsxIfBlock,
    ctx: &mut DomCodegenContext,
) -> (TokenStream2, Vec<syn::Ident>) {
    // An `if let` pattern binds its names inside the branch, so a closure built
    // there may move them: they are fresh on every run of the branch.
    let mut bound = HashSet::new();
    if let Some(pattern) = if_block.pattern.as_ref() {
        collect_pat_idents(pattern, &mut bound);
    }

    ctx.push_closure_frame(bound);
    let children_body = generate_children_body(children, ctx);
    ctx.pop_closure_frame();

    let body = if if_block.is_if_let {
        // Re-destructure to bind variables in the then branch
        let pattern = if_block.pattern.as_ref().unwrap();
        let condition = &if_block.condition;
        quote! {
            #[allow(unreachable_patterns)]
            let #pattern = #condition else { unreachable!() };
            #children_body
        }
    } else {
        children_body
    };

    let caps = collect_body_captures(&body);
    let closure = quote! {
        move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
            let __scope = __child_scope;
            #body
        }
    };
    (closure, caps)
}

/// Generate a render closure from a list of RSX children, and the values it
/// captures from the enclosing scope.
fn generate_children_closure(
    children: &[RsxNode],
    ctx: &mut DomCodegenContext,
) -> (TokenStream2, Vec<syn::Ident>) {
    ctx.push_closure_frame(HashSet::new());
    let body = generate_children_body(children, ctx);
    ctx.pop_closure_frame();

    let caps = collect_body_captures(&body);
    let closure = quote! {
        move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
            let __scope = __child_scope;
            #body
        }
    };
    (closure, caps)
}

/// Generate the body code for a list of RSX children, returning a single NodeHandle.
///
/// If there's one child, it's returned directly. Multiple children are wrapped
/// in a `display:contents` div. Leading `let` statements are emitted before
/// the RSX content.
fn generate_children_body(children: &[RsxNode], ctx: &mut DomCodegenContext) -> TokenStream2 {
    // Partition into leading statements and trailing RSX nodes
    let mut statements = Vec::new();
    let mut rsx_children = Vec::new();
    let mut past_statements = false;

    for child in children {
        if !past_statements {
            if let RsxNode::Statement(stmt) = child {
                // A leading `let` binds a local of *this* body, fresh on every
                // run, so a closure built below may still move it (issue #223).
                ctx.bind_statement(stmt);
                statements.push(stmt);
                continue;
            }
            past_statements = true;
        }
        rsx_children.push(child);
    }

    let stmt_code: Vec<TokenStream2> = statements.iter().map(|stmt| quote! { #stmt }).collect();

    if rsx_children.is_empty() {
        quote! {
            #(#stmt_code)*
            __scope.create_element("div")
        }
    } else if rsx_children.len() == 1 {
        // Single child — check if it's a control flow node that needs a parent
        match rsx_children[0] {
            RsxNode::IfBlock(_) | RsxNode::ForLoop(_) | RsxNode::MatchBlock(_) => {
                // Control flow nodes insert into a parent, so we need a wrapper
                let wrapper = ctx.next_var("cf_wrap");
                let child_code = super::generate_child_code(rsx_children[0], &wrapper, ctx);
                quote! {
                    {
                        #(#stmt_code)*
                        let #wrapper = __scope.create_element("div");
                        #wrapper.set_attribute("style", "display:contents");
                        #child_code
                        #wrapper
                    }
                }
            }
            _ => {
                let child_code = super::node_to_dom(rsx_children[0], ctx);
                quote! {
                    {
                        #(#stmt_code)*
                        #child_code
                    }
                }
            }
        }
    } else {
        let wrapper = ctx.next_var("branch_wrap");
        let children_code: Vec<TokenStream2> = rsx_children
            .iter()
            .map(|child| super::generate_child_code(child, &wrapper, ctx))
            .collect();
        quote! {
            {
                #(#stmt_code)*
                let #wrapper = __scope.create_element("div");
                #wrapper.set_attribute("style", "display:contents");
                #(#children_code)*
                #wrapper
            }
        }
    }
}

/// Generate DOM code for a native `for` loop in RSX.
///
/// The leading `let` statements of a `for` body that the key expression
/// actually depends on, in source order.
///
/// The key closure runs once per item on **every** reconcile pass, and is a
/// separate closure from the view. Copying the whole leading-`let` prologue into
/// it therefore re-executes each statement per item per pass — and the
/// documented per-item-state idiom puts a `Signal::new(..)` in exactly that
/// prologue:
///
/// ```ignore
/// for todo in todos.get() {
///     let editing = Signal::new(false);   // per-item state, for the *view*
///     div { key: todo.id, /* ... */ }     // the key needs `todo`, not `editing`
/// }
/// ```
///
/// Unfiltered, that mints a throwaway signal per item per reconcile, attributed
/// to the scope *enclosing* the `for` (the reconcile effect re-enters its
/// creation-time owner), so they accumulate until the whole component dies —
/// an invisible leak before #141, and an unbounded `Owned::signals` for the
/// dispose fixpoint to walk after it. Keeping only what the key reads removes
/// the allocation rather than re-homing it.
///
/// Traced backwards so a chain survives intact: if the key needs `b` and `b`'s
/// initialiser reads `a`, both are kept. Non-`let` statements are kept
/// unconditionally — they bind nothing to trace and may carry side effects the
/// key relies on.
fn key_relevant_leading_stmts(for_loop: &RsxForLoop, key_fn: &TokenStream2) -> Vec<TokenStream2> {
    let leading: Vec<&syn::Stmt> = for_loop
        .children
        .iter()
        .take_while(|c| matches!(c, RsxNode::Statement(_)))
        .filter_map(|c| match c {
            RsxNode::Statement(stmt) => Some(stmt),
            _ => None,
        })
        .collect();

    let mut needed = token_idents(key_fn);

    let mut keep = vec![false; leading.len()];
    for (i, stmt) in leading.iter().enumerate().rev() {
        let syn::Stmt::Local(local) = stmt else {
            keep[i] = true;
            continue;
        };
        let mut bound = HashSet::new();
        collect_pat_idents(&local.pat, &mut bound);
        if bound.iter().any(|name| needed.contains(name)) {
            keep[i] = true;
            if let Some(init) = &local.init {
                let expr = &init.expr;
                needed.extend(token_idents(&quote! { #expr }));
                // `let x = a else { ... }` — the divergent block can reference
                // bindings too.
                if let Some((_, diverge)) = &init.diverge {
                    needed.extend(token_idents(&quote! { #diverge }));
                }
            }
        }
    }

    leading
        .into_iter()
        .zip(keep)
        .filter(|&(_, keep)| keep)
        .map(|(stmt, _)| quote! { #stmt })
        .collect()
}

/// Every identifier that *might* be referenced by `tokens`.
///
/// Deliberately a token-level over-approximation rather than the scope-aware
/// [`collect_capture_idents`]: this drives a decision about what to **discard**,
/// so the only acceptable error is keeping too much. Two things a `syn::Expr`
/// walk misses outright, both of which would silently drop a binding the key
/// needs and turn it into a compile error in user code:
///
/// - **Macro arguments.** `syn` models a macro invocation's body as an opaque
///   `TokenStream`, so `key: format!("{}-{}", a, b)` yields no identifiers at all.
/// - **Inline format args.** In `format!("{prefix}-{}", id)`, `prefix` lives
///   inside a string *literal* and is not a token anywhere.
///
/// Field names and method names are collected too. Harmless: at worst a `let`
/// whose binding happens to share a name with a field is kept unnecessarily.
fn token_idents(tokens: &TokenStream2) -> HashSet<String> {
    use proc_macro2::TokenTree;

    let mut out = HashSet::new();
    let mut stack: Vec<TokenStream2> = vec![tokens.clone()];
    while let Some(stream) = stack.pop() {
        for tree in stream {
            match tree {
                TokenTree::Ident(id) => {
                    out.insert(id.to_string());
                }
                TokenTree::Group(g) => stack.push(g.stream()),
                TokenTree::Literal(lit) => collect_inline_format_args(&lit.to_string(), &mut out),
                TokenTree::Punct(_) => {}
            }
        }
    }
    out
}

/// Pull identifiers out of a string literal's `{...}` placeholders, so an inline
/// format arg such as the `prefix` in `format!("{prefix}-{}", id)` is seen.
///
/// Takes the leading identifier of each placeholder, stopping at `:` (the format
/// spec) — `{width$}`/`{0}`/`{}` contribute nothing. `{{` is an escaped brace and
/// is skipped. Over-collecting from an unrelated string literal is harmless.
fn collect_inline_format_args(literal: &str, out: &mut HashSet<String>) {
    let bytes: Vec<char> = literal.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '{' {
            i += 1;
            continue;
        }
        if bytes.get(i + 1) == Some(&'{') {
            i += 2; // escaped `{{`
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_alphanumeric() || bytes[end] == '_') {
            end += 1;
        }
        if end > start && !bytes[start].is_ascii_digit() {
            out.insert(bytes[start..end].iter().collect());
        }
        i = end.max(start + 1);
    }
}

/// Desugars to `for_each_dom_typed()`. The iterator expression is auto-wrapped
/// in a `move ||` closure. If a `key:` prop is found on the first child element,
/// it's extracted as the key function.
///
/// The collection, key and view closures are all constructed together and all
/// capture by value, so each gets its own shadow clone of what it shares with
/// the others (issue #223) on top of the iterator expression's unconditional one
/// (issue #26 part 3+4).
pub fn generate_for_loop(
    for_loop: &RsxForLoop,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let pattern = &for_loop.pattern;
    let iter_expr = &for_loop.iter_expr;

    // Try to extract key from first child element's `key:` prop
    let (key_fn, key_source) = extract_key_expr(for_loop);

    // Leading `let`s the key expression depends on, so `key:` can reference a
    // let-bound value. Deliberately filtered — see `key_relevant_leading_stmts`.
    let leading_stmts = key_relevant_leading_stmts(for_loop, &key_fn);

    // The loop pattern binds the item for the key and view closures, so neither
    // captures it and neither may shadow-clone it.
    let mut item_bound = HashSet::new();
    collect_pat_idents(pattern, &mut item_bound);

    // Build the view closure body from children
    ctx.push_closure_frame(item_bound.clone());
    let body = generate_children_body(&for_loop.children, ctx);
    ctx.pop_closure_frame();

    // Collection closure: move || iter_expr.into_iter().collect::<Vec<_>>()
    let collection = quote! {
        move || (#iter_expr).into_iter().collect::<Vec<_>>()
    };

    // Key closure: include leading let statements so key: can reference let-bound values
    let key_body = if leading_stmts.is_empty() {
        quote! { ::std::string::ToString::to_string(&#key_fn) }
    } else {
        quote! { { #(#leading_stmts)* ::std::string::ToString::to_string(&#key_fn) } }
    };
    let key_closure = quote! { move |#pattern| #key_body };

    let view_closure = quote! {
        move |#pattern, __child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
            let __scope = __child_scope;
            #body
        }
    };

    // Issue #26 part 3: pre-clone identifiers referenced by `iter_expr` so the
    // `move ||` collection closure can construct cleanly when the enclosing
    // scope is itself a non-FnOnce closure (e.g. an `if`/`match` branch). The
    // outer closure stays untouched; each call constructs a fresh shadow that
    // the inner closure consumes. Copy types' `.clone()` is a no-op.
    let iter_caps = collect_capture_idents(iter_expr);
    let key_caps = without(collect_body_captures(&key_body), &item_bound);
    let view_caps = without(collect_body_captures(&body), &item_bound);
    let shared = contested_names(&[&iter_caps, &key_caps, &view_caps]);

    let collection = wrap_site(&shadow_clones(iter_caps.iter()), collection);
    let key_closure = wrap_site(&ctx.site_shadows(&key_caps, &shared), key_closure);
    let view_closure = wrap_site(&ctx.site_shadows(&view_caps, &shared), view_closure);

    quote! {
        {
            rinch::core::for_each_dom_typed_with_key_source(
                __scope,
                &#parent_var,
                #collection,
                #key_closure,
                #view_closure,
                #key_source
            );
        }
    }
}

/// Drop the names `bound` introduces from a capture list — they are the
/// closure's own parameters, not values reaching it from the enclosing scope.
fn without(caps: Vec<syn::Ident>, bound: &HashSet<String>) -> Vec<syn::Ident> {
    caps.into_iter()
        .filter(|id| !bound.contains(&id.to_string()))
        .collect()
}

/// Extract a `key:` expression from the first child element of a for loop.
///
/// Returns the key expression token stream **and** a `rinch::core::KeySource`
/// expression saying who chose it. If no `key:` prop is found the key falls back
/// to Debug formatting the item, and the `KeySource::Fallback` marker tells
/// `for_each_dom` that a repeat of such a key is not a user error: `for tag in
/// ["rust", "rust", "gui"]` is an ordinary list, so the fabricated key is
/// uniquified by occurrence ordinal rather than the row being dropped (issue
/// #185).
///
/// Note: The `key:` prop is left on the element. HTML codegen treats `key`
/// as a special attribute and skips it (it would just become a harmless
/// `set_attribute("key", ...)` otherwise).
fn extract_key_expr(for_loop: &RsxForLoop) -> (TokenStream2, TokenStream2) {
    // Look for key: prop on the first child element (skip leading let statements)
    let first_element = for_loop
        .children
        .iter()
        .find(|c| !matches!(c, RsxNode::Statement(_)));

    if let Some(RsxNode::Element(el)) = first_element
        && let Some(key_prop) = el.props.iter().find(|p| p.name == "key")
    {
        let key_expr = &key_prop.value;
        return (
            quote! { #key_expr },
            quote! { rinch::core::KeySource::Explicit },
        );
    }

    // No key prop found — use debug format of the item as key (fallback)
    let pattern = &for_loop.pattern;
    (
        quote! { format!("{:?}", #pattern) },
        quote! { rinch::core::KeySource::Fallback },
    )
}

/// Generate DOM code for a native `match` block in RSX.
///
/// Desugars to `match_dom()`. The scrutinee is evaluated in a discriminant closure
/// that returns a branch index. Each arm becomes a boxed render closure.
///
/// The scrutinee is emitted once per arm on top of the discriminant, and every
/// arm closure is constructed — so a `match` shares the `if`/`else` defect twice
/// over, and takes the same per-site shadow clones (issue #223).
pub fn generate_match_block(
    match_block: &RsxMatchBlock,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let scrutinee = &match_block.scrutinee;
    let num_arms = match_block.arms.len();

    // Build the discriminant closure: move || match scrutinee { pat => 0, pat => 1, ... }
    let discriminant_arms: Vec<TokenStream2> = match_block
        .arms
        .iter()
        .enumerate()
        .map(|(i, arm)| {
            let pat = &arm.pattern;
            let idx = i;
            if let Some(ref guard) = arm.guard {
                quote! { #pat if #guard => #idx, }
            } else {
                quote! { #pat => #idx, }
            }
        })
        .collect();

    let discriminant = quote! {
        move || -> usize {
            #[allow(unreachable_patterns)]
            match #scrutinee {
                #(#discriminant_arms)*
                _ => #num_arms, // out-of-range = no branch rendered
            }
        }
    };
    let discriminant_caps = collect_capture_idents(scrutinee);

    // Build branch bodies. Each one re-evaluates the scrutinee to bind pattern
    // variables, using `match` with the specific pattern + a catch-all
    // unreachable.
    let arm_bodies: Vec<(TokenStream2, Vec<syn::Ident>)> = match_block
        .arms
        .iter()
        .map(|arm| {
            let pat = &arm.pattern;
            let guard_check = arm.guard.as_ref().map(|g| quote! { if #g });

            // The arm pattern binds its names inside the arm body.
            let mut bound = HashSet::new();
            collect_pat_idents(pat, &mut bound);
            ctx.push_closure_frame(bound);
            let body = generate_children_body(&arm.children, ctx);
            ctx.pop_closure_frame();

            let arm_body = quote! {
                #[allow(unreachable_patterns, unused_variables, irrefutable_let_patterns)]
                match #scrutinee {
                    #pat #guard_check => { #body }
                    _ => unreachable!()
                }
            };
            let caps = collect_body_captures(&arm_body);
            (arm_body, caps)
        })
        .collect();

    let mut sites: Vec<&[syn::Ident]> = vec![&discriminant_caps];
    sites.extend(arm_bodies.iter().map(|(_, caps)| caps.as_slice()));
    let shared = contested_names(&sites);

    let discriminant = wrap_site(&ctx.site_shadows(&discriminant_caps, &shared), discriminant);
    let branch_closures: Vec<TokenStream2> = arm_bodies
        .iter()
        .map(|(arm_body, caps)| {
            let closure = quote! {
                move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    #arm_body
                }
            };
            let closure = wrap_site(&ctx.site_shadows(caps, &shared), closure);
            quote! {
                Box::new(#closure) as Box<dyn Fn(&mut rinch::core::dom::RenderScope) -> rinch::core::dom::NodeHandle>
            }
        })
        .collect();

    quote! {
        {
            rinch::core::match_dom(
                __scope,
                &#parent_var,
                #discriminant,
                vec![#(#branch_closures),*]
            );
        }
    }
}

#[cfg(test)]
mod key_prologue_tests {
    use super::key_relevant_leading_stmts;
    use crate::node::RsxForLoop;
    use quote::quote;

    /// Render the statements `key_relevant_leading_stmts` would copy into the
    /// key closure, as one whitespace-free string.
    fn prologue(for_body: proc_macro2::TokenStream, key: proc_macro2::TokenStream) -> String {
        let for_loop: RsxForLoop = syn::parse2(for_body).expect("parse for loop");
        key_relevant_leading_stmts(&for_loop, &key)
            .iter()
            .map(|ts| ts.to_string())
            .collect::<String>()
            .replace(' ', "")
    }

    /// The documented per-item-state idiom must not be re-executed by the key
    /// closure: `Signal::new` there mints a throwaway signal per item per
    /// reconcile pass, attributed to the scope enclosing the `for` (issue #141).
    #[test]
    fn per_item_state_is_not_copied_into_the_key_closure() {
        let copied = prologue(
            quote! {
                for todo in todos.get() {
                    let editing = Signal::new(false);
                    div { key: todo.id, "x" }
                }
            },
            quote! { todo.id },
        );
        assert_eq!(
            copied, "",
            "the key reads only `todo`, so nothing needs re-running"
        );
    }

    /// The reason the prologue is copied at all still works: a key that reads a
    /// let-bound value keeps the binding that produced it.
    #[test]
    fn a_binding_the_key_reads_is_kept() {
        let copied = prologue(
            quote! {
                for todo in todos.get() {
                    let composite = format!("{}-{}", todo.id, todo.rev);
                    div { key: composite, "x" }
                }
            },
            quote! { composite },
        );
        assert!(
            copied.contains("letcomposite"),
            "the key depends on `composite`, so its binding must survive: {copied}"
        );
    }

    /// A key built with a macro keeps its dependencies.
    ///
    /// This is why the analysis is token-level: `syn` models a macro body as an
    /// opaque `TokenStream`, so an expression walk sees *no* identifiers in
    /// `format!("{}", part)` at all and would drop `part` — turning the filter
    /// from an optimisation into a compile error in user code.
    #[test]
    fn a_key_built_by_a_macro_keeps_its_dependencies() {
        let positional = prologue(
            quote! {
                for todo in todos.get() {
                    let part = todo.id.to_string();
                    let editing = Signal::new(false);
                    div { key: format!("{}-x", part), "x" }
                }
            },
            quote! { format!("{}-x", part) },
        );
        assert!(positional.contains("letpart"), "{positional}");
        assert!(!positional.contains("letediting"), "{positional}");

        // The same, with an inline format arg — the identifier lives inside a
        // string literal and is not a token anywhere.
        let inline = prologue(
            quote! {
                for todo in todos.get() {
                    let part = todo.id.to_string();
                    let editing = Signal::new(false);
                    div { key: format!("{part}-x"), "x" }
                }
            },
            quote! { format!("{part}-x") },
        );
        assert!(inline.contains("letpart"), "{inline}");
        assert!(!inline.contains("letediting"), "{inline}");
    }

    /// Dependencies are traced backwards through a chain, and unrelated
    /// bindings interleaved with it are still dropped.
    #[test]
    fn a_dependency_chain_is_kept_and_unrelated_bindings_are_not() {
        let copied = prologue(
            quote! {
                for todo in todos.get() {
                    let prefix = todo.group.clone();
                    let editing = Signal::new(false);
                    let composite = format!("{prefix}-{}", todo.id);
                    div { key: composite, "x" }
                }
            },
            quote! { composite },
        );
        assert!(copied.contains("letcomposite"), "{copied}");
        assert!(
            copied.contains("letprefix"),
            "`composite` reads `prefix`, so the chain must survive: {copied}"
        );
        assert!(
            !copied.contains("letediting"),
            "`editing` is not on the chain and must be dropped: {copied}"
        );
    }
}

#[cfg(test)]
mod shadow_tests {
    use super::{generate_for_loop, generate_if_block, generate_match_block};
    use crate::dom_codegen::DomCodegenContext;
    use crate::node::{RsxForLoop, RsxIfBlock, RsxMatchBlock};
    use quote::quote;

    fn parent() -> syn::Ident {
        syn::Ident::new("__parent", proc_macro2::Span::call_site())
    }

    /// The generated code for one `rsx!` construct, whitespace-free so the
    /// shadow bindings can be matched as substrings.
    fn if_code(src: proc_macro2::TokenStream) -> String {
        let block: RsxIfBlock = syn::parse2(src).expect("parse if");
        generate_if_block(&block, &parent(), &mut DomCodegenContext::new())
            .to_string()
            .replace(' ', "")
    }

    fn for_code(src: proc_macro2::TokenStream) -> String {
        let block: RsxForLoop = syn::parse2(src).expect("parse for");
        generate_for_loop(&block, &parent(), &mut DomCodegenContext::new())
            .to_string()
            .replace(' ', "")
    }

    fn match_code(src: proc_macro2::TokenStream) -> String {
        let block: RsxMatchBlock = syn::parse2(src).expect("parse match");
        generate_match_block(&block, &parent(), &mut DomCodegenContext::new())
            .to_string()
            .replace(' ', "")
    }

    /// `let mut #name = Clone::clone(&#name);` as it appears in the output.
    fn shadow(name: &str) -> String {
        format!("letmut{name}=::std::clone::Clone::clone(&{name});")
    }

    /// The headline case: a value both branches name is cloned once per branch,
    /// so each closure owns a copy and the enclosing scope keeps the original.
    #[test]
    fn a_value_two_branches_share_is_cloned_at_each_site() {
        let code = if_code(quote! {
            if shown.get() {
                p { {label.clone()} }
            } else {
                span { {label.clone()} }
            }
        });
        assert_eq!(
            code.matches(&shadow("label")).count(),
            2,
            "one shadow per branch: {code}"
        );
    }

    /// The condition is a third closure, constructed with the branches.
    #[test]
    fn a_value_the_condition_shares_with_a_branch_is_cloned() {
        let code = if_code(quote! {
            if label.is_empty() {
                p { {label.clone()} }
            }
        });
        assert_eq!(
            code.matches(&shadow("label")).count(),
            2,
            "condition and branch each get one: {code}"
        );
    }

    /// The over-cloning guard, stated in the codegen rather than in rustc: a
    /// value only one site names is moved, not cloned — including a `Signal`,
    /// which is `Copy` precisely so users never have to clone it.
    #[test]
    fn a_value_only_one_site_names_is_not_cloned() {
        let code = if_code(quote! {
            if shown.get() {
                p { {only_then.clone()} }
            } else {
                span { {only_else.clone()} }
            }
        });
        assert!(!code.contains(&shadow("only_then")), "{code}");
        assert!(!code.contains(&shadow("only_else")), "{code}");
        assert!(!code.contains(&shadow("shown")), "{code}");
    }

    /// Two signals in two branches are two distinct names, so nothing is
    /// contested and nothing is cloned — `Signal` and `Memo` are `Copy`
    /// precisely so users never write `.clone()` on them.
    #[test]
    fn distinct_signals_in_sibling_branches_are_not_cloned() {
        let code = if_code(quote! {
            if toggled.get() {
                p { {left.get().to_string()} }
            } else {
                p { {right.get().to_string()} }
            }
        });
        assert!(!code.contains(&shadow("left")), "{code}");
        assert!(!code.contains(&shadow("right")), "{code}");
        assert!(!code.contains(&shadow("toggled")), "{code}");
    }

    /// A reactive binding *inside* a branch is a different matter: the effect
    /// is a `'static move` closure rebuilt on every render of that branch, so
    /// it does take a shadow — one, from the branch's own copy. On a `Copy`
    /// `Signal` that clone is a copy; on a `String` it is the difference
    /// between compiling and not.
    #[test]
    fn an_effect_inside_a_branch_shadows_what_it_captures() {
        let code = if_code(quote! {
            if toggled.get() {
                p { {|| left.get().to_string()} }
            }
        });
        assert_eq!(code.matches(&shadow("left")).count(), 1, "{code}");
        assert!(!code.contains(&shadow("toggled")), "{code}");
    }

    /// A `match` scrutinee is re-emitted by the discriminant and by every arm.
    #[test]
    fn a_match_scrutinee_is_cloned_for_the_discriminant_and_each_arm() {
        let code = match_code(quote! {
            match label.as_str() {
                "a" => p { "first" },
                _ => span { "rest" },
            }
        });
        assert_eq!(
            code.matches(&shadow("label")).count(),
            3,
            "discriminant + two arms: {code}"
        );
    }

    /// An arm's own pattern binding is not a capture and is never cloned.
    #[test]
    fn a_match_arm_binding_is_not_cloned() {
        let code = match_code(quote! {
            match slot.clone() {
                Some(name) => p { {name} },
                None => span { "none" },
            }
        });
        assert!(!code.contains(&shadow("name")), "{code}");
    }

    /// The iterator expression keeps its unconditional shadow (issue #26), and
    /// a value the per-item view shares with it gets one of its own.
    #[test]
    fn a_for_clones_its_iterator_source_and_anything_the_view_shares() {
        let code = for_code(quote! {
            for name in names.clone() {
                p { key: name.clone(), {names.len().to_string()} }
            }
        });
        assert_eq!(
            code.matches(&shadow("names")).count(),
            2,
            "collection closure + view closure: {code}"
        );
    }

    /// The loop pattern binds the item; shadow-cloning it would name a binding
    /// the enclosing scope does not have.
    #[test]
    fn a_for_never_clones_its_own_item() {
        let code = for_code(quote! {
            for item in items.get() {
                p { key: item.id, {item.name.clone()} }
            }
        });
        assert!(!code.contains(&shadow("item")), "{code}");
    }
}
