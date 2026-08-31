//! What a closure built by `rsx!` captures, and which of those captures need a
//! shadow clone.
//!
//! # Why any of this exists
//!
//! Every closure `rsx!` emits is a `move` closure that must be `'static` — a
//! `show_dom` branch, a `match_dom` arm, a `for_each_dom` view, an effect body,
//! a registered event handler. A `move` closure captures **by value**, even
//! where its body only reads through a reference, and the closures are
//! *constructed* whether or not they ever run: `show_dom` takes `then_fn` **and**
//! `else_fn`, so both branches of an `if` consume what they name. A non-`Copy`
//! value mentioned by two of them is therefore a genuine E0382/E0507 — correct
//! Rust, but a rule the author of an `if`/`else` has no reason to expect, since
//! only one branch ever renders.
//!
//! The remedy is the one this module has always applied to a `for` loop's
//! iterator expression (issue #26 part 3+4): give each construction site its own
//! `let x = x.clone();` shadow, so the site's closure captures the shadow and the
//! original stays where it was. [`shadow_clones`] emits those bindings and
//! [`wrap_site`] wraps a closure expression in the block that holds them. Issue
//! #223 extends the same mechanism to `if`/`else`, `match`, the `for` key/view
//! closures, reactive attributes and reactive text.
//!
//! # What must never happen
//!
//! Cloning a value that did not need it can turn working code into an E0599, so
//! a shadow is only ever emitted where the value is **provably `Copy` today**:
//!
//! - **Contested between sibling sites** ([`contested_names`]) — two `move`
//!   closures of one construct both name it, so it is moved twice today unless
//!   it is `Copy`.
//! - **Captured from outside a repeatable body**
//!   ([`DomCodegenContext::site_shadows`]) — the site sits inside a branch /
//!   arm / view / effect closure that runs more than once, so constructing a
//!   `'static` `move` closure from it moves out of an `Fn`/`FnMut` capture
//!   today unless it is `Copy`.
//!
//! Either way the code being "fixed" does not compile without the shadow, and
//! the code that does compile without it is left alone: a value named by a
//! single branch is still moved, not cloned, and a value bound *inside* the
//! repeatable body is a fresh local on every run and is never touched.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::visit::Visit;

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
///   let-chains) patterns, `for` loop patterns and `match` arm patterns, each
///   scoped to the branch that can actually see them — the `if let` analogue of
///   the #32 fix.
/// - Excludes anything inside a nested item (`fn`, `struct`, `impl`): an item
///   cannot capture, and its parameters are not names of the enclosing scope.
///
/// The collected list is used to emit `let #id = #id.clone();` shadow bindings
/// before the inner `move ||` closure is constructed. Cloning Copy types via
/// `.clone()` is a no-op; the shadow only matters for non-Copy values like
/// `String`. Types that don't impl `Clone` will still fail with a clear error
/// pointing at the field — same behaviour as user code calling `.clone()` directly.
///
/// Identifiers hidden inside a macro invocation are **not** collected: `syn`
/// models a macro body as an opaque token stream. That is the safe direction to
/// err — a missed capture leaves today's error in place rather than inventing a
/// binding that doesn't exist.
pub(crate) fn collect_capture_idents(expr: &syn::Expr) -> Vec<syn::Ident> {
    let mut collector = Collector {
        idents: Vec::new(),
        seen: HashSet::new(),
        locals: Vec::new(),
    };
    collector.visit_expr(expr);
    collector.idents
}

