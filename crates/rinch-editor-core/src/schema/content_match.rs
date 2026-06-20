//! Compiled content expressions.
//!
//! A node type's `content` string (e.g. `"inline*"`, `"block+"`, `"list_item+"`,
//! `"paragraph block*"`) describes which child sequences are valid. The old
//! engine matched these with an ad-hoc string-splitter (`schema/mod.rs`'s
//! `matches_content`) that was correct only for single-part expressions. This
//! module compiles an expression once (resolving group names like `block` to the
//! concrete node types in that group) and matches child sequences with a small,
//! correct dynamic program that also handles compound expressions.
//!
//! Grammar supported (the starter-kit's): a space-separated sequence of tokens,
//! each a node-type name or a group name, each with an optional `*` / `+` / `?`
//! quantifier. Alternation (`|`), grouping (`()`) and ranges (`{n,m}`) are not in
//! the starter-kit grammar and are intentionally unsupported until a schema needs
//! them.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quant {
    /// exactly one
    One,
    /// `?` — zero or one
    ZeroOrOne,
    /// `*` — zero or more
    ZeroOrMore,
    /// `+` — one or more
    OneOrMore,
}

#[derive(Clone, Debug)]
struct Part {
    /// The concrete node-type names this part accepts (a group is pre-expanded
    /// to its members, so matching never needs the schema again).
    types: HashSet<Box<str>>,
    quant: Quant,
}

impl Part {
    fn accepts(&self, name: &str) -> bool {
        self.types.contains(name)
    }
}

/// A compiled content expression for one node type.
///
/// An empty `ContentMatch` (no parts) means the type is a **leaf**: it permits no
/// children and matches only the empty child sequence (e.g. `text`, `hard_break`,
/// `image`, `horizontal_rule`).
#[derive(Clone, Debug, Default)]
pub struct ContentMatch {
    parts: Vec<Part>,
}

impl ContentMatch {
    /// The leaf content match — accepts no children.
    pub fn empty() -> Self {
        ContentMatch { parts: Vec::new() }
    }

    /// True if this type permits no children at all (a leaf type).
    pub fn is_leaf(&self) -> bool {
        self.parts.is_empty()
    }

    /// Compile a content expression. `expr` of `None` (a type with no `content`)
    /// yields the leaf match. `groups` maps a group name to the set of node-type
    /// names belonging to it.
    pub fn compile(expr: Option<&str>, groups: &HashMap<String, HashSet<Box<str>>>) -> Self {
        let Some(expr) = expr else {
            return ContentMatch::empty();
        };
        let mut parts = Vec::new();
        for tok in expr.split_whitespace() {
            let (name, quant) = split_quant(tok);
            // A malformed token (empty name, or leftover special chars from an
            // unsupported feature like `*+`, alternation `|`, or grouping `()`)
            // means a mis-specified schema. We have no fallible build path in M1,
            // so assert in dev and skip in release rather than silently compiling
            // an always-reject literal part.
            debug_assert!(
                !name.is_empty() && !name.contains(['*', '+', '?', '|', '(', ')']),
                "unsupported content-expression token: {tok:?}"
            );
            if name.is_empty() {
                continue;
            }
            // A declared group name expands to its member set; any other token is a
            // concrete node-type name matched literally. A declared-but-empty group
            // yields an unsatisfiable part (an empty set) — never a literal match on
            // the group name itself.
            let types = match groups.get(name) {
                Some(set) => set.clone(),
                None => {
                    let mut s = HashSet::new();
                    s.insert(Box::from(name));
                    s
                }
            };
            parts.push(Part { types, quant });
        }
        ContentMatch { parts }
    }

    /// Does any part of this expression accept a child of type `name`?
    ///
    /// This answers "can node type X ever appear as a child here" — the basis for
    /// `Schema::allows_child`. It does **not** check ordering/cardinality; use
    /// [`ContentMatch::matches`] for a full sequence check.
    pub fn accepts(&self, name: &str) -> bool {
        self.parts.iter().any(|p| p.accepts(name))
    }

