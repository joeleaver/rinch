//! Declaration-level arithmetic on an element's inline `style` attribute.
//!
//! An element's inline style has more than one author. A component writes its
//! own declarations from `render` — every custom property it publishes to its
//! stylesheet travels that way — and the caller can then hand it a `style:`
//! prop, and a style shorthand (`p:`, `mt:`, `w:` …) is a third. The `style:`
//! prop used to be applied by writing the whole attribute, which erased the
//! other two (issue #647).
//!
//! [`StyleProp`] applies one author's declarations **over** whatever is already
//! there, last-wins per property, and remembers enough to take them off again
//! on a re-run without disturbing anyone else's.

use std::borrow::Cow;

use super::NodeHandle;

/// Split an inline-style string into its `(property, value)` declarations.
///
/// Quote- and bracket-aware: a `;` or `:` inside `"…"`, `'…'` or `url(…)` is
/// part of the value, not a separator. That matters for the shape that reaches
/// inline styles most often — a `background-image:
/// url("data:image/svg+xml;base64,…")`, whose value carries both. `/* … */`
/// comments are removed first, wherever they sit.
///
/// A property declared twice collapses the way CSSOM collapses it: the last
/// value, at the *first* declaration's position. A part with no top-level `:`,
/// or an empty property name, is dropped — it is not a declaration.
///
/// Values are kept verbatim, `!important` included, so a round trip through
/// [`serialize_declarations`] preserves what the author wrote.
pub fn split_declarations(css: &str) -> Vec<(String, String)> {
    let css = strip_comments(css);
    let mut out: Vec<(String, String)> = Vec::new();
    for part in split_top_level(&css) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(colon) = top_level_colon(part) else {
            continue;
        };
        let name = part[..colon].trim();
        let value = part[colon + 1..].trim();
        if name.is_empty() {
            continue;
        }
        match out.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = value.to_string(),
            None => out.push((name.to_string(), value.to_string())),
        }
    }
    out
}

/// Render declarations back into an inline-style string.
///
/// The spelling matches what `RinchDocument::set_styles` writes — `"a: 1; b: 2"`
/// — so a node whose style has been touched by both reads the same either way.
pub fn serialize_declarations(decls: &[(String, String)]) -> String {
    decls
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Remove `/* … */` comments, which a browser does before it sees declarations
/// at all — so a comment may sit anywhere, property name included.
///
/// Quote-aware, because `content: "/*"` is a string and not a comment. An
/// unterminated comment runs to the end, as CSS says it does. The common case
/// (no comment) borrows.
///
/// Every index cut here lands on an ASCII byte, so a multi-byte value —
/// `content: "→"` — is never split mid-character.
fn strip_comments(css: &str) -> Cow<'_, str> {
    if !css.contains("/*") {
        return Cow::Borrowed(css);
    }
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut keep_from = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => {
                quote = Some(b);
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                out.push_str(&css[keep_from..i]);
                let mut j = i + 2;
                while j + 1 < bytes.len() && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                    j += 1;
                }
                i = if j + 1 < bytes.len() {
                    j + 2
                } else {
                    bytes.len()
                };
                keep_from = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&css[keep_from..]);
    Cow::Owned(out)
}

/// Where the top-level separators are, honouring quotes and brackets.
///
/// `depth` counts `(`/`[`/`{`; CSS has no nesting inside a declaration value
/// beyond those, and an unbalanced closer simply saturates at zero rather than
/// going negative, so a malformed value degrades to "one declaration" instead
/// of panicking. Comments are gone before this runs.
struct Scanner {
    depth: u32,
    quote: Option<u8>,
    escaped: bool,
}

impl Scanner {
    fn new() -> Self {
        Self {
            depth: 0,
            quote: None,
            escaped: false,
        }
    }

    /// Feed one byte; answer whether it is a *top-level* byte, i.e. one a
    /// separator search may act on.
    fn step(&mut self, bytes: &[u8], i: usize) -> bool {
        let b = bytes[i];
        if let Some(q) = self.quote {
            if self.escaped {
                self.escaped = false;
            } else if b == b'\\' {
                self.escaped = true;
            } else if b == q {
                self.quote = None;
            }
            return false;
        }
        match b {
            b'"' | b'\'' => {
                self.quote = Some(b);
                false
            }
            b'(' | b'[' | b'{' => {
                self.depth += 1;
                false
            }
            b')' | b']' | b'}' => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
            _ => self.depth == 0,
        }
    }
}

fn split_top_level(css: &str) -> Vec<&str> {
    let bytes = css.as_bytes();
    let mut scanner = Scanner::new();
    let mut parts = Vec::new();
    let mut start = 0usize;
    for i in 0..bytes.len() {
        if scanner.step(bytes, i) && bytes[i] == b';' {
            parts.push(&css[start..i]);
            start = i + 1;
        }
    }
    parts.push(&css[start..]);
    parts
}