/// The captures of an already-generated closure body.
///
/// Codegen builds a body out of the user's RSX before it knows what to shadow,
/// so the analysis runs over the emitted tokens: they are the same user
/// expressions, plus scaffolding the heuristics above already ignore (every
/// generated binding is `__`-prefixed, every runtime path is multi-segment).
///
/// Tokens that don't parse as a block expression yield no captures, which is
/// exactly today's behaviour — no shadows, no new errors.
pub(crate) fn collect_body_captures(body: &TokenStream2) -> Vec<syn::Ident> {
    match syn::parse2::<syn::Expr>(quote! { { #body } }) {
        Ok(expr) => collect_capture_idents(&expr),
        Err(_) => Vec::new(),
    }
}

/// The names referenced by two or more of `sites`.
///
/// Each site is one `move` closure of a single construct, and every one of them
/// is constructed — an `if`'s condition/then/else, a `match`'s discriminant and
/// every arm, a `for`'s collection/key/view. A name in two of those lists is
/// moved twice today, so it is `Copy` or the code does not compile.
pub(crate) fn contested_names(sites: &[&[syn::Ident]]) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut shared: HashSet<String> = HashSet::new();
    for site in sites {
        // One site naming a value twice is not a conflict with itself.
        let mut site_names: HashSet<String> = HashSet::new();
        for id in site.iter() {
            site_names.insert(id.to_string());
        }
        for name in site_names {
            if !seen.insert(name.clone()) {
                shared.insert(name);
            }
        }
    }
    shared
}

/// `let mut x = Clone::clone(&x);` for each ident, in source order.
///
/// `mut` (with the lint silenced) because the shadow stands in for the original
/// everywhere the closure uses it, including a mutating use.
pub(crate) fn shadow_clones<'a>(idents: impl IntoIterator<Item = &'a syn::Ident>) -> TokenStream2 {
    let binds: Vec<TokenStream2> = idents
        .into_iter()
        .map(|id| quote! { #[allow(unused_mut)] let mut #id = ::std::clone::Clone::clone(&#id); })
        .collect();
    quote! { #(#binds)* }
}

/// Put `closure` in a block that shadow-clones first, so the closure captures
/// the shadows and the enclosing scope keeps its own values.
///
/// A construction site with nothing to shadow is emitted unchanged.
pub(crate) fn wrap_site(shadows: &TokenStream2, closure: TokenStream2) -> TokenStream2 {
    if shadows.is_empty() {
        closure
    } else {
        quote! { { #shadows #closure } }
    }
}

/// Whether `expr` is a `move` closure written by the user.
///
/// Only a `move` closure captures by value, so only a `move` closure moves a
/// value out of the `FnMut` effect that reconstructs it on every fire. A
/// borrowing closure is left alone — cloning for it could break code that
/// compiles today.
pub(crate) fn is_move_closure(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Closure(c) if c.capture.is_some())
}

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
    fn scan_cond(&mut self, cond: &syn::Expr) {
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

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        // `for x in xs { .. }` binds `x` in the body only. The iterator
        // expression is visited in the *outer* scope, as an initialiser is.
        self.visit_expr(&node.expr);
        let mut frame = HashSet::new();
        collect_pat_idents(&node.pat, &mut frame);
        self.locals.push(frame);
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

    fn visit_item(&mut self, _item: &'ast syn::Item) {
        // An item declared inside a body (`fn helper(a: u32) { .. }`) captures
        // nothing, and its own parameters are not names of the enclosing scope.
    }
}

/// Add every identifier *bound by* `pat` to `out`. Walks tuple/struct/tuple-struct
/// and or-patterns; ignores paths/literals/wildcards/ranges (those don't bind).
///
/// Used to determine which names a closure's params or a `let` introduces, so the
/// capture scanner can skip them.
pub(crate) fn collect_pat_idents(pat: &syn::Pat, out: &mut HashSet<String>) {
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

#[cfg(test)]
mod tests {
    use super::{collect_body_captures, collect_capture_idents, contested_names};
    use quote::quote;

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
        //
        // NB: use array literals (`[..]`), not `vec![..]`. `syn`'s visitor treats
        // macro token streams as opaque, so an ident only ever referenced inside a
        // `vec![..]` is invisible to the scanner and would never be collected —
        // which would make the `fallback` assertion below spuriously fail.
        let captures = caps("{ if let Some(x) = maybe { [x] } else { [fallback] } }");
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
        let captures = caps("{ while let Some(n) = cursor.next() { total += n; } [total] }");
        assert!(captures.contains(&"cursor".to_string()));
        assert!(captures.contains(&"total".to_string()));
        assert!(!captures.contains(&"n".to_string()));
    }

    #[test]
    fn let_chain_later_link_sees_earlier_binding() {
        // In `if let Some(x) = a && x > 0`, `x` in the second link resolves to
        // the binding from the first — not an outer capture. `a` is a capture.
        let captures = caps("{ if let Some(x) = a && x > threshold { [x] } else { [] } }");
        assert!(captures.contains(&"a".to_string()));
        assert!(captures.contains(&"threshold".to_string()));
        assert!(!captures.contains(&"x".to_string()));
    }

    #[test]
    fn match_guard_sees_arm_binding() {
        // A `match` arm guard can reference the names bound by its pattern, so
        // those names are not outer captures — but a name used *only* in the guard
        // (`limit`) still is.
        let captures = caps(
            "items.iter().filter(|i| match active.get() { Editor(c) if c > limit => true, _ => false })",
        );
        assert!(captures.contains(&"items".to_string()));
        assert!(captures.contains(&"active".to_string()));
        assert!(captures.contains(&"limit".to_string()));
        assert!(!captures.contains(&"c".to_string()));
    }

    #[test]
    fn nested_else_if_let_scopes_each_branch() {
        // `else if let Some(y) = b` recurses through the same `visit_expr_if`
        // path, so `y` is scoped to its own branch (not a capture) while both
        // scrutinees `a` and `b` are captured.
        let captures =
            caps("{ if let Some(x) = a { [x] } else if let Some(y) = b { [y] } else { [] } }");
        assert!(captures.contains(&"a".to_string()));
        assert!(captures.contains(&"b".to_string()));
        assert!(!captures.contains(&"x".to_string()));
        assert!(!captures.contains(&"y".to_string()));
    }

    /// A `for` statement binds its own loop variable. Shadow-cloning it would
    /// emit `let n = n.clone();` for a name the enclosing scope doesn't have —
    /// an E0425 in user code, not a fix.
    #[test]
    fn for_loop_pattern_is_not_a_capture() {
        let captures = caps("{ let mut sum = 0; for n in items.iter() { sum += n; } sum }");
        assert!(captures.contains(&"items".to_string()));
        assert!(!captures.contains(&"n".to_string()));
        assert!(!captures.contains(&"sum".to_string()));
    }

    /// An item declared inside a body captures nothing, and its parameters are
    /// not names of the enclosing scope.
    #[test]
    fn a_nested_item_contributes_no_captures() {
        let captures = caps("{ fn double(operand: u32) -> u32 { operand * 2 } double(width) }");
        assert!(captures.contains(&"width".to_string()));
        assert!(!captures.contains(&"operand".to_string()));
    }

    /// Generated bodies are scanned as tokens, and every name codegen itself
    /// introduces is `__`-prefixed or multi-segment, so only user values survive.
    #[test]
    fn generated_scaffolding_is_not_captured() {
        let body = quote! {
            let __el0 = __scope.create_element("p");
            let __child1 = rinch::core::IntoNode::into_node(row.label.clone(), __scope);
            __el0.append_child(&__child1);
            __el0
        };
        let captures: Vec<String> = collect_body_captures(&body)
            .iter()
            .map(|i| i.to_string())
            .collect();
        assert_eq!(captures, vec!["row".to_string()]);
    }

    /// Tokens that aren't a block expression yield nothing rather than panicking.
    #[test]
    fn unparseable_tokens_yield_no_captures() {
        assert!(collect_body_captures(&quote! { let x = ; }).is_empty());
    }

    #[test]
    fn only_a_name_two_sites_share_is_contested() {
        let ids = |names: &[&str]| -> Vec<syn::Ident> {
            names
                .iter()
                .map(|n| syn::Ident::new(n, proc_macro2::Span::call_site()))
                .collect()
        };
        let cond = ids(&["row"]);
        let then = ids(&["row", "only_then"]);
        let els = ids(&["only_else"]);
        let shared = contested_names(&[&cond, &then, &els]);
        assert!(shared.contains("row"));
        assert!(!shared.contains("only_then"));
        assert!(!shared.contains("only_else"));
    }
}
