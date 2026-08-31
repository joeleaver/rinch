//! RSX node types.

use proc_macro2::TokenTree;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Pat, Result, Token, token};

use crate::element::RsxElement;

/// A node in the RSX tree.
pub enum RsxNode {
    /// A component or HTML element with optional props and children.
    Element(RsxElement),
    /// A text literal.
    Text(LitStr),
    /// A Rust expression in braces.
    Expr(Expr),
    /// An `if` / `else if` / `else` block.
    IfBlock(RsxIfBlock),
    /// A `for ... in ...` loop.
    ForLoop(RsxForLoop),
    /// A `match` block.
    MatchBlock(RsxMatchBlock),
    /// A `let` statement (allowed inside control flow bodies).
    Statement(syn::Stmt),
}

impl Parse for RsxNode {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(LitStr) {
            Ok(RsxNode::Text(input.parse()?))
        } else if input.peek(token::Brace) {
            parse_braced_node(input)
        } else if input.peek(Token![|]) || input.peek(Token![move]) {
            // Parse bare closure as expression (for For component view functions)
            Ok(RsxNode::Expr(input.parse()?))
        } else if input.peek(Token![if]) {
            Ok(RsxNode::IfBlock(input.parse()?))
        } else if input.peek(Token![for]) {
            Ok(RsxNode::ForLoop(input.parse()?))
        } else if input.peek(Token![match]) {
            Ok(RsxNode::MatchBlock(input.parse()?))
        } else {
            Ok(RsxNode::Element(input.parse()?))
        }
    }
}

/// The kind of control flow found inside a braced node, if any.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlFlowKind {
    If,
    For,
    Match,
}

impl ControlFlowKind {
    fn keyword(self) -> &'static str {
        match self {
            ControlFlowKind::If => "if",
            ControlFlowKind::For => "for",
            ControlFlowKind::Match => "match",
        }
    }
}

/// Parse `{ … }` in node position.
///
/// **A brace around control flow is transparent** (issue #221): `{ match x { … } }`
/// renders the same reactive `match_dom` that `match x { … }` does. It has to be,
/// because the alternative was silence — the brace used to be peeked before the
/// `if`/`for`/`match` keywords, so the construct became a plain `syn::Expr`,
/// codegen'd through `IntoNode::into_node` and evaluated **once**. The branch
/// that rendered on mount was the branch you kept, with nothing at the call site
/// to mark the difference from its unbraced twin.
///
/// The content is tried as one rsx construct on a fork, and only a clean parse
/// that consumes the whole brace commits. Everything else is the expression it
/// has always been: `{ count.to_string() }`, `{|| count.get()}`,
/// `{ section(__scope) }` never reach the trial at all, since they do not start
/// with a control-flow keyword.
///
/// Control flow whose bodies are *not* rsx — `{ if c { helper() } else { other() } }`,
/// or arms written as nested `rsx! { … }` — cannot be made reactive this way. It
/// is rejected with [`braced_control_flow_error`] rather than left silently
/// static, so no shape of braced control flow renders once in silence.
fn parse_braced_node(input: ParseStream) -> Result<RsxNode> {
    // Look inside the braces without consuming them.
    let ahead = input.fork();
    let inner;
    syn::braced!(inner in ahead);

    let kind = if inner.peek(Token![if]) {
        Some(ControlFlowKind::If)
    } else if inner.peek(Token![for]) {
        Some(ControlFlowKind::For)
    } else if inner.peek(Token![match]) {
        Some(ControlFlowKind::Match)
    } else {
        None
    };

    let Some(kind) = kind else {
        let content;
        syn::braced!(content in input);
        return Ok(RsxNode::Expr(content.parse()?));
    };

    // Speculative parse on the fork.
    let trial_parsed = match kind {
        ControlFlowKind::If => inner.parse::<RsxIfBlock>().is_ok(),
        ControlFlowKind::For => inner.parse::<RsxForLoop>().is_ok(),
        ControlFlowKind::Match => inner.parse::<RsxMatchBlock>().is_ok(),
    };
    // Leftover tokens mean the braces hold more than the one construct, which is
    // not something we can render — treat it as a failure.
    let parses_as_rsx = trial_parsed && inner.is_empty();

    let content;
    syn::braced!(content in input);
    if parses_as_rsx {
        return match kind {
            ControlFlowKind::If => Ok(RsxNode::IfBlock(content.parse()?)),
            ControlFlowKind::For => Ok(RsxNode::ForLoop(content.parse()?)),
            ControlFlowKind::Match => Ok(RsxNode::MatchBlock(content.parse()?)),
        };
    }

    let expr: Expr = content.parse()?;
    if matches!(expr, Expr::If(_) | Expr::Match(_) | Expr::ForLoop(_)) {
        return Err(braced_control_flow_error(&expr, kind));
    }
    Ok(RsxNode::Expr(expr))
}