fn top_level_colon(part: &str) -> Option<usize> {
    let bytes = part.as_bytes();
    let mut scanner = Scanner::new();
    (0..bytes.len()).find(|&i| scanner.step(bytes, i) && bytes[i] == b':')
}

/// One author's `style:` declarations, laid over an element's own inline style.
///
/// `rsx!` emits one of these per reactive `style:` binding. The first `apply`
/// merges the caller's declarations in; a later `apply` first *undoes* the
/// previous one — restoring whatever value each declaration displaced, and
/// removing the ones that displaced nothing — so a re-run replaces the caller's
/// own declarations rather than stacking onto them, and leaves every
/// declaration the component wrote exactly as it found it.
///
/// A binding whose node is rebuilt on every run (a reactive *component*, whose
/// `render` returns a fresh element each time) must not carry state across runs
/// — the memory would describe a node that no longer exists, and could talk the
/// next run into removing a declaration the component had just written. Those
/// sites use [`NodeHandle::merge_style`], which is this with no memory.
#[derive(Debug, Default, Clone)]
pub struct StyleProp {
    /// The properties the last `apply` wrote, each with the value it displaced
    /// (`None` when it displaced nothing and was a pure addition).
    applied: Vec<(String, Option<String>)>,
}

impl StyleProp {
    /// Lay `css` over the node's inline style, taking the previous `apply`'s
    /// declarations off first.
    pub fn apply(&mut self, node: &NodeHandle, css: &str) {
        let existing = node.get_attribute("style").unwrap_or_default();

        // Nothing to merge with and nothing to undo: write the author's string
        // through untouched. This is the overwhelmingly common case — an HTML
        // element carries no inline style until `rsx!` gives it one — and
        // keeping it verbatim means a `style:` prop still reads back exactly as
        // written wherever there is no second author to compose with.
        if existing.trim().is_empty() && self.applied.is_empty() {
            node.set_attribute("style", css);
            self.applied = split_declarations(css)
                .into_iter()
                .map(|(k, _)| (k, None))
                .collect();
            return;
        }

        let mut decls = split_declarations(&existing);
        for (property, displaced) in &self.applied {
            match displaced {
                Some(value) => {
                    if let Some(slot) = decls.iter_mut().find(|(k, _)| k == property) {
                        slot.1 = value.clone();
                    }
                }
                None => decls.retain(|(k, _)| k != property),
            }
        }

        let mut applied = Vec::new();
        for (property, value) in split_declarations(css) {
            match decls.iter_mut().find(|(k, _)| *k == property) {
                Some(slot) => {
                    applied.push((property, Some(std::mem::replace(&mut slot.1, value))));
                }
                None => {
                    applied.push((property.clone(), None));
                    decls.push((property, value));
                }
            }
        }

        node.set_attribute("style", &serialize_declarations(&decls));
        self.applied = applied;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::mock::MockDomDocument;
    use crate::dom::traits::DomDocument;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn names(decls: &[(String, String)]) -> Vec<&str> {
        decls.iter().map(|(k, _)| k.as_str()).collect()
    }

    fn value<'a>(decls: &'a [(String, String)], property: &str) -> Option<&'a str> {
        decls
            .iter()
            .find(|(k, _)| k == property)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_trailing_semicolon_and_stray_whitespace_are_not_declarations() {
        let decls = split_declarations("  color : red ;  gap:4px;  ");
        assert_eq!(names(&decls), ["color", "gap"]);
        assert_eq!(value(&decls, "color"), Some("red"));
        assert_eq!(value(&decls, "gap"), Some("4px"));
    }

