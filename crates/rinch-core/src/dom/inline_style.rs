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
//!
//! It works by rewriting the whole attribute from its own parsed contents,
//! rather than by handing each declaration to
//! [`set_style`](super::NodeHandle::set_style). That is one code path for both
//! backends, and it carries `!important` with no special handling, which the
//! per-declaration route would need: measured in Chrome 150,
//! `el.style.setProperty("color", "red !important")` is a **no-op** (the value
//! does not parse; the attribute stays absent), while
//! `el.setAttribute("style", "color: red !important")` keeps it and
//! `getPropertyPriority("color")` answers `"important"`. A per-declaration
//! route *can* carry a priority — `setProperty("color", "red", "important")`
//! works, also measured — but only by splitting it out of the value first,
//! which means a second CSS parser in the write path and a third rule about
//! where priority lives. The same probe is where the web half of the bug was
//! confirmed: `setAttribute("style", …)` replaces the whole declaration block
//! there too, so `rinch-web` had this defect identically.

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
/// A property declared twice collapses to its last value, kept at the *first*
/// declaration's position. **That position is a deviation from CSSOM**, not a
/// match for it. Measured in Chrome 150: for
/// `margin: 1px; color: red; gap: 2px; color: blue`, `style.cssText` is
/// `margin: 1px; gap: 2px; color: blue;` — the *last* position. (The attribute
/// itself reads back as the author wrote it until something touches the CSSOM,
/// so `getAttribute` is not where the collapse shows.) The difference is
/// observable where a shorthand and one of its longhands are involved: Chrome
/// gives `inset: 0px; left: 25px; inset: 4px` a computed `left` of `4px` —
/// collapsing it to `inset: 4px` and dropping the longhand — where
/// re-serialising to `inset: 4px; left: 25px` computes `25px`. This mirrors
/// `rinch-dom`'s own `parse_style_string`, which has always collapsed this way,
/// so the two agree with each other; reconciling both with CSSOM is separate
/// work. A part with no top-level `:`, or an empty property name, is dropped —
/// it is not a declaration.
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

/// One declaration this author wrote, and what it has to know to take it back.
#[derive(Debug, Clone)]
struct Written {
    property: String,
    /// The value this author last wrote. If the property no longer holds it,
    /// somebody else has written it since and it is not ours to take back.
    value: String,
    /// The value it displaced — `None` when it displaced nothing and the
    /// declaration was a pure addition, so undoing it means removing it.
    displaced: Option<String>,
}

/// One author's `style:` declarations, laid over an element's own inline style.
///
/// `rsx!` emits one of these per reactive `style:` binding on a **stable** node.
/// The first `apply` merges the caller's declarations in; a later `apply` first
/// takes the previous one's declarations back, so a re-run replaces the
/// caller's own declarations rather than stacking onto them.
///
/// Three rules make "takes them back" mean what it says, and each is a case the
/// naive version got wrong:
///
/// - **A property this author still declares keeps its slot.** It is overwritten
///   in place, never removed and re-added, because re-adding appends — and a
///   declaration that moves to the end of the block changes which of two
///   colliding declarations wins. `div { style: {|| …}, mt: "8px" }` lost its
///   top margin on the first signal change that way: the caller's `margin`
///   shorthand was removed and re-appended *after* the prop's `margin-top`.
/// - **A property whose current value is not the one this author wrote is left
///   alone.** Somebody else has written it since — a component effect, a
///   `set_style`, the runtime — and reverting it to a value they never saw
///   would be this author silently undoing their work.
/// - **What a declaration reverts to is carried forward** across re-runs in
///   which this author keeps declaring it, so the value it originally displaced
///   is still what comes back when it finally stops.
///
/// What is deliberately *not* repaired: a genuine same-property collision with
/// a style shorthand. `div { style: {|| …}, p: "12px" }` where the closure also
/// names `padding` gives the shorthand at mount (it is applied last) and the
/// closure's value from the first re-fire onward, because nothing re-asserts a
/// shorthand after the fact. Declaring one property from two props on one
/// element is the bug; making shorthands reactive is separate work.
///
/// A binding whose node is rebuilt on every run (a reactive *component*, whose
/// `render` returns a fresh element each time) must not carry state across runs
/// — the memory would describe a node that no longer exists, and could talk the
/// next run into removing a declaration the component had just written. Those
/// sites use [`NodeHandle::merge_style`], which is this with no memory.
#[derive(Debug, Default, Clone)]
pub struct StyleProp {
    /// What the last `apply` wrote, in the order it wrote it.
    applied: Vec<Written>,
    /// The exact string last written through the verbatim path, if the last
    /// `apply` took it. While the attribute still reads back as this, the whole
    /// attribute is known to be this author's and nobody else has touched it,
    /// so the next `apply` can write verbatim again instead of re-serialising.
    verbatim: Option<String>,
}