/// The diagnostic for braced control flow that cannot be parsed as rsx.
///
/// This is the case the transparent brace cannot rescue: the construct is
/// control flow, but its bodies are Rust expressions rather than rsx nodes, so
/// there is nothing to render reactively. Before issue #221 it compiled and went
/// stale on the first frame; now it says so, and names both ways out — the one
/// for rendering, and the one for a value.
fn braced_control_flow_error(expr: &Expr, kind: ControlFlowKind) -> syn::Error {
    let keyword = kind.keyword();
    syn::Error::new_spanned(
        expr,
        format!(
            "`{keyword}` wrapped in braces renders once and never updates.\n\
             help: for reactive markup, drop the braces — `{keyword} … {{ … }}` directly in \
             node position — and write each body as rsx (an element, a text literal, or \
             nested control flow) rather than a nested `rsx! {{ … }}` invocation\n\
             help: for a reactive *value* rather than reactive markup, wrap it in a closure \
             instead — `{{|| {keyword} … }}`"
        ),
    )
}

// ============================================================================
// If / Else If / Else
// ============================================================================

/// An `if` block in RSX with optional `else if` / `else` chains.
pub struct RsxIfBlock {
    /// The condition expression (for plain `if`) or the expression after `=` (for `if let`).
    pub condition: Expr,
    /// Whether this is an `if let` block.
    pub is_if_let: bool,
    /// The pattern for `if let` (None for plain `if`).
    pub pattern: Option<Pat>,
    /// Children to render when condition is true.
    pub then_children: Vec<RsxNode>,
    /// Optional else branch.
    pub else_branch: Option<RsxElseBranch>,
}

/// The else branch of an if block.
pub enum RsxElseBranch {
    /// `else if ...`
    ElseIf(Box<RsxIfBlock>),
    /// `else { ... }`
    Else(Vec<RsxNode>),
}

impl Parse for RsxIfBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        input.parse::<Token![if]>()?;

        // Check for `if let`
        let is_if_let = input.peek(Token![let]);
        let pattern = if is_if_let {
            input.parse::<Token![let]>()?;
            let pat = Pat::parse_multi_with_leading_vert(input)?;
            input.parse::<Token![=]>()?;
            Some(pat)
        } else {
            None
        };

        // Parse condition expression (stop before the opening brace)
        let condition = parse_expr_before_brace(input)?;

        // Parse then body
        let content;
        syn::braced!(content in input);
        let then_children = parse_rsx_children(&content)?;

        // Check for else
        let else_branch = if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;
            if input.peek(Token![if]) {
                // `else if` — recurse
                Some(RsxElseBranch::ElseIf(Box::new(input.parse()?)))
            } else {
                // `else { ... }`
                let content;
                syn::braced!(content in input);
                let children = parse_rsx_children(&content)?;
                Some(RsxElseBranch::Else(children))
            }
        } else {
            None
        };

        Ok(RsxIfBlock {
            condition,
            is_if_let,
            pattern,
            then_children,
            else_branch,
        })
    }
}

// ============================================================================
// For Loop
// ============================================================================

/// A `for` loop in RSX.
pub struct RsxForLoop {
    /// The loop variable pattern (e.g., `todo` or `(i, item)`).
    pub pattern: Pat,
    /// The iterator expression (e.g., `todos.get()`).
    pub iter_expr: Expr,
    /// RSX children (the loop body).
    pub children: Vec<RsxNode>,
}

impl Parse for RsxForLoop {
    fn parse(input: ParseStream) -> Result<Self> {
        input.parse::<Token![for]>()?;

        // Parse the loop variable pattern
        let pattern = Pat::parse_single(input)?;

        input.parse::<Token![in]>()?;

        // Parse iterator expression (stop before the opening brace)
        let iter_expr = parse_expr_before_brace(input)?;

        // Parse body
        let content;
        syn::braced!(content in input);
        let children = parse_rsx_children(&content)?;

        Ok(RsxForLoop {
            pattern,
            iter_expr,
            children,
        })
    }
}

// ============================================================================
// Match Block
// ============================================================================