    /// A `;` inside a quoted `url()` is part of the value. This is not a corner
    /// case in practice: `;base64,` is in every data-URI background there is,
    /// and a naive split leaves `background-image: url("data:image/svg+xml`
    /// behind.
    #[test]
    fn a_data_uri_keeps_its_semicolons_and_colons() {
        let css =
            r#"background-image: url("data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="); color: red"#;
        let decls = split_declarations(css);
        assert_eq!(names(&decls), ["background-image", "color"]);
        assert_eq!(
            value(&decls, "background-image"),
            Some(r#"url("data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=")"#)
        );
    }

    /// The same value **unquoted**, which is legal CSS and takes the bracket
    /// arm of the scanner rather than the quote arm.
    #[test]
    fn an_unquoted_url_keeps_its_semicolons() {
        let decls = split_declarations("background: url(data:image/png;base64,AAA=) no-repeat");
        assert_eq!(names(&decls), ["background"]);
        assert_eq!(
            value(&decls, "background"),
            Some("url(data:image/png;base64,AAA=) no-repeat")
        );
    }

    #[test]
    fn a_comment_is_not_a_separator_and_does_not_close_on_its_own_opener() {
        let decls = split_declarations("color: red /*; gap: 1px */; margin: 0");
        assert_eq!(names(&decls), ["color", "margin"]);
        let decls = split_declarations("/*/ still a comment; */ color: red");
        assert_eq!(names(&decls), ["color"]);
        assert_eq!(value(&decls, "color"), Some("red"));
        // A comment inside the property name is removed rather than kept as
        // part of it, which is what a browser does — it never sees one.
        let decls = split_declarations("col/**/or: red");
        assert_eq!(names(&decls), ["color"]);
        // …but `/*` inside a string is a string.
        let decls = split_declarations(r#"content: "/*"; color: red"#);
        assert_eq!(names(&decls), ["content", "color"]);
    }

    /// `!important` is part of the value and survives a round trip, so a merge
    /// cannot quietly demote a declaration the author marked.
    #[test]
    fn important_survives_a_round_trip() {
        let decls = split_declarations("color: red !important");
        assert_eq!(value(&decls, "color"), Some("red !important"));
        assert_eq!(serialize_declarations(&decls), "color: red !important");
    }

    /// A property declared twice collapses like CSSOM: the last value, at the
    /// first position. Two other properties either side, because a two-element
    /// list gives the same answer whichever position rule is in force.
    #[test]
    fn a_repeated_property_keeps_the_first_position_and_the_last_value() {
        let decls = split_declarations("a: 1; color: red; b: 2; color: blue");
        assert_eq!(names(&decls), ["a", "color", "b"]);
        assert_eq!(value(&decls, "color"), Some("blue"));
    }

    #[test]
    fn a_part_with_no_colon_is_dropped() {
        let decls = split_declarations("color: red; nonsense; : 4px; gap: 4px");
        assert_eq!(names(&decls), ["color", "gap"]);
    }

    // ── StyleProp ───────────────────────────────────────────────────────────

    /// A node carrying its own declarations, the way a component's `render`
    /// leaves its root.
    fn node_with(style: &str) -> (Rc<RefCell<MockDomDocument>>, NodeHandle) {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let id = doc.borrow_mut().create_element("div");
        let weak: std::rc::Weak<RefCell<dyn DomDocument>> = {
            let rc: Rc<RefCell<dyn DomDocument>> = doc.clone();
            Rc::downgrade(&rc)
        };
        let node = NodeHandle::new(id, weak);
        if !style.is_empty() {
            node.set_attribute("style", style);
        }
        (doc, node)
    }

    #[test]
    fn an_empty_node_takes_the_authors_string_verbatim() {
        let (_doc, node) = node_with("");
        StyleProp::default().apply(&node, "color:red;gap:4px");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("color:red;gap:4px"),
            "nothing to compose with, so nothing to reformat"
        );
    }

    #[test]
    fn a_collision_goes_to_the_later_author_and_the_rest_is_kept() {
        let (_doc, node) = node_with("--overlay-z: 517; margin: 8px");
        StyleProp::default().apply(&node, "margin: 0");
        let decls = split_declarations(&node.get_attribute("style").unwrap());
        assert_eq!(value(&decls, "--overlay-z"), Some("517"));
        assert_eq!(value(&decls, "margin"), Some("0"));
    }

    /// The whole point of the memory: a second `apply` replaces the first
    /// one's declarations instead of stacking onto them, and gives back the
    /// value each displaced.
    #[test]
    fn a_second_apply_undoes_the_first_without_touching_anyone_else() {
        let (_doc, node) = node_with("--overlay-z: 517; margin: 8px");
        let mut prop = StyleProp::default();
        prop.apply(&node, "margin: 0; padding: 4px");
        prop.apply(&node, "color: red");

        let decls = split_declarations(&node.get_attribute("style").unwrap());
        assert_eq!(
            value(&decls, "padding"),
            None,
            "a pure addition the author has stopped making goes away"
        );
        assert_eq!(
            value(&decls, "margin"),
            Some("8px"),
            "an override the author has stopped making hands the value back"
        );
        assert_eq!(value(&decls, "color"), Some("red"));
        assert_eq!(
            value(&decls, "--overlay-z"),
            Some("517"),
            "a property this author never declared is untouched throughout"
        );
    }

    /// A declaration the caller adds lands *after* the ones already there, so
    /// a caller longhand beats a component shorthand — which is the order CSS
    /// gives a later declaration in the same block.
    #[test]
    fn an_added_declaration_lands_last() {
        let (_doc, node) = node_with("inset: 0");
        StyleProp::default().apply(&node, "left: 25px");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("inset: 0; left: 25px")
        );
    }

    /// An author that declares nothing removes only what it had declared.
    #[test]
    fn an_empty_string_clears_only_this_authors_declarations() {
        let (_doc, node) = node_with("--overlay-z: 517");
        let mut prop = StyleProp::default();
        prop.apply(&node, "color: red");
        prop.apply(&node, "");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("--overlay-z: 517")
        );
    }
}
