//! Control flow DOM code generation.
//!
//! Handles `Fragment` and native `if`/`for`/`match` control flow in RSX.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::visit::Visit;

use crate::element::RsxElement;
use crate::node::{RsxElseBranch, RsxForLoop, RsxIfBlock, RsxMatchBlock, RsxNode};

use super::DomCodegenContext;

/// Collect simple identifier references in `expr` that look like captures of
/// the surrounding scope (local variables / function parameters), as a fix for
/// the nested `if { for { ... } }` capture conflict (issue #26 part 3+4).
///
/// **Heuristics** — we don't have type info in the macro, so this filters
/// based on naming conventions:
///
/// - Single-segment path expressions only (`foo`, not `foo::bar` or `Module::FOO`)
/// - Identifier's first character must be lowercase (filters out PascalCase types
///   like `String`, `Vec`, user-defined `MyStruct`)
/// - Excludes Rust keywords/literals (`self`, `true`, `false`)
/// - Excludes macro-internal names (anything starting with `__`)
/// - Excludes identifiers introduced by closure parameters or `let` bindings
///   inside the expression itself (issue #32 — `.filter(|b| b % 4 == 0)` must
///   not shadow a non-existent outer `b`). Scope-tracked via a stack pushed on
///   `Closure` / `Block` entry and popped on exit.
/// - Excludes identifiers bound by `if let` / `while let` (incl. `&&`
///   let-chains) patterns and `match` arm patterns, each scoped to the branch
///   that can actually see them — the `if let` analogue of the #32 fix.
///
/// The collected list is used to emit `let #id = #id.clone();` shadow bindings
/// before the inner `move ||` closure is constructed. Cloning Copy types via
/// `.clone()` is a no-op; the shadow only matters for non-Copy values like
/// `String`. Types that don't impl `Clone` will still fail with a clear error
/// pointing at the field — same behaviour as user code calling `.clone()` directly.
fn collect_capture_idents(expr: &syn::Expr) -> Vec<syn::Ident> {
    struct Collector {
        idents: Vec<syn::Ident>,
        seen: HashSet<String>,
        /// Stack of locally-bound identifier frames. Each frame holds names
        /// introduced by a closure's params or by `let` bindings inside a block.
        /// `is_locally_bound` checks every frame; frames pop in LIFO order on
        /// scope exit.
        locals: Vec<HashSet<String>>,
    }

    impl Collector {
        fn is_locally_bound(&self, name: &str) -> bool {
            self.locals.iter().any(|frame| frame.contains(name))
        }

        /// Scan an `if`/`while` condition that may introduce `let` bindings
        /// (`if let`, `while let`, and `&&` let-chains). Each scrutinee
        /// expression is visited so its identifiers resolve against the
        /// *current* scope — which already includes bindings from earlier links
        /// of a let-chain (`if let Some(x) = a && x > 0`) — while the names
        /// bound by each pattern are added to the top `locals` frame so the
        /// branch body (pushed by the caller) skips them. A plain boolean
        /// condition just falls through to a normal visit.
        fn scan_cond<'ast>(&mut self, cond: &'ast syn::Expr) {
            match cond {
                syn::Expr::Let(let_expr) => {
                    self.visit_expr(&let_expr.expr);
                    if let Some(frame) = self.locals.last_mut() {
                        collect_pat_idents(&let_expr.pat, frame);
                    }
                }
                syn::Expr::Binary(bin) if matches!(bin.op, syn::BinOp::And(_)) => {
                    self.scan_cond(&bin.left);
                    self.scan_cond(&bin.right);
                }
                other => self.visit_expr(other),
            }
        }
    }

    impl<'ast> Visit<'ast> for Collector {
        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            // Only single-segment paths with no generics / no leading `::`.
            if expr.qself.is_some()
                || expr.path.leading_colon.is_some()
                || expr.path.segments.len() != 1
            {
                syn::visit::visit_expr_path(self, expr);
                return;
            }
            let seg = &expr.path.segments[0];
            if !seg.arguments.is_none() {
                syn::visit::visit_expr_path(self, expr);
                return;
            }
            let name = seg.ident.to_string();
            let first = name.chars().next().unwrap_or('_');
            let is_likely_local = first.is_ascii_lowercase()
                && !name.starts_with("__")
                && !matches!(name.as_str(), "self" | "true" | "false");
            if is_likely_local && !self.is_locally_bound(&name) && self.seen.insert(name) {
                self.idents.push(seg.ident.clone());
            }
            syn::visit::visit_expr_path(self, expr);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            // Only visit the receiver — method names are not captures.
            self.visit_expr(&call.receiver);
            for arg in &call.args {
                self.visit_expr(arg);
            }
        }

        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            // Skip the function expression if it's a plain path (the function
            // name), only descend into arguments. Function names aren't moved.
            if !matches!(*call.func, syn::Expr::Path(_)) {
                self.visit_expr(&call.func);
            }
            for arg in &call.args {
                self.visit_expr(arg);
            }
        }

        fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
            // Closure params bind names visible only inside the body — e.g. the
            // `b` in `.filter(|b| b % 4 == 0)`. Push a frame for the duration of
            // the closure so `visit_expr_path` skips them.
            let mut frame = HashSet::new();
            for input in &closure.inputs {
                collect_pat_idents(input, &mut frame);
            }
            self.locals.push(frame);
            syn::visit::visit_expr_closure(self, closure);
            self.locals.pop();
        }

        fn visit_block(&mut self, block: &'ast syn::Block) {
            // Each block introduces a scope for any `let` bindings within it.
            self.locals.push(HashSet::new());
            for stmt in &block.stmts {
                self.visit_stmt(stmt);
            }
            self.locals.pop();
        }

        fn visit_local(&mut self, local: &'ast syn::Local) {
            // Visit the init expression **first** — names in the RHS resolve
            // against the *outer* scope (`let x = x + 1` reads outer `x`), so
            // the new binding only enters scope after the init runs.
            if let Some(init) = &local.init {
                self.visit_expr(&init.expr);
                if let Some((_, diverge)) = &init.diverge {
                    self.visit_expr(diverge);
                }
            }
            if let Some(frame) = self.locals.last_mut() {
                collect_pat_idents(&local.pat, frame);
            }
        }

        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            // `if let PAT = scrutinee { then } else { else }` binds the names in
            // `PAT` only within `then` — NOT the scrutinee and NOT the else
            // branch. The default visitor doesn't know this, so it would treat
            // those names as outer captures and emit `let name = name.clone();`
            // for a binding that doesn't exist (the `if let` analogue of #32).
            // A plain `if cond` leaves the pushed frame empty and is unaffected.
            self.locals.push(HashSet::new());
            self.scan_cond(&node.cond);
            self.visit_block(&node.then_branch);
            self.locals.pop();
            if let Some((_, else_branch)) = &node.else_branch {
                self.visit_expr(else_branch);
            }
        }

        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            // `while let PAT = scrutinee { body }` — same scoping as `if let`,
            // with the pattern visible in the body.
            self.locals.push(HashSet::new());
            self.scan_cond(&node.cond);
            self.visit_block(&node.body);
            self.locals.pop();
        }

        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            // A `match` arm pattern binds names visible in the arm's guard and
            // body only. The scrutinee is visited by `visit_expr_match` in the
            // outer scope, so we only scope the arm here.
            let mut frame = HashSet::new();
            collect_pat_idents(&arm.pat, &mut frame);
            self.locals.push(frame);
            if let Some((_, guard)) = &arm.guard {
                self.visit_expr(guard);
            }
            self.visit_expr(&arm.body);
            self.locals.pop();
        }
    }

    let mut collector = Collector {
        idents: Vec::new(),
        seen: HashSet::new(),
        locals: Vec::new(),
    };
    collector.visit_expr(expr);
    collector.idents
}