/// A `match` block in RSX.
pub struct RsxMatchBlock {
    /// The expression being matched.
    pub scrutinee: Expr,
    /// The match arms.
    pub arms: Vec<RsxMatchArm>,
}

/// A single arm in an RSX `match` block.
pub struct RsxMatchArm {
    /// The match pattern (e.g., `0`, `Some(x)`, `_`).
    pub pattern: Pat,
    /// Optional `if` guard expression.
    pub guard: Option<Expr>,
    /// RSX children for this arm. Usually a single node; a braced `{ let …; node }`
    /// body yields leading statement(s) + node(s), like an `if`/`for` body.
    pub children: Vec<RsxNode>,
}

impl Parse for RsxMatchBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        input.parse::<Token![match]>()?;

        // Parse scrutinee expression (stop before the opening brace)
        let scrutinee = parse_expr_before_brace(input)?;

        // Parse arms
        let content;
        syn::braced!(content in input);
        let mut arms = Vec::new();
        while !content.is_empty() {
            arms.push(content.parse()?);
        }

        Ok(RsxMatchBlock { scrutinee, arms })
    }
}

impl Parse for RsxMatchArm {
    fn parse(input: ParseStream) -> Result<Self> {
        // Parse pattern (with | alternatives)
        let pattern = Pat::parse_multi_with_leading_vert(input)?;

        // Optional guard: `if expr`
        let guard = if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;
            Some(parse_expr_before_fat_arrow(input)?)
        } else {
            None
        };

        // =>
        input.parse::<Token![=>]>()?;

        // Parse the arm body. Normally one node (`0 => div { … }`), but a braced
        // block whose content starts with `let` is parsed as rsx children
        // (leading statements + node) so an arm can do per-branch setup — the
        // same shape `if`/`for` bodies already accept. `generate_children_body`
        // collapses the statements + node(s) into one NodeHandle. A non-`let`
        // brace (`{ expr }`, `{|| … }`) keeps the single-node behavior.
        let children = if input.peek(token::Brace) {
            let ahead = input.fork();
            let inner;
            syn::braced!(inner in ahead);
            if inner.peek(Token![let]) {
                let content;
                syn::braced!(content in input);
                parse_rsx_children(&content)?
            } else {
                vec![input.parse::<RsxNode>()?]
            }
        } else {
            vec![input.parse::<RsxNode>()?]
        };

