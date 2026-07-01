//! Input rules — markdown-style shortcuts that rewrite text as you type.
//!
//! An [`InputRule`] pairs a `$`-anchored [`Regex`] (matched against the text before
//! the cursor in the current textblock) with a handler that, on a match, returns a
//! [`Transaction`] performing the rewrite (e.g. `"## "` → an `<h2>`, `"- "` → a
//! bullet list). Rules are contributed by plugins and aggregated into the editor
//! config; the view runs [`apply_input_rules`] on each text input *before* the
//! plain insert, and dispatches the rule's transaction instead when one fires.
//!
//! Faithful in shape to ProseMirror's `prosemirror-inputrules` (the salvaged
//! `rules.rs`), rebound to return `Option<Transaction>`.

use crate::model::{AttrValue, Attrs, Fragment, Mark, Node};
use crate::pos::Pos;
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::block_range_simple;
use regex::{Captures, Regex};
use std::rc::Rc;

/// How many characters before the cursor are considered for a match (PM's
/// `MAX_MATCH`). Keeps long paragraphs from re-matching `^`-anchored rules.
const MAX_MATCH: usize = 500;

/// The handler half of an [`InputRule`]: given the state, the regex captures, and
/// the document range `start..end` the match covers, build the rewrite
/// transaction (or `None` to decline).
pub type InputRuleHandler =
    Rc<dyn Fn(&EditorState, &Captures, usize, usize) -> Option<Transaction>>;

/// A single input rule: a pattern plus its handler.
#[derive(Clone)]
pub struct InputRule {
    /// The pattern, matched against the text before the cursor (should be anchored
    /// at the end with `$`, and usually at the start with `^`).
    pub pattern: Regex,
    handler: InputRuleHandler,
}

impl InputRule {
    /// Build a rule from a pattern string and a handler.
    ///
    /// # Panics
    /// Panics if `pattern` is not a valid regex (patterns are compile-time
    /// constants in plugin code, so a bad pattern is a programming error).
    pub fn new(
        pattern: &str,
        handler: impl Fn(&EditorState, &Captures, usize, usize) -> Option<Transaction> + 'static,
    ) -> InputRule {
        InputRule {
            pattern: Regex::new(pattern).expect("input rule pattern must be a valid regex"),
            handler: Rc::new(handler),
        }
    }
}

impl std::fmt::Debug for InputRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputRule")
            .field("pattern", &self.pattern.as_str())
            .finish_non_exhaustive()
    }
}

/// The text of `parent`'s inline content from `start_offset` to `end_offset`
/// (char offsets), with non-text inline leaves rendered as `\u{fffc}` (the object
/// replacement char, as PM does) so positions line up.
fn text_between(parent: &Node, start_offset: usize, end_offset: usize) -> String {
    let mut out = String::new();
    let mut pos = 0usize;
    for child in parent.content().iter() {
        if pos >= end_offset {
            break;
        }
        let len = if child.is_text() { child.text_len() } else { 1 };
        let child_end = pos + len;
        if child_end > start_offset {
            if let Some(t) = child.text() {
                let take_start = start_offset.saturating_sub(pos);
                let take_end = (end_offset - pos).min(len);
                out.extend(t.chars().skip(take_start).take(take_end - take_start));
            } else {
                out.push('\u{fffc}');
            }
        }
        pos = child_end;
    }
    out
}