/// Add every identifier *bound by* `pat` to `out`. Walks tuple/struct/tuple-struct
/// and or-patterns; ignores paths/literals/wildcards/ranges (those don't bind).
///
/// Used to determine which names a closure's params or a `let` introduces, so the
/// capture scanner can skip them.
fn collect_pat_idents(pat: &syn::Pat, out: &mut HashSet<String>) {
    use syn::Pat;
    match pat {
        Pat::Ident(pat_ident) => {
            out.insert(pat_ident.ident.to_string());
            if let Some((_, sub)) = &pat_ident.subpat {
                collect_pat_idents(sub, out);
            }
        }
        Pat::Tuple(t) => {
            for p in &t.elems {
                collect_pat_idents(p, out);
            }
        }
        Pat::TupleStruct(t) => {
            for p in &t.elems {
                collect_pat_idents(p, out);
            }
        }
        Pat::Struct(s) => {
            for field in &s.fields {
                collect_pat_idents(&field.pat, out);
            }
        }
        Pat::Or(o) => {
            for p in &o.cases {
                collect_pat_idents(p, out);
            }
        }
        Pat::Reference(r) => collect_pat_idents(&r.pat, out),
        Pat::Paren(p) => collect_pat_idents(&p.pat, out),
        Pat::Slice(s) => {
            for p in &s.elems {
                collect_pat_idents(p, out);
            }
        }
        Pat::Type(t) => collect_pat_idents(&t.pat, out),
        // No-binding patterns:
        // - Wild, Lit, Range, Rest, Path, Const, Macro, Verbatim
        _ => {}
    }
}

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

    // Build then closure from children
    let then_closure = generate_branch_closure(&if_block.then_children, if_block, ctx);

    // Build else closure
    let else_option = match &if_block.else_branch {
        Some(RsxElseBranch::Else(children)) => {
            let else_closure = generate_children_closure(children, ctx);
            quote! { Some(#else_closure) }
        }
        Some(RsxElseBranch::ElseIf(inner_if)) => {
            // Nested else-if: the else branch renders a display:contents wrapper
            // containing a nested show_dom
            let wrapper_var = ctx.next_var("elif_wrap");
            let nested = generate_if_block(inner_if, &wrapper_var, ctx);
            quote! {
                Some(move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    let #wrapper_var = __scope.create_element("div");
                    #wrapper_var.set_attribute("style", "display:contents");
                    #nested
                    #wrapper_var
                })
            }
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

/// Generate a render closure for `if` then-branch children.
///
/// For `if let`, the closure re-destructures the expression to bind variables.
fn generate_branch_closure(
    children: &[RsxNode],
    if_block: &RsxIfBlock,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let body = generate_children_body(children, ctx);

    if if_block.is_if_let {
        // Re-destructure to bind variables in the then branch
        let pattern = if_block.pattern.as_ref().unwrap();
        let condition = &if_block.condition;
        quote! {
            move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                let __scope = __child_scope;
                #[allow(unreachable_patterns)]
                let #pattern = #condition else { unreachable!() };
                #body
            }
        }
    } else {
        quote! {
            move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                let __scope = __child_scope;
                #body
            }
        }
    }
}

/// Generate a render closure from a list of RSX children.
fn generate_children_closure(children: &[RsxNode], ctx: &mut DomCodegenContext) -> TokenStream2 {
    let body = generate_children_body(children, ctx);
    quote! {
        move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
            let __scope = __child_scope;
            #body
        }
    }
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
/// Desugars to `for_each_dom_typed()`. The iterator expression is auto-wrapped
/// in a `move ||` closure. If a `key:` prop is found on the first child element,
/// it's extracted as the key function.
pub fn generate_for_loop(
    for_loop: &RsxForLoop,
    parent_var: &syn::Ident,
    ctx: &mut DomCodegenContext,
) -> TokenStream2 {
    let pattern = &for_loop.pattern;
    let iter_expr = &for_loop.iter_expr;

    // Try to extract key from first child element's `key:` prop
    let key_fn = extract_key_expr(for_loop);

    // Collect leading let statements so they can be included in the key closure too
    let leading_stmts: Vec<TokenStream2> = for_loop
        .children
        .iter()
        .take_while(|c| matches!(c, RsxNode::Statement(_)))
        .filter_map(|c| {
            if let RsxNode::Statement(stmt) = c {
                Some(quote! { #stmt })
            } else {
                None
            }
        })
        .collect();

    // Build the view closure body from children
    let body = generate_children_body(&for_loop.children, ctx);

    // Collection closure: move || iter_expr.into_iter().collect::<Vec<_>>()
    let collection = quote! {
        move || (#iter_expr).into_iter().collect::<Vec<_>>()
    };

    // Key closure: include leading let statements so key: can reference let-bound values
    let key_closure = if leading_stmts.is_empty() {
        quote! { move |#pattern| ::std::string::ToString::to_string(&#key_fn) }
    } else {
        quote! { move |#pattern| { #(#leading_stmts)* ::std::string::ToString::to_string(&#key_fn) } }
    };

    // Issue #26 part 3: pre-clone identifiers referenced by `iter_expr` so the
    // `move ||` collection closure can construct cleanly when the enclosing
    // scope is itself a non-FnOnce closure (e.g. an `if`/`match` branch). The
    // outer closure stays untouched; each call constructs a fresh shadow that
    // the inner closure consumes. Copy types' `.clone()` is a no-op.
    let captures = collect_capture_idents(iter_expr);
    let capture_clones: Vec<TokenStream2> = captures
        .iter()
        .map(|id| quote! { #[allow(unused_mut)] let mut #id = ::std::clone::Clone::clone(&#id); })
        .collect();

    quote! {
        {
            #(#capture_clones)*
            rinch::core::for_each_dom_typed(
                __scope,
                &#parent_var,
                #collection,
                #key_closure,
                move |#pattern, __child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    #body
                }
            );
        }
    }
}

/// Extract a `key:` expression from the first child element of a for loop.
///
/// Returns the key expression token stream. If no `key:` prop is found,
/// falls back to Debug formatting the item.
///
/// Note: The `key:` prop is left on the element. HTML codegen treats `key`
/// as a special attribute and skips it (it would just become a harmless
/// `set_attribute("key", ...)` otherwise).
fn extract_key_expr(for_loop: &RsxForLoop) -> TokenStream2 {
    // Look for key: prop on the first child element (skip leading let statements)
    let first_element = for_loop
        .children
        .iter()
        .find(|c| !matches!(c, RsxNode::Statement(_)));

    if let Some(RsxNode::Element(el)) = first_element
        && let Some(key_prop) = el.props.iter().find(|p| p.name == "key")
    {
        let key_expr = &key_prop.value;
        return quote! { #key_expr };
    }

    // No key prop found — use debug format of the item as key (fallback)
    let pattern = &for_loop.pattern;
    quote! { format!("{:?}", #pattern) }
}

/// Generate DOM code for a native `match` block in RSX.
///
/// Desugars to `match_dom()`. The scrutinee is evaluated in a discriminant closure
/// that returns a branch index. Each arm becomes a boxed render closure.
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

    // Build branch closures
    let branch_closures: Vec<TokenStream2> = match_block
        .arms
        .iter()
        .map(|arm| {
            let pat = &arm.pattern;
            let guard_check = arm.guard.as_ref().map(|g| quote! { if #g });
            let body = generate_children_body(&arm.children, ctx);

            // Each branch re-evaluates the scrutinee to bind pattern variables.
            // We use `match` with the specific pattern + a catch-all unreachable.
            quote! {
                Box::new(move |__child_scope: &mut rinch::core::dom::RenderScope| -> rinch::core::dom::NodeHandle {
                    let __scope = __child_scope;
                    #[allow(unreachable_patterns, unused_variables, irrefutable_let_patterns)]
                    match #scrutinee {
                        #pat #guard_check => { #body }
                        _ => unreachable!()
                    }
                }) as Box<dyn Fn(&mut rinch::core::dom::RenderScope) -> rinch::core::dom::NodeHandle>
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
mod tests {
    use super::collect_capture_idents;

    fn caps(src: &str) -> Vec<String> {
        let expr: syn::Expr = syn::parse_str(src).expect("parse");
        collect_capture_idents(&expr)
            .iter()
            .map(|i| i.to_string())
            .collect()
    }

    #[test]
    fn closure_params_are_not_captures() {
        // The #32 regression repro.
        let captures = caps("(0..total_bars).filter(|b| b % 4 == 0).collect::<Vec<u32>>()");
        assert!(captures.contains(&"total_bars".to_string()));
        assert!(!captures.contains(&"b".to_string()));
    }

    #[test]
    fn tuple_destructure_closure_params() {
        let captures = caps("data.iter().filter(|(a, b)| a > b).collect::<Vec<_>>()");
        assert!(captures.contains(&"data".to_string()));
        assert!(!captures.contains(&"a".to_string()));
        assert!(!captures.contains(&"b".to_string()));
    }

    #[test]
    fn nested_closure_params() {
        // Outer closure's `x` shouldn't leak into the outer scan, nor should inner's `y`.
        let captures = caps(
            "outer.iter().map(|x| inner.iter().map(|y| x + y).sum::<i32>()).collect::<Vec<_>>()",
        );
        assert!(captures.contains(&"outer".to_string()));
        assert!(captures.contains(&"inner".to_string()));
        assert!(!captures.contains(&"x".to_string()));
        assert!(!captures.contains(&"y".to_string()));
    }

    #[test]
    fn let_binding_inside_block_is_not_a_capture() {
        let captures = caps(
            "{ let extra = base.len(); items.iter().filter(|i| i.size > extra).collect::<Vec<_>>() }",
        );
        assert!(captures.contains(&"base".to_string()));
        assert!(captures.contains(&"items".to_string()));
        assert!(!captures.contains(&"extra".to_string()));
        assert!(!captures.contains(&"i".to_string()));
    }

    #[test]
    fn rhs_of_let_sees_outer_scope() {
        // `x` in the RHS of `let x = x + 1` should refer to the outer binding,
        // which IS a capture.
        let captures = caps("{ let total = total + 1; vec![total] }");
        assert!(captures.contains(&"total".to_string()));
    }

    #[test]
    fn pre_regression_simple_capture_still_works() {
        // The original #26 case — a non-Copy fn param used in the iter source.
        let captures = caps("variant_options(default_variant.clone())");
        assert!(captures.contains(&"default_variant".to_string()));
    }

    #[test]
    fn if_let_binding_is_not_a_capture() {
        // The repro: `cid` is bound by `if let` inside a filter closure and used
        // in the then-branch. It must not be treated as an outer capture, while
        // the scrutinee `active_pane` still is.
        let captures = caps(
            "items.into_iter().filter(|f| if let Pane::Editor(ref cid) = active_pane.get() { f.id == *cid } else { false })",
        );
        assert!(captures.contains(&"items".to_string()));
        assert!(captures.contains(&"active_pane".to_string()));
        assert!(!captures.contains(&"cid".to_string()));
    }

    #[test]
    fn if_let_else_branch_does_not_see_binding() {
        // A name used only in the else branch IS an outer capture; the `if let`
        // pattern binding does not leak there.
        let captures =
            caps("{ if let Some(x) = maybe { vec![x] } else { vec![fallback] } }");
        assert!(captures.contains(&"maybe".to_string()));
        assert!(captures.contains(&"fallback".to_string()));
        assert!(!captures.contains(&"x".to_string()));
    }

    #[test]
    fn match_arm_binding_is_not_a_capture() {
        let captures = caps(
            "items.iter().filter(|i| match active.get() { Pane::Editor(ref c) => i.id == *c, _ => false })",
        );
        assert!(captures.contains(&"items".to_string()));
        assert!(captures.contains(&"active".to_string()));
        assert!(!captures.contains(&"c".to_string()));
    }

    #[test]
    fn while_let_binding_is_not_a_capture() {
        let captures = caps("{ while let Some(n) = cursor.next() { total += n; } vec![total] }");
        assert!(captures.contains(&"cursor".to_string()));
        assert!(captures.contains(&"total".to_string()));
        assert!(!captures.contains(&"n".to_string()));
    }

    #[test]
    fn let_chain_later_link_sees_earlier_binding() {
        // In `if let Some(x) = a && x > 0`, `x` in the second link resolves to
        // the binding from the first — not an outer capture. `a` is a capture.
        let captures = caps("{ if let Some(x) = a && x > threshold { vec![x] } else { vec![] } }");
        assert!(captures.contains(&"a".to_string()));
        assert!(captures.contains(&"threshold".to_string()));
        assert!(!captures.contains(&"x".to_string()));
    }
}