        // Optional trailing comma
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }

        Ok(RsxMatchArm {
            pattern,
            guard,
            children,
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Parse RSX children from a braced content stream.
pub(crate) fn parse_rsx_children(input: ParseStream) -> Result<Vec<RsxNode>> {
    let mut children = Vec::new();
    while !input.is_empty() {
        // Check for `let` statements (allowed inside control flow bodies)
        if input.peek(Token![let]) {
            children.push(RsxNode::Statement(input.parse()?));
        } else {
            children.push(input.parse()?);
        }
        // Consume optional trailing comma
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(children)
}

/// Parse an expression, stopping before the next top-level `{`.
///
/// This is needed for `if`, `for`, and `match` conditions/scrutinees where
/// `Expr::parse` would greedily consume the braces as a struct literal.
fn parse_expr_before_brace(input: ParseStream) -> Result<Expr> {
    let mut tokens = proc_macro2::TokenStream::new();
    while !input.is_empty() && !input.peek(token::Brace) {
        tokens.extend(input.parse::<TokenTree>());
    }
    if tokens.is_empty() {
        return Err(input.error("expected expression before `{`"));
    }
    syn::parse2(tokens)
}

/// Parse an expression, stopping before `=>`.
///
/// Used for match arm guard expressions.
fn parse_expr_before_fat_arrow(input: ParseStream) -> Result<Expr> {
    let mut tokens = proc_macro2::TokenStream::new();
    while !input.is_empty() && !input.peek(Token![=>]) {
        tokens.extend(input.parse::<TokenTree>());
    }
    if tokens.is_empty() {
        return Err(input.error("expected expression before `=>`"));
    }
    syn::parse2(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_str;

    /// Helper to check what variant was parsed.
    fn parse_variant(input: &str) -> &'static str {
        match parse_str::<RsxNode>(input) {
            Ok(RsxNode::Text(_)) => "Text",
            Ok(RsxNode::Expr(_)) => "Expr",
            Ok(RsxNode::Element(_)) => "Element",
            Ok(RsxNode::IfBlock(_)) => "IfBlock",
            Ok(RsxNode::ForLoop(_)) => "ForLoop",
            Ok(RsxNode::MatchBlock(_)) => "MatchBlock",
            Ok(RsxNode::Statement(_)) => "Statement",
            Err(_) => "Error",
        }
    }

    // ── Text literal parsing ─────────────────────────────────────

    #[test]
    fn parse_text_literal() {
        assert_eq!(parse_variant(r#""hello world""#), "Text");
    }

    #[test]
    fn parse_text_empty_string() {
        assert_eq!(parse_variant(r#""""#), "Text");
    }

    #[test]
    fn parse_text_with_special_chars() {
        assert_eq!(parse_variant(r#""hello <b>world</b>""#), "Text");
    }

    // ── Braced expression parsing ────────────────────────────────

    #[test]
    fn parse_braced_expression_integer() {
        assert_eq!(parse_variant("{ 42 }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_variable() {
        assert_eq!(parse_variant("{ x }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_method_call() {
        assert_eq!(parse_variant("{ count.get() }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_to_string() {
        assert_eq!(parse_variant("{ count.get().to_string() }"), "Expr");
    }

    #[test]
    fn parse_braced_expression_format() {
        assert_eq!(parse_variant(r#"{ format!("val: {}", x) }"#), "Expr");
    }

    // ── Braced closure parsing ───────────────────────────────────

    #[test]
    fn parse_braced_closure() {
        assert_eq!(parse_variant("{|| count.get()}"), "Expr");
    }

    #[test]
    fn parse_braced_closure_with_to_string() {
        assert_eq!(parse_variant("{|| count.get().to_string()}"), "Expr");
    }

    #[test]
    fn parse_braced_closure_with_if() {
        assert_eq!(
            parse_variant(r#"{|| if count.get() > 5 { "high" } else { "low" }}"#),
            "Expr"
        );
    }

    #[test]
    fn parse_braced_move_closure() {
        assert_eq!(parse_variant("{move || count.get()}"), "Expr");
    }

    // ── Bare closure parsing (For view functions) ────────────────

    #[test]
    fn parse_bare_closure_simple() {
        assert_eq!(parse_variant("|item| { item }"), "Expr");
    }

    #[test]
    fn parse_bare_closure_with_body() {
        assert_eq!(parse_variant("|item| { item.name.clone() }"), "Expr");
    }

    #[test]
    fn parse_bare_closure_with_type_annotation() {
        assert_eq!(
            parse_variant("|item: &ForItem| { item.name.clone() }"),
            "Expr"
        );
    }

    #[test]
    fn parse_move_closure() {
        assert_eq!(parse_variant("move |x| x + 1"), "Expr");
    }

    #[test]
    fn parse_move_closure_with_block() {
        assert_eq!(parse_variant("move |x| { x + 1 }"), "Expr");
    }

    // ── Element parsing ──────────────────────────────────────────

    #[test]
    fn parse_element_simple() {
        assert_eq!(parse_variant("div {}"), "Element");
    }

    #[test]
    fn parse_element_with_text() {
        assert_eq!(parse_variant(r#"div { "hello" }"#), "Element");
    }

    #[test]
    fn parse_element_with_props() {
        assert_eq!(parse_variant(r#"div { class: "foo" }"#), "Element");
    }

    #[test]
    fn parse_element_nested() {
        assert_eq!(parse_variant(r#"div { p { "hello" } }"#), "Element");
    }

    // ── Component parsing (PascalCase → Element) ──────────

    #[test]
    fn parse_component() {
        assert_eq!(parse_variant(r#"Button { variant: "filled" }"#), "Element");
    }

    #[test]
    fn parse_show_component() {
        assert_eq!(parse_variant("Show { when: {|| true} }"), "Element");
    }

    #[test]
    fn parse_for_component() {
        assert_eq!(
            parse_variant("For { each: {|| vec![]}, |item| { item } }"),
            "Element"
        );
    }

    // ── If block parsing ─────────────────────────────────────────

    #[test]
    fn parse_if_simple() {
        assert_eq!(parse_variant(r#"if visible { p { "yes" } }"#), "IfBlock");
    }

    #[test]
    fn parse_if_method_call_condition() {
        assert_eq!(
            parse_variant(r#"if visible.get() { p { "yes" } }"#),
            "IfBlock"
        );
    }

    #[test]
    fn parse_if_else() {
        assert_eq!(
            parse_variant(r#"if visible.get() { p { "yes" } } else { p { "no" } }"#),
            "IfBlock"
        );
    }

    #[test]
    fn parse_if_else_if_else() {
        let input = r#"if a.get() { p { "a" } } else if b.get() { p { "b" } } else { p { "c" } }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::IfBlock(if_block) => {
                assert!(!if_block.is_if_let);
                assert!(if_block.else_branch.is_some());
                match if_block.else_branch.as_ref().unwrap() {
                    RsxElseBranch::ElseIf(inner) => {
                        assert!(inner.else_branch.is_some());
                        assert!(matches!(
                            inner.else_branch.as_ref().unwrap(),
                            RsxElseBranch::Else(_)
                        ));
                    }
                    _ => panic!("Expected ElseIf"),
                }
            }
            _ => panic!("Expected IfBlock"),
        }
    }

    #[test]
    fn parse_if_let() {
        let input = r#"if let Some(x) = value.get() { p { "found" } }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::IfBlock(if_block) => {
                assert!(if_block.is_if_let);
                assert!(if_block.pattern.is_some());
                assert!(if_block.else_branch.is_none());
            }
            _ => panic!("Expected IfBlock"),
        }
    }

    #[test]
    fn parse_if_comparison() {
        assert_eq!(
            parse_variant(r#"if count.get() > 5 { p { "big" } }"#),
            "IfBlock"
        );
    }

    // ── For loop parsing ─────────────────────────────────────────

    #[test]
    fn parse_for_simple() {
        assert_eq!(
            parse_variant(r#"for item in items.get() { div { "item" } }"#),
            "ForLoop"
        );
    }

    #[test]
    fn parse_for_tuple_pattern() {
        assert_eq!(
            parse_variant(
                r#"for (i, item) in items.get().into_iter().enumerate() { div { "item" } }"#
            ),
            "ForLoop"
        );
    }

    #[test]
    fn parse_for_with_children() {
        let input = r#"for todo in todos.get() { div { key: todo.id, "text" } }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::ForLoop(for_loop) => {
                assert_eq!(for_loop.children.len(), 1);
            }
            _ => panic!("Expected ForLoop"),
        }
    }

    // ── Match block parsing ──────────────────────────────────────

    #[test]
    fn parse_match_simple() {
        let input =
            r#"match tab.get() { 0 => div { "Home" }, 1 => div { "About" }, _ => div { "404" } }"#;
        assert_eq!(parse_variant(input), "MatchBlock");
    }

    #[test]
    fn parse_match_arms() {
        let input =
            r#"match tab.get() { 0 => div { "Home" }, 1 => div { "About" }, _ => div { "404" } }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::MatchBlock(match_block) => {
                assert_eq!(match_block.arms.len(), 3);
            }
            _ => panic!("Expected MatchBlock"),
        }
    }

    #[test]
    fn parse_match_with_guard() {
        let input = r#"match x.get() { n if n > 5 => div { "big" }, _ => div { "small" } }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::MatchBlock(match_block) => {
                assert_eq!(match_block.arms.len(), 2);
                assert!(match_block.arms[0].guard.is_some());
                assert!(match_block.arms[1].guard.is_none());
            }
            _ => panic!("Expected MatchBlock"),
        }
    }

    #[test]
    fn parse_match_with_text_arm() {
        let input = r#"match x.get() { 0 => "hello", _ => "world" }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::MatchBlock(match_block) => {
                assert_eq!(match_block.arms.len(), 2);
                assert!(matches!(&match_block.arms[0].children[0], RsxNode::Text(_)));
            }
            _ => panic!("Expected MatchBlock"),
        }
    }

    #[test]
    fn parse_match_arm_with_let_block() {
        // A braced arm body starting with `let` parses as statements + node,
        // like an if/for body (issue #102 Gap 2).
        let input = r#"match x.get() { 0 => { let k = 1; div { "a" } }, _ => "b" }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::MatchBlock(mb) => {
                assert_eq!(mb.arms.len(), 2);
                assert_eq!(mb.arms[0].children.len(), 2);
                assert!(matches!(&mb.arms[0].children[0], RsxNode::Statement(_)));
                assert!(matches!(&mb.arms[0].children[1], RsxNode::Element(_)));
                // The non-block arm is unchanged (single node).
                assert_eq!(mb.arms[1].children.len(), 1);
                assert!(matches!(&mb.arms[1].children[0], RsxNode::Text(_)));
            }
            _ => panic!("Expected MatchBlock"),
        }
    }

    #[test]
    fn parse_match_arm_braced_expr_stays_single_node() {
        // A braced arm body that is NOT a `let` block keeps single-node behavior
        // (reactive embed) — no regression from the Gap 2 change.
        let input = r#"match x.get() { 0 => {|| count.get()}, _ => div {} }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::MatchBlock(mb) => {
                assert_eq!(mb.arms[0].children.len(), 1);
                assert!(matches!(&mb.arms[0].children[0], RsxNode::Expr(_)));
            }
            _ => panic!("Expected MatchBlock"),
        }
    }

    // ── Braced control flow (issue #221) ─────────────────────────

    /// A brace around control flow is transparent: the same reactive node the
    /// unbraced form produces. It used to parse as a plain `Expr`, which
    /// codegens through `IntoNode::into_node` and renders exactly once.
    #[test]
    fn braced_control_flow_parses_as_control_flow() {
        assert_eq!(
            parse_variant(r#"{ match x.get() { 0 => div { "a" }, _ => div { "b" } } }"#),
            "MatchBlock"
        );
        assert_eq!(
            parse_variant(r#"{ if flag.get() { div { "a" } } else { div { "b" } } }"#),
            "IfBlock"
        );
        // No `else` — still control flow.
        assert_eq!(parse_variant(r#"{ if flag.get() { "a" } }"#), "IfBlock");
        assert_eq!(
            parse_variant(r#"{ for item in items.get() { div { "row" } } }"#),
            "ForLoop"
        );
        // `if let`, which parses through the same RsxIfBlock path.
        assert_eq!(
            parse_variant(r#"{ if let Some(n) = slot.get() { div { {n} } } }"#),
            "IfBlock"
        );
    }

    /// Braced control flow nested in a `match` arm — the shape issue #221
    /// reports, and the reason it was found in the arm position: `_ => { … }`
    /// is where a brace is most idiomatic.
    #[test]
    fn a_braced_arm_body_of_control_flow_parses_as_control_flow() {
        let input = r#"match outer.get() { 0 => div { "none" }, _ => { match inner.get() { 0 => "zero", _ => "many" } } }"#;
        let node = parse_str::<RsxNode>(input).unwrap();
        match node {
            RsxNode::MatchBlock(mb) => {
                assert_eq!(mb.arms[1].children.len(), 1);
                assert!(matches!(&mb.arms[1].children[0], RsxNode::MatchBlock(_)));
            }
            _ => panic!("Expected MatchBlock"),
        }
    }

    /// Everything else in braces is the expression it has always been — these
    /// never start with a control-flow keyword, so they never reach the trial
    /// parse. The middle one is this repo's own idiom
    /// (`examples/ui-zoo/src/lib.rs`).
    #[test]
    fn a_braced_expression_is_still_an_expression() {
        assert_eq!(parse_variant("{ count.to_string() }"), "Expr");
        assert_eq!(parse_variant("{ overview_section(__scope) }"), "Expr");
        assert_eq!(parse_variant("{|| count.get()}"), "Expr");
        assert_eq!(
            parse_variant("{ items.iter().map(render).collect() }"),
            "Expr"
        );
    }

    /// Control flow whose bodies are Rust expressions rather than rsx cannot be
    /// made reactive by dropping the brace, so it is rejected instead of
    /// rendering once in silence. The message has to carry both ways out,
    /// because the right one depends on what the author meant.
    #[test]
    fn braced_control_flow_that_is_not_rsx_is_rejected() {
        let msg = match parse_str::<RsxNode>("{ if c.get() { helper() } else { other() } }") {
            Err(err) => err.to_string(),
            Ok(_) => panic!("braced control flow with non-rsx bodies must not compile"),
        };
        assert!(msg.contains("renders once and never updates"), "{msg}");
        assert!(msg.contains("drop the braces"), "{msg}");
        assert!(msg.contains("closure"), "{msg}");

        // The exact shape issue #221 reports: arms written as nested `rsx!`.
        let msg = match parse_str::<RsxNode>(
            r#"{ match sel.get() { Some(p) => rsx! { div { "a" } }, None => rsx! { div { "b" } } } }"#,
        ) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("nested rsx! arms cannot be parsed as rsx nodes"),
        };
        assert!(msg.contains("renders once and never updates"), "{msg}");
    }

    // ── Error cases ──────────────────────────────────────────────

    #[test]
    fn parse_fails_on_empty_input() {
        assert!(parse_str::<RsxNode>("").is_err());
    }

    #[test]
    fn parse_fails_on_invalid_token() {
        // A lone number without braces is not valid RSX
        assert_eq!(parse_variant("42"), "Error");
    }
}