/// Try each rule against the text before the (collapsed) cursor at `pos` plus the
/// just-typed `text`, returning the first rule's rewrite transaction. The caller
/// inserts `text` normally only when this returns `None`. Mirrors PM's
/// `inputRules` plugin `handleTextInput`.
///
/// `pos` is the position where `text` is about to be inserted (the selection must
/// be a collapsed cursor for a rule to fire).
pub fn apply_input_rules(
    state: &EditorState,
    rules: &[InputRule],
    pos: usize,
    text: &str,
) -> Option<Transaction> {
    let r = state.doc.resolve(Pos(pos)).ok()?;
    let parent = r.parent();
    // No input rules inside code blocks (raw text — PM checks `spec.code`).
    if parent.type_name() == "code_block" {
        return None;
    }
    let parent_offset = r.parent_offset();
    let from_offset = parent_offset.saturating_sub(MAX_MATCH);
    let mut before = text_between(parent, from_offset, parent_offset);
    before.push_str(text);

    let text_chars = text.chars().count();
    for rule in rules {
        if let Some(caps) = rule.pattern.captures(&before) {
            let whole = caps.get(0).unwrap();
            let match_chars = whole.as_str().chars().count();
            // The match's document start: walk back `match_chars - text_chars`
            // chars (the marker text that is already in the doc) from `pos`.
            let start = pos.saturating_sub(match_chars.saturating_sub(text_chars));
            let end = pos;
            if let Some(tr) = (rule.handler)(state, &caps, start, end) {
                return Some(tr);
            }
        }
    }
    None
}

// -- rule builders (PM-style) -------------------------------------------------

/// A rule that turns the current textblock into `node_type` when `pattern` matches
/// at the block start — e.g. `"## "` → heading. `attrs_from` derives the new node's
/// attributes from the match (e.g. heading level from the `#` count).
pub fn textblock_type_input_rule(
    pattern: &str,
    node_type: &'static str,
    attrs_from: impl Fn(&Captures) -> Attrs + 'static,
) -> InputRule {
    InputRule::new(pattern, move |state, caps, start, end| {
        let ty = state.schema().node_type(node_type)?.clone();
        if !ty.is_textblock() {
            return None;
        }
        let attrs = attrs_from(caps);
        let mut tr = state.tr();
        tr.delete(start, end).ok()?;
        tr.set_block_type(start, start, ty, attrs).ok()?;
        Some(tr)
    })
}

/// A rule that wraps the current block in `node_type` (a list/blockquote) when
/// `pattern` matches at the block start — e.g. `"- "` → bullet list. For list
/// wrappers an inner `list_item` is added automatically.
pub fn wrapping_input_rule(
    pattern: &str,
    node_type: &'static str,
    attrs_from: impl Fn(&Captures) -> Attrs + 'static,
) -> InputRule {
    InputRule::new(pattern, move |state, caps, start, end| {
        let wrap_ty = state.schema().node_type(node_type)?.clone();
        let attrs = attrs_from(caps);
        let mut tr = state.tr();
        tr.delete(start, end).ok()?;
        let r_from = tr.doc().resolve(Pos(start)).ok()?;
        let range = block_range_simple(&r_from, &r_from)?;
        // A list wraps its blocks in list_item; a blockquote wraps them directly.
        let needs_item = matches!(node_type, "bullet_list" | "ordered_list" | "task_list");
        if needs_item {
            let item_name = if node_type == "task_list" {
                "task_item"
            } else {
                "list_item"
            };
            let item_ty = state.schema().node_type(item_name)?.clone();
            tr.wrap(&range, &[(wrap_ty, attrs), (item_ty, Attrs::new())])
                .ok()?;
        } else {
            tr.wrap(&range, &[(wrap_ty, attrs)]).ok()?;
        }
        Some(tr)
    })
}

/// A rule that turns the current block into a single-item **task list** when the
/// user types a markdown task checkbox at the block start — `[ ] ` / `[] `
/// (unchecked) or `[x] ` / `[X] ` (checked). A task is typed as the bare checkbox
/// (not `- [ ] `): the bullet rule already claims `- `, so — like Tiptap — the
/// checkbox alone is the trigger.
pub fn task_list_input_rule() -> InputRule {
    InputRule::new(r"^\s*\[([ xX]?)\]\s$", |state, caps, start, end| {
        let list_ty = state.schema().node_type("task_list")?.clone();
        let item_ty = state.schema().node_type("task_item")?.clone();
        let checked = matches!(caps.get(1).map(|m| m.as_str()), Some("x") | Some("X"));
        let item_attrs = if checked {
            Attrs::from_iter([("checked", AttrValue::Bool(true))])
        } else {
            Attrs::new()
        };
        let mut tr = state.tr();
        tr.delete(start, end).ok()?;
        let r_from = tr.doc().resolve(Pos(start)).ok()?;
        let range = block_range_simple(&r_from, &r_from)?;
        tr.wrap(&range, &[(list_ty, Attrs::new()), (item_ty, item_attrs)])
            .ok()?;
        Some(tr)
    })
}

