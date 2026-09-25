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
    /// RSX children for this arm. Usually a single node; a braced body that opens
    /// with `let`, text, control flow or an element yields statement(s) + node(s),
    /// like an `if`/`for` body (see `parse_braced_arm_body`, issue #395).
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

        let children = if input.peek(token::Brace) {
            parse_braced_arm_body(input)?
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

/// Parse `=> { … }`, a braced arm body (issue #395).
///
/// An arm holds rsx children — several nodes, as an `if`/`for` body holds,
/// collapsed into one `NodeHandle` by `generate_children_body` — when its body
/// opens with an rsx node that is **not the whole story**:
///
/// - `let` always opens children.
/// - A string literal, `Name {` (element or component), `if`/`for`/`match` or a
///   braced interpolation `{ … }` is the *head*. When the head is followed by
///   another token that starts an rsx node ([`starts_rsx_node`]), the body is
///   children, and a typo anywhere in it is reported where it is.
/// - A literal or `Name {` head that is alone, or that fails to parse as rsx,
///   is children too (one node, or the rsx error at the typo).
/// - Everything else is the single braced node it always was: a lone control
///   flow construct (#221's transparent brace, diagnostic included), a lone
///   `{ … }`, a head followed by `.` or an operator (`"a".to_string()`,
///   `Foo { a: 1 }.into_node(__scope)`, `if … {} else {} .len()`), and every
///   body that opens with anything else — `{ section(__scope) }` (ui-zoo's
///   routing), `{a.clone()}`, `{|| …}`.
///
/// Parsing every braced arm as children would have broken those last ones:
/// `section(__scope)` is not an rsx node.
///
/// `{ Point { x: 1 } }` is a component, as `Point { x: 1 }` unbraced always
/// was. A path (`geom::Point { … }`) is not an element name and stays an
/// expression.
fn parse_braced_arm_body(input: ParseStream) -> Result<Vec<RsxNode>> {
    let ahead = input.fork();
    let inner;
    syn::braced!(inner in ahead);

    let children = |input: ParseStream| -> Result<Vec<RsxNode>> {
        let content;
        syn::braced!(content in input);
        parse_rsx_children(&content)
    };

    if inner.peek(Token![let]) {
        return children(input);
    }
    let rsx_head = inner.peek(LitStr) || (inner.peek(syn::Ident) && inner.peek2(token::Brace));
    let other_head = inner.peek(Token![if])
        || inner.peek(Token![for])
        || inner.peek(Token![match])
        || inner.peek(token::Brace);
    if !rsx_head && !other_head {
        return Ok(vec![input.parse::<RsxNode>()?]);
    }

    // Parse the head alone, on the fork.
    let head_ok = inner.parse::<RsxNode>().is_ok();
    let commit = if !head_ok {
        // A literal / element head is rsx: report its error. Control flow or a
        // braced head falls back, so #221 can diagnose its own case.
        rsx_head
    } else if inner.is_empty() {
        rsx_head
    } else {
        starts_rsx_node(&inner)
    };

    if commit {
        children(input)
    } else {
        Ok(vec![input.parse::<RsxNode>()?])
    }
}

/// Whether the next token in a braced arm body starts another rsx node (or is
/// the `,` that separates two), i.e. the head before it was not the end of a
/// Rust expression such as `… .len()` or `… == d`.
fn starts_rsx_node(input: ParseStream) -> bool {
    input.peek(LitStr)
        || input.peek(token::Brace)
        || input.peek(Token![if])
        || input.peek(Token![for])
        || input.peek(Token![match])
        || input.peek(Token![let])
        || input.peek(Token![,])
        || (input.peek(syn::Ident) && input.peek2(token::Brace))
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

    // ── Braced arm bodies: decided by the first token (issue #395) ──

    /// The arm children of `match x.get() { 0 => <arm>, _ => "b" }`'s first arm,
    /// as variant names.
    fn first_arm(arm: &str) -> Vec<&'static str> {
        let input = format!("match x.get() {{ 0 => {arm}, _ => \"b\" }}");
        let node = match parse_str::<RsxNode>(&input) {
            Ok(node) => node,
            Err(err) => panic!("{arm} did not parse: {err}"),
        };
        let RsxNode::MatchBlock(mb) = node else {
            panic!("Expected MatchBlock");
        };
        mb.arms[0]
            .children
            .iter()
            .map(|c| match c {
                RsxNode::Text(_) => "Text",
                RsxNode::Expr(_) => "Expr",
                RsxNode::Element(_) => "Element",
                RsxNode::IfBlock(_) => "IfBlock",
                RsxNode::ForLoop(_) => "ForLoop",
                RsxNode::MatchBlock(_) => "MatchBlock",
                RsxNode::Statement(_) => "Statement",
            })
            .collect()
    }

    /// A braced arm holds several nodes when it starts with an element, as an
    /// `if`/`for` body does. It used to take exactly one node unless the arm
    /// happened to open with `let`: `unexpected token, expected }`.
    #[test]
    fn a_braced_arm_holds_several_elements() {
        assert_eq!(first_arm("{ div {} span {} }"), ["Element", "Element"]);
        assert_eq!(first_arm(r#"{ div { "a" } "t" }"#), ["Element", "Text"]);
        assert_eq!(first_arm("{ Card {} Badge {} }"), ["Element", "Element"]);
        // One element in braces is the unbraced arm, not an expression.
        assert_eq!(first_arm("{ div {} }"), ["Element"]);
    }

    /// A string literal opens rsx children too.
    #[test]
    fn a_braced_arm_may_start_with_text() {
        assert_eq!(first_arm(r#"{ "a" span {} }"#), ["Text", "Element"]);
        assert_eq!(first_arm(r#"{ "a" }"#), ["Text"]);
    }

    /// Control flow followed by more nodes is children; control flow alone is
    /// still the one transparent construct #221 made it.
    #[test]
    fn a_braced_arm_may_start_with_control_flow() {
        assert_eq!(
            first_arm(r#"{ if c.get() { "a" } span {} }"#),
            ["IfBlock", "Element"]
        );
        assert_eq!(
            first_arm(r#"{ for i in v.get() { div {} } "tail" }"#),
            ["ForLoop", "Text"]
        );
        assert_eq!(
            first_arm(r#"{ match y.get() { 0 => "z", _ => "n" } div {} }"#),
            ["MatchBlock", "Element"]
        );
        assert_eq!(
            first_arm(r#"{ match y.get() { 0 => "z", _ => "n" } }"#),
            ["MatchBlock"]
        );
    }

    /// `let` keeps opening children (issue #102 Gap 2), now with several nodes.
    #[test]
    fn a_braced_let_arm_holds_several_nodes() {
        assert_eq!(
            first_arm("{ let k = 1; div {} span {} }"),
            ["Statement", "Element", "Element"]
        );
    }

    /// Everything else in a braced arm is the expression it always was. The
    /// call arm is `examples/ui-zoo/src/lib.rs`'s section routing; `{a.clone()}`
    /// is `rsx_captured_handle_branch.rs`'s; the issue's own suggestion (always
    /// children) broke both.
    #[test]
    fn a_braced_arm_expression_stays_an_expression() {
        assert_eq!(first_arm("{ overview_section(__scope) }"), ["Expr"]);
        assert_eq!(first_arm("{a.clone()}"), ["Expr"]);
        assert_eq!(first_arm("{|| count.get()}"), ["Expr"]);
        assert_eq!(first_arm("{ move || count.get() }"), ["Expr"]);
        assert_eq!(first_arm("{ panel }"), ["Expr"]);
        assert_eq!(
            first_arm("{ items.iter().map(f).collect::<Vec<_>>() }"),
            ["Expr"]
        );
        assert_eq!(first_arm("{ rinch::helper(__scope) }"), ["Expr"]);
    }

    /// `{ Name { … } }` reads as a component, as `Name { … }` unbraced does —
    /// the first token decides, so a Rust struct literal in a braced arm is no
    /// longer one. A *path* (`a::Point { … }`) is not an element name, so it
    /// stays an expression.
    #[test]
    fn a_braced_struct_literal_shape_is_a_component() {
        assert_eq!(first_arm("{ Point { x: 1, y: 2 } }"), ["Element"]);
        assert_eq!(first_arm("{ geom::Point { x: 1, y: 2 } }"), ["Expr"]);
    }

    /// #221's diagnostic still fires for a braced arm of non-rsx control flow.
    #[test]
    fn a_braced_arm_of_non_rsx_control_flow_is_still_rejected() {
        let input =
            "match x.get() { 0 => { if c.get() { helper() } else { other() } }, _ => \"b\" }";
        let msg = match parse_str::<RsxNode>(input) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("braced control flow with non-rsx bodies must not compile"),
        };
        assert!(msg.contains("renders once and never updates"), "{msg}");
    }

    /// The error a braced arm reports, as text.
    fn arm_error(arm: &str) -> String {
        let input = format!("match x.get() {{ 0 => {arm}, _ => \"b\" }}");
        match parse_str::<RsxNode>(&input) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("{arm} must not parse"),
        }
    }

    /// A typo in a multi-node arm led by control flow is reported at the typo
    /// (`class "x"` is an element named `class` missing its braces), not as a
    /// struct-literal error inside the valid `if` body, and not as #221's
    /// "renders once" — the author wrote no braced control flow (PR #1017
    /// review, F1).
    #[test]
    fn a_typo_after_leading_control_flow_is_reported_at_the_typo() {
        let msg = arm_error(r#"{ if c.get() { b { "a" } } span { "b" } i { class "x" } }"#);
        assert_eq!(msg, "expected curly braces");
        let msg = arm_error(
            r#"{ for i in v.get() { b { {i.to_string()} } } span { "b" } i { class "x" } }"#,
        );
        assert_eq!(msg, "expected curly braces");
        let msg = arm_error(r#"{ if c.get() { "a" } span { "b" "c" d } }"#);
        assert!(!msg.contains("renders once"), "{msg}");
        // `d` is the last token of `span { … }`, so the element runs out of input.
        assert_eq!(msg, "unexpected end of input, expected curly braces");
        // An element-led arm whose own head is the typo reports it there too,
        // not as a struct-literal error from the expression path.
        let msg = arm_error(r#"{ i { class "x" } span { "b" } }"#);
        assert_eq!(msg, "expected curly braces");
    }

    /// Control flow followed by something that starts no rsx node (a method
    /// call, an operator) is the one Rust expression it always was, not a
    /// children parse that fails at the `.` (PR #1017 review, F2).
    #[test]
    fn control_flow_followed_by_a_method_call_stays_an_expression() {
        assert_eq!(
            first_arm(r#"{ if a { "b" } else { "c" } .len() }"#),
            ["Expr"]
        );
        assert_eq!(
            first_arm(r#"{ if c.get() { "a" } else { "b" } == d }"#),
            ["Expr"]
        );
    }

    /// A literal or `Name { … }` with a method called on it is an expression,
    /// as before #395 (PR #1017 review, F3).
    #[test]
    fn a_method_on_a_leading_literal_or_struct_stays_an_expression() {
        assert_eq!(first_arm(r#"{ "a".to_string() }"#), ["Expr"]);
        assert_eq!(first_arm(r#"{ "a".into() }"#), ["Expr"]);
        assert_eq!(first_arm("{ Foo { a: 1 }.into_node(__scope) }"), ["Expr"]);
    }

    /// An arm may lead with an interpolation when more nodes follow; a lone
    /// braced expression keeps its old meaning, including a braced `match` of
    /// non-rsx arms, which is a plain Rust block there, not #221's error
    /// (PR #1017 review, F4).
    #[test]
    fn a_braced_arm_may_start_with_an_interpolation() {
        assert_eq!(first_arm("{ {label} span {} }"), ["Expr", "Element"]);
        assert_eq!(first_arm(r#"{ {|| x.get()} " items" }"#), ["Expr", "Text"]);
        assert_eq!(first_arm("{ {x} }"), ["Expr"]);
        // A braced head after control flow is another node.
        assert_eq!(
            first_arm(r#"{ if c.get() { "a" } { helper() } }"#),
            ["IfBlock", "Expr"]
        );
        // A double brace around rsx control flow keeps its pre-#395 meaning,
        // a plain Rust block: only a head followed by more nodes is children.
        assert_eq!(
            first_arm(r#"{ { match y.get() { 0 => "z", _ => "n" } } }"#),
            ["Expr"]
        );
        assert_eq!(
            first_arm("{ { match y.get() { 0 => helper(), _ => other() } } }"),
            ["Expr"]
        );
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