impl StyleProp {
    /// Lay `css` over the node's inline style, taking the previous `apply`'s
    /// declarations back first.
    pub fn apply(&mut self, node: &NodeHandle, css: &str) {
        let existing = node.get_attribute("style").unwrap_or_default();

        // Nothing to compose with: write the author's string through untouched.
        // This is the overwhelmingly common case — an HTML element carries no
        // inline style until `rsx!` gives it one, and a reactive binding that
        // is the element's only author stays in this arm on every fire — and
        // keeping it verbatim means a `style:` prop reads back exactly as
        // written wherever there is nobody to compose with.
        let untouched = match &self.verbatim {
            Some(last) => *last == existing,
            None => existing.trim().is_empty() && self.applied.is_empty(),
        };
        if untouched {
            node.set_attribute("style", css);
            self.applied = split_declarations(css)
                .into_iter()
                .map(|(property, value)| Written {
                    property,
                    value,
                    displaced: None,
                })
                .collect();
            self.verbatim = Some(css.to_string());
            return;
        }

        let incoming = split_declarations(css);
        let mut decls = split_declarations(&existing);

        // 1. Take back the declarations this author is no longer making. One it
        //    *is* still making keeps its slot — step 2 overwrites it where it
        //    stands, which is what stops a re-run from reordering the block.
        for previous in &self.applied {
            if incoming.iter().any(|(k, _)| *k == previous.property) {
                continue;
            }
            let Some(at) = decls.iter().position(|(k, _)| *k == previous.property) else {
                continue;
            };
            if decls[at].1 != previous.value {
                continue;
            }
            match &previous.displaced {
                Some(value) => decls[at].1 = value.clone(),
                None => {
                    decls.remove(at);
                }
            }
        }

        // 2. Lay this author's declarations on, in place wherever the property
        //    is already declared.
        let mut applied = Vec::with_capacity(incoming.len());
        for (property, value) in incoming {
            let displaced = match decls.iter_mut().find(|(k, _)| *k == property) {
                Some(slot) => {
                    let was = std::mem::replace(&mut slot.1, value.clone());
                    match self.applied.iter().find(|w| w.property == property) {
                        // Overwriting our own previous value: what this
                        // declaration reverts to has not changed.
                        Some(previous) if previous.value == was => previous.displaced.clone(),
                        _ => Some(was),
                    }
                }
                None => {
                    decls.push((property.clone(), value.clone()));
                    None
                }
            };
            applied.push(Written {
                property,
                value,
                displaced,
            });
        }

        node.set_attribute("style", &serialize_declarations(&decls));
        self.applied = applied;
        self.verbatim = None;
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

    /// A `;` inside a **quoted** value is part of it. Separate from the
    /// bracket cases above: every other fixture whose value carries a `;` is
    /// inside `url(…)`, so deleting the scanner's quote arm left all of them
    /// green while `content: "a;b"` split into `("content", "\"a")`.
    #[test]
    fn a_quoted_semicolon_is_not_a_separator() {
        let decls = split_declarations(r#"content: "a;b"; color: red"#);
        assert_eq!(names(&decls), ["content", "color"]);
        assert_eq!(value(&decls, "content"), Some(r#""a;b""#));
        // An escaped closing quote does not end the string either.
        let decls = split_declarations(r#"content: "a\";b"; color: red"#);
        assert_eq!(names(&decls), ["content", "color"]);
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

    /// A property declared twice collapses to the last value, at the **first**
    /// position. Two other properties either side, because a two-element list
    /// gives the same answer whichever position rule is in force.
    ///
    /// This is a deviation from CSSOM, not a match for it — Chrome 150 keeps
    /// the last position — and it is pinned here as the deviation it is, so a
    /// future reconciliation with `rinch-dom`'s `parse_style_string` has to
    /// come through this fixture rather than around it. See the note on
    /// [`split_declarations`].
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

    /// A property the author keeps declaring is overwritten **where it
    /// stands**, never removed and re-appended.
    ///
    /// The position is the whole point: a longhand written by someone else
    /// after this author's shorthand only wins while it stays after it, so a
    /// re-apply that moved `margin` to the end would kill a `margin-top` that
    /// had been applying since mount. Two declarations either side of it,
    /// because a one-element list has no order to get wrong.
    #[test]
    fn a_property_the_author_keeps_declaring_holds_its_position() {
        let (_doc, node) = node_with("");
        let mut prop = StyleProp::default();
        prop.apply(&node, "margin: 0");
        // A second author adds a longhand after it, the way a `mt:` shorthand
        // does. Spelled as an attribute write rather than `set_style` because
        // `MockDomDocument::set_style` appends with no separator (#666); the
        // string below is what a real backend's `set_style` produces, and the
        // macro fixture `a_reactive_style_prop_does_not_demote_a_shorthand`
        // drives the real one.
        node.set_attribute("style", "margin: 0; margin-top: 8px");

        prop.apply(&node, "margin: 0; color: red");
        let decls = split_declarations(&node.get_attribute("style").unwrap());
        assert_eq!(
            names(&decls),
            ["margin", "margin-top", "color"],
            "`margin` must keep its slot, or `margin-top` stops winning"
        );
    }

    /// A property somebody else has written since is not taken back. Reverting
    /// it would be this author undoing a value they never saw.
    #[test]
    fn a_third_partys_write_is_not_taken_back() {
        let (_doc, node) = node_with("");
        let mut prop = StyleProp::default();
        prop.apply(&node, "color: red");
        // Somebody else restyles the same property (see #666 on why this is an
        // attribute write and not `set_style`).
        node.set_attribute("style", "color: green");

        prop.apply(&node, "gap: 4px");
        let decls = split_declarations(&node.get_attribute("style").unwrap());
        assert_eq!(
            value(&decls, "color"),
            Some("green"),
            "the author stopped declaring `color`, but the value there is no \
             longer the one it wrote"
        );
        assert_eq!(value(&decls, "gap"), Some("4px"));
    }

    /// What a declaration reverts to survives re-runs that keep declaring it.
    ///
    /// Three applies, because two cannot tell "carried forward" from "recorded
    /// on the first apply and never consulted again".
    #[test]
    fn the_displaced_value_is_carried_across_re_runs() {
        let (_doc, node) = node_with("margin: 8px");
        let mut prop = StyleProp::default();
        prop.apply(&node, "margin: 0");
        prop.apply(&node, "margin: 1px");
        prop.apply(&node, "color: red");
        let decls = split_declarations(&node.get_attribute("style").unwrap());
        assert_eq!(
            value(&decls, "margin"),
            Some("8px"),
            "the value the FIRST apply displaced is what comes back"
        );
    }

    /// The verbatim path holds on a re-run too, while this author is still the
    /// element's only one — so a reactive `style:` with nobody to compose with
    /// reads back exactly as written on every fire, not just the first.
    #[test]
    fn a_re_run_with_no_second_author_is_still_verbatim() {
        let (_doc, node) = node_with("");
        let mut prop = StyleProp::default();
        prop.apply(&node, "color:red;gap:4px");
        prop.apply(&node, "color:blue;gap:4px");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("color:blue;gap:4px")
        );
        // …and it stops as soon as there IS a second author.
        node.set_attribute("style", "color:blue;gap:4px;margin: 0");
        prop.apply(&node, "color:green");
        let decls = split_declarations(&node.get_attribute("style").unwrap());
        assert_eq!(value(&decls, "margin"), Some("0"));
        assert_eq!(value(&decls, "color"), Some("green"));
        assert_eq!(
            value(&decls, "gap"),
            None,
            "gap was this author's, and it stopped declaring it"
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