/// A rule that wraps the captured text in an inline `mark_type` when the closing
/// delimiter is typed — e.g. `**bold**` → bold, `==text==` → highlight, `` `x` `` →
/// code. The pattern must expose **two** capture groups: group 1 is the whole
/// delimited markup (`**bold**`) and group 2 is the inner text (`bold`). On a match
/// the delimiters are removed, the inner text is re-inserted carrying the mark, and
/// the mark is cleared from the stored set so it does not bleed into the next
/// keystroke. (Rust's `regex` has no look-around, so the patterns instead require the
/// inner text to neither start nor end with whitespace, and a leading `(?:^|\s)`
/// boundary keeps `_` from firing inside `snake_case`.)
pub fn mark_input_rule(pattern: &str, mark_type: &'static str) -> InputRule {
    InputRule::new(pattern, move |state, caps, start, end| {
        let mt = state.schema().mark_type(mark_type)?.clone();
        let whole_chars = caps.get(0)?.as_str().chars().count();
        let markup_chars = caps.get(1)?.as_str().chars().count();
        let inner = caps.get(2)?.as_str();
        if inner.is_empty() {
            return None;
        }
        // `start` is the doc position of the whole match (group 0); the delimited
        // markup (group 1) sits at its tail, so skip the leading-boundary chars.
        let markup_start = start + (whole_chars - markup_chars);
        let inner_len = inner.chars().count();
        let mut tr = state.tr();
        let node = state
            .schema()
            .text_with_marks(inner, vec![Mark::simple(mt.clone())])
            .ok()?;
        // Replace the already-typed markup (`markup_start..end`; the just-typed closing
        // delimiter is not yet in the doc) with the marked inner text.
        tr.replace_with(markup_start, end, Fragment::from_node(node))
            .ok()?;
        tr.set_selection(Selection::cursor(Pos(markup_start + inner_len)));
        // Don't carry the mark into the next character typed after the markup.
        tr.remove_stored_mark(&Mark::simple(mt));
        Some(tr)
    })
}