    /// Does the exact child sequence `children` (by type name, in order) satisfy
    /// this content expression?
    ///
    /// A correct dynamic program: `reachable[i]` tracks whether some prefix of the
    /// parts can consume exactly `children[0..i]`. This handles compound
    /// expressions and overlapping quantifiers that the old greedy matcher got
    /// wrong.
    pub fn matches(&self, children: &[&str]) -> bool {
        let n = children.len();
        let mut reachable = vec![false; n + 1];
        reachable[0] = true;
        for part in &self.parts {
            let mut next = vec![false; n + 1];
            for i in 0..=n {
                if !reachable[i] {
                    continue;
                }
                match part.quant {
                    Quant::One => {
                        if i < n && part.accepts(children[i]) {
                            next[i + 1] = true;
                        }
                    }
                    Quant::ZeroOrOne => {
                        next[i] = true; // consume zero
                        if i < n && part.accepts(children[i]) {
                            next[i + 1] = true; // or one
                        }
                    }
                    Quant::ZeroOrMore => {
                        let mut j = i;
                        next[j] = true; // zero
                        while j < n && part.accepts(children[j]) {
                            j += 1;
                            next[j] = true;
                        }
                    }
                    Quant::OneOrMore => {
                        // requires >= 1: only mark positions strictly past i
                        let mut j = i;
                        while j < n && part.accepts(children[j]) {
                            j += 1;
                            next[j] = true;
                        }
                    }
                }
            }
            reachable = next;
        }
        reachable[n]
    }
}

fn split_quant(tok: &str) -> (&str, Quant) {
    if let Some(s) = tok.strip_suffix('*') {
        (s, Quant::ZeroOrMore)
    } else if let Some(s) = tok.strip_suffix('+') {
        (s, Quant::OneOrMore)
    } else if let Some(s) = tok.strip_suffix('?') {
        (s, Quant::ZeroOrOne)
    } else {
        (tok, Quant::One)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror the starter-kit groups so these tests are schema-independent.
    fn groups() -> HashMap<String, HashSet<Box<str>>> {
        let mut g: HashMap<String, HashSet<Box<str>>> = HashMap::new();
        let block: HashSet<Box<str>> = [
            "paragraph",
            "heading",
            "blockquote",
            "code_block",
            "bullet_list",
            "ordered_list",
            "task_list",
            "horizontal_rule",
        ]
        .into_iter()
        .map(Box::from)
        .collect();
        let inline: HashSet<Box<str>> = ["text", "hard_break", "image"]
            .into_iter()
            .map(Box::from)
            .collect();
        g.insert("block".into(), block);
        g.insert("inline".into(), inline);
        g
    }

    fn cm(expr: &str) -> ContentMatch {
        ContentMatch::compile(Some(expr), &groups())
    }

    #[test]
    fn leaf_matches_only_empty() {
        let leaf = ContentMatch::compile(None, &groups());
        assert!(leaf.is_leaf());
        assert!(leaf.matches(&[]));
        assert!(!leaf.matches(&["text"]));
        assert!(!leaf.accepts("text"));
    }

    #[test]
    fn star_allows_zero_or_more() {
        assert!(cm("inline*").matches(&[]));
        assert!(cm("inline*").matches(&["text"]));
        assert!(cm("inline*").matches(&["text", "hard_break", "text"]));
        assert!(!cm("inline*").matches(&["paragraph"]));
    }

    #[test]
    fn plus_requires_one() {
        assert!(!cm("block+").matches(&[]));
        assert!(cm("block+").matches(&["paragraph"]));
        assert!(cm("block+").matches(&["paragraph", "heading"]));
        assert!(!cm("block+").matches(&["text"]));
    }

    #[test]
    fn specific_type_name() {
        assert!(cm("list_item+").matches(&["list_item"]));
        assert!(cm("list_item+").matches(&["list_item", "list_item"]));
        assert!(!cm("list_item+").matches(&["paragraph"]));
    }

    #[test]
    fn text_star_is_the_text_node() {
        assert!(cm("text*").matches(&[]));
        assert!(cm("text*").matches(&["text"]));
        assert!(!cm("text*").matches(&["paragraph"]));
    }

    #[test]
    fn compound_paragraph_then_blocks() {
        assert!(cm("paragraph block*").matches(&["paragraph"]));
        assert!(cm("paragraph block*").matches(&["paragraph", "heading"]));
        assert!(!cm("paragraph block*").matches(&["heading"]));
        assert!(!cm("paragraph block*").matches(&[])); // leading paragraph required
    }

    #[test]
    fn optional_quantifier() {
        assert!(cm("image?").matches(&[]));
        assert!(cm("image?").matches(&["image"]));
        assert!(!cm("image?").matches(&["image", "image"]));
    }

    #[test]
    fn accepts_is_membership_not_ordering() {
        assert!(cm("block+").accepts("paragraph"));
        assert!(!cm("block+").accepts("text"));
        assert!(cm("inline*").accepts("text"));
    }
}