/// The default markdown-shortcut input rules: block shortcuts (heading, blockquote,
/// bullet/ordered/task list, code block) plus inline mark shortcuts (`**bold**`,
/// `*italic*`, `~~strike~~`, `==highlight==`, `` `code` ``). Contributed by the
/// markdown-rules plugin. Block rules are tried first; bold before italic so
/// `**x**` is not mis-read as italic.
pub fn markdown_input_rules() -> Vec<InputRule> {
    vec![
        // `# ` … `###### ` → heading levels 1–6
        textblock_type_input_rule(r"^(#{1,6})\s$", "heading", |caps| {
            let level = caps.get(1).map_or(1, |m| m.as_str().len()) as i64;
            Attrs::from_iter([("level", AttrValue::Int(level))])
        }),
        // ``` ``` ``` → code block (before the inline-code rule, which also triggers
        // on a backtick)
        textblock_type_input_rule(r"^```$", "code_block", |_| Attrs::new()),
        // `> ` → blockquote
        wrapping_input_rule(r"^\s*>\s$", "blockquote", |_| Attrs::new()),
        // `[ ] ` / `[x] ` → task list (before the bullet rule is harmless — different
        // trigger char — but keeps the list shortcuts grouped)
        task_list_input_rule(),
        // `- ` / `* ` / `+ ` → bullet list
        wrapping_input_rule(r"^\s*([-+*])\s$", "bullet_list", |_| Attrs::new()),
        // `1. ` → ordered list (start = the typed number)
        wrapping_input_rule(r"^(\d+)\.\s$", "ordered_list", |caps| {
            let start = caps
                .get(1)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(1);
            Attrs::from_iter([("start", AttrValue::Int(start))])
        }),
        // -- inline marks (closing-delimiter triggered) --
        // `**bold**` / `__bold__`
        mark_input_rule(r"(?:^|\s)(\*\*([^\s*](?:[^*]*[^\s*])?)\*\*)$", "bold"),
        mark_input_rule(r"(?:^|\s)(__([^\s_](?:[^_]*[^\s_])?)__)$", "bold"),
        // `*italic*` / `_italic_`
        mark_input_rule(r"(?:^|\s)(\*([^\s*](?:[^*]*[^\s*])?)\*)$", "italic"),
        mark_input_rule(r"(?:^|\s)(_([^\s_](?:[^_]*[^\s_])?)_)$", "italic"),
        // `~~strike~~`
        mark_input_rule(r"(?:^|\s)(~~([^\s~](?:[^~]*[^\s~])?)~~)$", "strike"),
        // `==highlight==`
        mark_input_rule(r"(?:^|\s)(==([^\s=](?:[^=]*[^\s=])?)==)$", "highlight"),
        // `` `code` ``
        mark_input_rule(r"(?:^|\s)(`([^\s`](?:[^`]*[^\s`])?)`)$", "code"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::model::Fragment;
    use crate::plugin::{Plugin, PluginKey};
    use std::rc::Rc;

    struct MdRules;
    impl Plugin for MdRules {
        fn key(&self) -> PluginKey {
            PluginKey("markdown-input-rules")
        }
        fn input_rules(&self) -> Vec<InputRule> {
            markdown_input_rules()
        }
    }

    fn state_with(text: &str) -> EditorState {
        let s = Rc::new(Schema::starter_kit());
        let para = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        EditorState::create(s, doc, vec![Rc::new(MdRules)])
    }

    #[test]
    fn heading_rule_fires_on_hash_space() {
        // paragraph "#"; cursor after '#' at pos 2; type ' '
        let state = state_with("#");
        let rules = state.input_rules();
        let tr = apply_input_rules(&state, rules, 2, " ").expect("heading rule should fire");
        let next = state.apply(tr);
        assert_eq!(next.doc.child(0).type_name(), "heading");
        assert_eq!(next.doc.child(0).attrs().get_int("level"), Some(1));
        // the "#" marker was removed
        assert_eq!(next.doc.child(0).content().size(), 0);
    }

    #[test]
    fn heading_level_from_hash_count() {
        let state = state_with("###");
        let rules = state.input_rules();
        // cursor after "###" at pos 4; type ' '
        let tr = apply_input_rules(&state, rules, 4, " ").expect("h3 rule should fire");
        let next = state.apply(tr);
        assert_eq!(next.doc.child(0).type_name(), "heading");
        assert_eq!(next.doc.child(0).attrs().get_int("level"), Some(3));
    }

    #[test]
    fn bullet_list_rule_wraps() {
        let state = state_with("-");
        let rules = state.input_rules();
        // cursor after "-" at pos 2; type ' '
        let tr = apply_input_rules(&state, rules, 2, " ").expect("bullet rule should fire");
        let next = state.apply(tr);
        assert_eq!(next.doc.child(0).type_name(), "bullet_list");
        assert_eq!(next.doc.child(0).child(0).type_name(), "list_item");
        assert_eq!(next.doc.child(0).child(0).child(0).type_name(), "paragraph");
    }

    #[test]
    fn no_rule_fires_mid_word() {
        // "ab#"; cursor after at pos 4; typing space should NOT match ^# (not at start)
        let state = state_with("ab#");
        let rules = state.input_rules();
        assert!(apply_input_rules(&state, rules, 4, " ").is_none());
    }

    /// The text content of the first block, ignoring marks.
    fn block_text(node: &Node) -> String {
        node.content()
            .iter()
            .filter_map(|c| c.text().map(str::to_string))
            .collect()
    }

    /// Whether any inline node in `doc` carries a mark named `name`.
    fn doc_has_mark(doc: &Node, name: &str) -> bool {
        let mut found = false;
        doc.nodes_between(0, doc.content_size(), &mut |node, _, _| {
            if node.marks().iter().any(|m| m.type_name() == name) {
                found = true;
            }
            true
        });
        found
    }

    /// Fire the rules for `marker` typed in a fresh paragraph: the doc starts with
    /// everything but the final char of `marker`, then that final char is the trigger.
    fn fire(marker: &str) -> Option<EditorState> {
        let (head, last) = marker.split_at(marker.len() - 1);
        let state = state_with(head);
        let rules = state.input_rules();
        // cursor sits after `head`: 1 (open paragraph) + head.chars().count()
        let pos = 1 + head.chars().count();
        apply_input_rules(&state, rules, pos, last).map(|tr| state.apply(tr))
    }

    #[test]
    fn bold_rule_double_star() {
        let next = fire("**bold**").expect("bold rule should fire");
        assert_eq!(block_text(next.doc.child(0)), "bold");
        assert!(doc_has_mark(&next.doc, "bold"));
        // the markup was removed entirely
        assert_eq!(next.doc.child(0).content().size(), 4);
    }

    #[test]
    fn bold_rule_double_underscore() {
        let next = fire("__bold__").expect("bold rule should fire");
        assert_eq!(block_text(next.doc.child(0)), "bold");
        assert!(doc_has_mark(&next.doc, "bold"));
    }

    #[test]
    fn italic_rule_single_star() {
        let next = fire("*it*").expect("italic rule should fire");
        assert_eq!(block_text(next.doc.child(0)), "it");
        assert!(doc_has_mark(&next.doc, "italic"));
        assert!(!doc_has_mark(&next.doc, "bold"));
    }

    #[test]
    fn highlight_rule_double_equals() {
        let next = fire("==hi==").expect("highlight rule should fire");
        assert_eq!(block_text(next.doc.child(0)), "hi");
        assert!(doc_has_mark(&next.doc, "highlight"));
    }

    #[test]
    fn code_rule_backtick() {
        let next = fire("`x`").expect("code rule should fire");
        assert_eq!(block_text(next.doc.child(0)), "x");
        assert!(doc_has_mark(&next.doc, "code"));
    }

    #[test]
    fn strike_rule_double_tilde() {
        let next = fire("~~no~~").expect("strike rule should fire");
        assert_eq!(block_text(next.doc.child(0)), "no");
        assert!(doc_has_mark(&next.doc, "strike"));
    }

    #[test]
    fn mark_rule_requires_a_boundary() {
        // "a*b"; typing the closing "*" gives "a*b*" — no `^`/whitespace before the
        // opening "*", so the italic rule must NOT fire inside a word.
        let state = state_with("a*b");
        let rules = state.input_rules();
        assert!(apply_input_rules(&state, rules, 4, "*").is_none());
    }

    #[test]
    fn mark_rule_rejects_empty_inner() {
        // "**"; typing a third "*" gives "***" — no inner text, no mark.
        let state = state_with("**");
        let rules = state.input_rules();
        assert!(apply_input_rules(&state, rules, 3, "*").is_none());
    }

    #[test]
    fn task_list_rule_unchecked() {
        let next = fire("[ ] ").expect("task rule should fire");
        let list = next.doc.child(0);
        assert_eq!(list.type_name(), "task_list");
        let item = list.child(0);
        assert_eq!(item.type_name(), "task_item");
        // Unset `checked` == the schema default (false) — an unchecked item, exactly as
        // `toggleTaskList`/paste create one.
        assert_ne!(item.attrs().get_bool("checked"), Some(true));
        assert_eq!(item.child(0).type_name(), "paragraph");
    }

    #[test]
    fn task_list_rule_no_inner_space() {
        let next = fire("[] ").expect("task rule should fire");
        assert_eq!(next.doc.child(0).type_name(), "task_list");
    }

    #[test]
    fn task_list_rule_checked() {
        let next = fire("[x] ").expect("checked task rule should fire");
        let item = next.doc.child(0).child(0);
        assert_eq!(item.type_name(), "task_item");
        assert_eq!(item.attrs().get_bool("checked"), Some(true));
    }
}
