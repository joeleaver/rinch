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
//! "Per property" is CSS's own notion of one property, not a string match:
//! names are folded with [`normalize_property_name`], so `COLOR` and `color`
//! are one and `--Foo` and `--foo` are two (#711).
//!
//! **The fold reaches a name iff that name came through the parser.**
//! `StyleProp`'s memory of what it wrote and the declarations already on the
//! node both did, so neither restates the rule. A name a caller hands
//! `set_style` did **not** — it never passes through [`split_declarations`] —
//! so the three sites that compare one call [`normalize_property_name`]
//! themselves: `RinchDocument::merged_inline_style`,
//! `MockDomDocument::set_style` and
//! `rinch-dom`'s `paint::contenteditable::get_style_property`. A new by-name
//! consumer has to ask which side its name comes from, and call the function
//! if the answer is "a caller". Dropping any one of those three calls is a
//! mutant the suite kills, which is the same statement in a form that fails.
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
/// url("data:image/svg+xml;base64,…")`, whose value carries both. A `\` escapes
/// the byte after it, so `url(http://a/x\)y;z.png)` is one token too.
///
/// `/* … */` comments are removed first — from a property name as readily as a
/// value, and each leaves a **space**, since a comment ends the token before it.
/// Two places CSS does not see one, so neither does this: inside a string, and
/// inside an unquoted `url(…)`. One where it sees one and keeps it, which rinch
/// does not: the interior of a **custom property** value (Refs #713, a
/// serialisation difference over an identical token stream). See
/// [`strip_comments`].
///
/// **Property names are compared ASCII case-insensitively** and come out
/// lowercased, because that is what CSS does with them — `COLOR` and `color`
/// are one property, and `MARGIN-left: 7px` serialises through Chrome 150's
/// CSSOM as `margin-left: 7px` (#711). A **custom property** is the exception
/// and stays exactly as written: `--Foo` and `--foo` are two properties, in
/// Chrome and here. See [`normalize_property_name`].
///
/// A property declared twice collapses the way CSSOM collapses it: the earlier
/// declaration is dropped and the **last** one keeps its own position — but
/// the value that survives is the **important** one where the two disagree,
/// which is not always the last. Both halves are measured in Chrome 150:
/// `margin: 1px; color: red !important; gap: 2px; color: blue` serialises as
/// `margin: 1px; gap: 2px; color: red !important`, so the slot moved to the
/// end while the value stayed at the first declaration's. Two importants
/// collapse to the last, as two plain ones do.
///
/// **Where that stops being CSSOM's rule (#722):** this collapse is *syntactic*
/// and a browser's is *post-validity*. Blink drops an invalid declaration when it
/// parses it, so the duplicate never competes — `color: notacolor !important;
/// color: blue` and `color: red !important !important; color: blue` both
/// compute blue in Chrome 150, where rinch keeps the important declaration,
/// Stylo then rejects it, and the property falls to its initial value. The
/// deviation is older than the priority arm (`color: blue; color: notacolor`
/// has always diverged the same way) but the arm widened it, since the
/// unconditional collapse used to discard those two for the wrong reason.
/// Closing it means deciding validity here, which this parser deliberately does
/// not do — it keeps values verbatim and leaves every question about them to
/// Stylo, so it is **#722** rather than something to patch here. Pinned by
/// `inline_style_case_tests::a_duplicate_collapses_before_validity_not_after`,
/// which flips when #722 is closed.
///
/// Measured in Chrome 150, because the position is not cosmetic. For
/// `margin: 1px; color: red; gap: 2px; color: blue`, `style.cssText` is
/// `margin: 1px; gap: 2px; color: blue;` — `color` at the *last* position, not
/// the first. (The attribute itself reads back as the author wrote it until
/// something touches the CSSOM, so `getAttribute` is not where the collapse
/// shows.) It is behaviourally observable wherever a shorthand and one of its
/// longhands are involved: Chrome gives `inset: 0px; left: 25px; inset: 4px` a
/// computed `left` of `4px`, because the surviving `inset` sits after the
/// `left` it overrides — where collapsing at the first position gives
/// `inset: 4px; left: 25px` and a computed `left` of `25px`.
///
/// **This is the whole workspace's inline-style parser** (#670).
/// `RinchDocument::set_styles` and `MockDomDocument::set_style` both split and
/// re-join with this pair, so an attribute is collapsed the same way whoever
/// rewrites it. `rinch-dom` used to keep its own `parse_style_string`, which
/// split on a bare `;`/`:` and collapsed at the *first* position — so a
/// `url(data:…)` already in the attribute was destroyed by the next unrelated
/// `set_style`, and a duplicate that survived this module's **verbatim** path
/// (which passes the author's string through unchanged) was collapsed the
/// other way as soon as a second author touched the node. Both are gone;
/// `rsx_style_prop::a_duplicate_in_a_style_prop_collapses_the_same_way_for_
/// the_other_author` is what pins the two authors agreeing, at Chrome's value.
///
/// Not every `;`-splitter in the workspace is this one: a few *read-only*
/// lookups still scan a style string for one property
/// (`rinch-editor-core`'s `serialize::html::parse_style`,
/// `rinch-visual-test`'s `strip_css_variables`), in crates that do not depend
/// on `rinch-core`. They cannot destroy a value — nothing round-trips through
/// them — and are **Refs #705**.
///
/// A part with no top-level `:`, or an empty property name, is dropped — it is
/// not a declaration.
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
        let mut value = part[colon + 1..].trim().to_string();
        if name.is_empty() {
            continue;
        }
        let name = normalize_property_name(name);
        if let Some(at) = out.iter().position(|(k, _)| k.as_str() == &*name) {
            let (_, displaced) = out.remove(at);
            // The slot moves to the last declaration's position either way;
            // only the *value* is decided by priority.
            if is_important(&displaced) && !is_important(&value) {
                value = displaced;
            }
        }
        out.push((name.into_owned(), value));
    }
    out
}

/// A property name as CSS compares it: ASCII-lowercased, unless it is a
/// **custom property**.
///
/// CSS property names are ASCII case-insensitive (css-syntax-3 tokenises them
/// as identifiers and every consumer matches them that way), so `COLOR` and
/// `color` are one property and the two cannot both be declared. Custom
/// properties are case-**sensitive** by css-variables-1 §2, which is not a
/// quirk to be normalised away: a design system may legitimately publish
/// `--Foo` and `--foo` as two variables. Measured in Chrome 150,
/// `style="--Foo: 1px; --foo: 2px"` has `length === 2` and
/// `setProperty("--Foo", …)` on a block already holding `--foo` adds a second
/// entry rather than overwriting the first.
///
/// This is the rule for **both** ends of an inline style: the names
/// [`split_declarations`] parses out of an attribute, and the name a caller
/// hands `set_style`. CSSOM lowercases at both too — `setProperty("COLOR",
/// "green")` on an empty block gives `cssText === "color: green;"` and
/// `length === 1`, also measured — and if only one end normalised, a
/// `set_style("COLOR", …)` would add a second declaration of a property the
/// block already had.
///
/// The common case (already lowercase) borrows.
pub fn normalize_property_name(name: &str) -> Cow<'_, str> {
    if name.starts_with("--") || !name.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.to_ascii_lowercase())
    }
}

/// Whether a declaration value ends in a **top-level** `!important`.
///
/// Only consulted when a property is declared twice, so the common path pays
/// nothing for it.
///
/// "Top-level" is [`Scanner`]'s rule, and it is what keeps
/// `content: "a !important"` an ordinary declaration — measured in Chrome 150,
/// which serialises that pair back as `content: "a !important"; color: blue`
/// with the `color` still overridable. `url(x)!important` is important, because
/// the `!` is outside the brackets; `url(a!important)` is not.
///
/// The spelling is loose on both sides of the word, as CSS is: `red!important`,
/// `red ! IMPORTANT` and `red !ImPoRtAnT` are all important (the last two
/// measured). Comments are gone before this runs, and each left a space, so a
/// `red /*c*/ !important` has already become `red   !important`.
fn is_important(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut scanner = Scanner::new();
    let mut bang = None;
    for i in 0..bytes.len() {
        if scanner.step(bytes, i) && bytes[i] == b'!' {
            bang = Some(i);
        }
    }
    bang.is_some_and(|at| value[at + 1..].trim().eq_ignore_ascii_case("important"))
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
/// Each becomes a **space**, not nothing: a comment is not a token, but it
/// ends the one before it, so deleting it textually glues two tokens into one.
/// Three measured consequences of getting that wrong are in
/// `a_removed_comment_still_separates_the_tokens_it_sat_between`.
///
/// **Two places a `/*` is not a comment**, and both are destructive to get
/// wrong, because everything here is on the round trip `set_style` puts an
/// author's whole attribute through:
///
/// - **inside a string**, because `content: "/*"` is a string; and
/// - **inside an unquoted `url(…)`**, because a url-token is a *single* token
///   and CSS never looks inside one for a comment. Measured in Chrome 150:
///   `background-image: url(http://a/*b*/c.png)` keeps the `/*b*/`, and
///   stripping it resolves a **different image**. The unterminated shape is
///   worse — an unterminated comment runs to the end of the string, as CSS
///   says it does, so treating `url(http://a/*b.png); color: red` as one would
///   swallow the `color` declaration whole (#670 review, F1).
///
/// `url("…")` is not this case: that is a function taking a string, so the
/// quote machinery already covers it. A comment inside any *other* function is
/// a comment — `width: calc(10px /* x */ + 5px)` loses it, as in Chrome — so
/// this is narrower than "anything in brackets", deliberately.
///
/// The common case (no comment) borrows.
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
        // A url-token is one token: every byte of it is kept, `/*` included.
        if let Some(end) = unquoted_url_end(bytes, i) {
            i = end;
            continue;
        }
        match b {
            b'"' | b'\'' => {
                quote = Some(b);
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                out.push_str(&css[keep_from..i]);
                // A **space**, not nothing. A comment is consumed by the
                // tokenizer and produces no token, but it still ends the token
                // before it — deleting it textually glues them into one.
                // Measured in Chrome 150: `font-family: Foo/*c*/Bar` is the
                // two-ident family `"Foo Bar"`, `letter-spacing: 1/*c*/px` is
                // invalid (not `1px`), and `col/**/or: red` is not a `color`
                // declaration at all. Deleting gave rinch `FooBar`, `1px` and
                // `color` — three different answers from one missing byte.
                out.push(' ');
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

/// If an **unquoted** `url(` token starts at `i`, the index just past its
/// closing `)` — or the end of the input when it has none, which is what CSS's
/// bad-url-token does.
///
/// `None` for everything else, including `url("…")`: that is a function token
/// taking a string, and the caller's quote handling already covers it.
///
/// The returned index is always a UTF-8 boundary: it is either one past an
/// ASCII `)` or the end of the slice. The `\` skip may *step* onto a
/// continuation byte, which matches neither arm and is simply stepped over.
fn unquoted_url_end(bytes: &[u8], i: usize) -> Option<usize> {
    if !bytes.get(i..i + 4)?.eq_ignore_ascii_case(b"url(") {
        return None;
    }
    // `myurl(` is not a url token — the `url` has to start the identifier.
    if i > 0
        && matches!(
            bytes[i - 1],
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'\\'
        )
    {
        return None;
    }
    let mut j = i + 4;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    if matches!(bytes.get(j), Some(b'"') | Some(b'\'')) {
        return None;
    }
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b')' => return Some(j + 1),
            _ => j += 1,
        }
    }
    Some(bytes.len())
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
    ///
    /// A `\` escapes the byte after it **everywhere**, not only inside a
    /// string. CSS says so, and it is also the rule
    /// [`unquoted_url_end`] already applies: without it the two disagree about
    /// one string, which is the defect class this module exists to remove.
    /// `url(http://a/x\)y;z.png)` is one url-token, so its `;` is not a
    /// separator — measured in Chrome 150, whose `background-image` reads back
    /// `url("http://a/x)y;z.png")`, the `color: red` after it intact.
    fn step(&mut self, bytes: &[u8], i: usize) -> bool {
        let b = bytes[i];
        if self.escaped {
            self.escaped = false;
            return false;
        }
        if b == b'\\' {
            self.escaped = true;
            return false;
        }
        if let Some(q) = self.quote {
            if b == q {
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
///   would be this author silently undoing their work. The converse is the
///   limit of the rule: a write that is **byte-identical** to this author's
///   last value is indistinguishable from it, so it is taken to be this
///   author's and taken back with the rest. Telling two authors apart when
///   they wrote the same bytes for the same property needs a per-author shadow
///   copy of the block, which this does not keep. Reaching it takes two
///   authors writing the same bytes for one property — either the
///   same-property collision described below, which is already the caller's
///   mistake, or a component rewriting its own root style after `render` to a
///   value the caller happens to be declaring too.
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
        // A comment inside the property name is removed — but it leaves the
        // space that keeps `col` and `or` two identifiers, so this is NOT a
        // `color` declaration. This fixture said it was until the #706 review
        // round: measured in Chrome 150, `col/**/or: red` produces an **empty**
        // `cssText` and no `color`, so merging them was rinch inventing a
        // declaration the author did not write. `col or` is not a property
        // Stylo knows either, so it is dropped at the cascade, which is the
        // browser's outcome by a different route.
        let decls = split_declarations("col/**/or: red");
        assert_eq!(names(&decls), ["col or"]);
        // …but `/*` inside a string is a string.
        let decls = split_declarations(r#"content: "/*"; color: red"#);
        assert_eq!(names(&decls), ["content", "color"]);
    }

    /// A `/*` inside an **unquoted** `url(…)` is part of the URL, not a
    /// comment — a url-token is one token and CSS never looks inside it.
    ///
    /// Both shapes are here because they fail differently and the second is
    /// the destructive one. Stripping a *terminated* `/*b*/` resolves a
    /// **different image**, quietly; an *unterminated* `/*` runs to the end of
    /// the string, so it would swallow every declaration after it. Measured in
    /// Chrome 150: `background-image: url(http://a/*b*/c.png)` serialises back
    /// with the `/*b*/` intact (#670 review, F1).
    #[test]
    fn a_comment_marker_inside_an_unquoted_url_is_part_of_the_url() {
        let decls = split_declarations("background-image: url(http://a/*b*/c.png)");
        assert_eq!(
            value(&decls, "background-image"),
            Some("url(http://a/*b*/c.png)")
        );

        // The unterminated opener: the declaration *after* it must survive.
        let decls = split_declarations("background-image: url(http://a/*b.png); color: red");
        assert_eq!(names(&decls), ["background-image", "color"]);
        assert_eq!(
            value(&decls, "background-image"),
            Some("url(http://a/*b.png)")
        );
        assert_eq!(value(&decls, "color"), Some("red"));
    }

    /// The positive control for the rule above, and the reason it is spelled
    /// as "a url-token" rather than "anything in brackets": a comment inside
    /// any *other* function is still a comment, which is what Chrome does.
    ///
    /// Without it, a bracket-depth spelling of the same fix would pass the
    /// `url()` fixture while diverging from the browser everywhere else, and
    /// nothing would say so.
    #[test]
    fn a_comment_inside_any_other_function_is_still_stripped() {
        let decls = split_declarations("width: calc(10px /* x */ + 5px)");
        // Three spaces: the two the author wrote around the comment, plus the
        // one it was replaced with.
        assert_eq!(value(&decls, "width"), Some("calc(10px   + 5px)"));

        // `url("…")` is a function taking a string, not a url-token — but the
        // `/*` is inside the string, so it survives for the *other* reason.
        let decls = split_declarations(r#"background-image: url("http://a/*b*/c.png")"#);
        assert_eq!(
            value(&decls, "background-image"),
            Some(r#"url("http://a/*b*/c.png")"#)
        );

        // And `url` has to start the identifier: `myurl(` is an ordinary
        // function, so a comment inside it goes.
        let decls = split_declarations("background-image: myurl(a/*b*/c)");
        assert_eq!(value(&decls, "background-image"), Some("myurl(a c)"));
    }

    /// A removed comment leaves a **space**, because a comment ends the token
    /// before it even though it is not a token itself. Deleting it textually
    /// glues two tokens into one, and all three of these were measured wrong
    /// in Chrome 150 before the #706 review round:
    ///
    /// | input | rinch before | Chrome |
    /// |---|---|---|
    /// | `font-family: Foo/*c*/Bar` | `FooBar` — a *different* family | `"Foo Bar"` |
    /// | `letter-spacing: 1/*c*/px` | `1px` — a valid declaration | invalid, dropped |
    /// | `col/**/or: red` | a `color` declaration | no declaration at all |
    ///
    /// The first is the one that matters: it is silent, well-formed, and names
    /// something else — the same shape as the `url()` defect the review found.
    #[test]
    fn a_removed_comment_still_separates_the_tokens_it_sat_between() {
        let decls = split_declarations("font-family: Foo/*c*/Bar");
        assert_eq!(value(&decls, "font-family"), Some("Foo Bar"));

        let decls = split_declarations("letter-spacing: 1/*c*/px");
        assert_eq!(value(&decls, "letter-spacing"), Some("1 px"));
    }

    /// A custom property is where rinch and CSSOM still part company over a
    /// comment, and the divergence is now **serialisation only**.
    ///
    /// Measured in Chrome 150: a custom property's value keeps an *interior*
    /// comment verbatim (`--x: 1px /* c */ 2px` reads back as
    /// `"1px /* c */ 2px"`) while a *leading* one is trimmed
    /// (`--x: /* c */red` → `"red"`). rinch strips both, so it agrees on the
    /// leading case and writes `1px  2px` for the interior one.
    ///
    /// Pinned at rinch's value rather than the browser's, deliberately: with
    /// the comment replaced by a space the two token streams are identical —
    /// `<dimension 1px> <dimension 2px>` either way — so nothing a `var()`
    /// substitution can see differs, and matching Chrome exactly needs a
    /// declaration-aware comment pass (it would have to know the property name
    /// before it strips, and then keep interior comments while trimming
    /// leading and trailing ones). Refs #713.
    ///
    /// The *unterminated* case needs no carve-out at all: Chrome drops the
    /// `color: red` too, because an unterminated comment runs to the end of
    /// the string there as well.
    #[test]
    fn a_comment_in_a_custom_property_value_is_a_named_serialisation_deviation() {
        let decls = split_declarations("--x: 1px /* c */ 2px; color: red");
        assert_eq!(value(&decls, "--x"), Some("1px   2px"));
        assert_eq!(value(&decls, "color"), Some("red"));

        // The leading case, where rinch and Chrome agree.
        let decls = split_declarations("--x: /* c */red");
        assert_eq!(value(&decls, "--x"), Some("red"));

        // And the unterminated one, where they agree that it eats the rest.
        let decls = split_declarations("--x: a /* b; color: red");
        assert_eq!(names(&decls), ["--x"]);
        assert_eq!(value(&decls, "--x"), Some("a"));
    }

    /// A `\` escapes the byte after it outside a string as well as inside, so
    /// `url(http://a/x\)y;z.png)` is one url-token and its `;` is not a
    /// separator.
    ///
    /// The escape used to be honoured by `unquoted_url_end` and not by
    /// `Scanner` — one rule in two places, disagreeing, which is exactly what
    /// this module exists to stop. Measured in Chrome 150: `background-image`
    /// reads back `url("http://a/x)y;z.png")` with the `color: red` after it
    /// intact (#706 review round, item 3).
    #[test]
    fn an_escaped_paren_inside_a_url_does_not_end_it() {
        let decls = split_declarations(r"background-image: url(http://a/x\)y;z.png); color: red");
        assert_eq!(names(&decls), ["background-image", "color"]);
        assert_eq!(
            value(&decls, "background-image"),
            Some(r"url(http://a/x\)y;z.png)")
        );
        assert_eq!(value(&decls, "color"), Some("red"));
    }

    /// `!important` is part of the value and survives a round trip, so a merge
    /// cannot quietly demote a declaration the author marked.
    #[test]
    fn important_survives_a_round_trip() {
        let decls = split_declarations("color: red !important");
        assert_eq!(value(&decls, "color"), Some("red !important"));
        assert_eq!(serialize_declarations(&decls), "color: red !important");
    }

    /// A property declared twice collapses to the last value at the **last**
    /// position, the way CSSOM does — measured in Chrome 150, where
    /// `margin: 1px; color: red; gap: 2px; color: blue` has a `cssText` of
    /// `margin: 1px; gap: 2px; color: blue;`.
    ///
    /// Two other properties either side, because a two-element list gives the
    /// same answer whichever position rule is in force. The position is what
    /// makes this more than cosmetic: see
    /// `rsx_style_prop::a_repeated_property_collapses_where_css_says_it_does`,
    /// which asserts the computed `left` it decides.
    #[test]
    fn a_repeated_property_keeps_the_last_position_and_the_last_value() {
        let decls = split_declarations("a: 1; color: red; b: 2; color: blue");
        assert_eq!(names(&decls), ["a", "b", "color"]);
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
        // does. This used to be spelled as a hand-written attribute — the whole
        // string, `margin: 0` included — because `MockDomDocument::set_style`
        // appended with no separator and could not compose one (#666). It
        // merges the way both real backends do now, so the second author is
        // written as the call it actually is. The macro fixture
        // `a_reactive_style_prop_does_not_demote_a_shorthand` drives the real
        // backend.
        node.set_style("margin-top", "8px");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("margin: 0; margin-top: 8px"),
            "the mock composes what a real backend's `set_style` produces"
        );

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
        // Somebody else restyles the same property — through `set_style`,
        // which the mock composes correctly since #666.
        node.set_style("color", "green");

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

    // ===== #711: property names are ASCII case-insensitive =====

    /// Two spellings of one property collapse, and the survivor comes out
    /// lowercased. Measured in Chrome 150: `style="color: blue; COLOR: red"`
    /// computes red and serialises as `color: red;`.
    ///
    /// Kills the mutant `let name = part[..colon].trim();` — the
    /// `normalize_property_name` call dropped from `split_declarations`.
    #[test]
    fn a_property_name_is_ascii_case_insensitive() {
        let decls = split_declarations("color: blue; COLOR: red");
        assert_eq!(names(&decls), ["color"]);
        assert_eq!(value(&decls, "color"), Some("red"));

        // Every byte of the name, not just the first.
        let decls = split_declarations("MARGIN-LEFT: 5px; margin-left: 9px");
        assert_eq!(names(&decls), ["margin-left"]);
        assert_eq!(value(&decls, "margin-left"), Some("9px"));
    }

    /// A **custom** property is case-sensitive, so these stay two. Measured in
    /// Chrome 150: `style="--Foo: 1px; --foo: 2px"` has `length === 2` and
    /// `getPropertyValue` answers `1px` and `2px` respectively.
    ///
    /// Kills the mutant that lowercases unconditionally — the
    /// `starts_with("--")` arm of `normalize_property_name` removed.
    #[test]
    fn a_custom_property_name_is_case_sensitive() {
        let decls = split_declarations("--Foo: 1px; --foo: 2px");
        assert_eq!(names(&decls), ["--Foo", "--foo"]);
        assert_eq!(value(&decls, "--Foo"), Some("1px"));
        assert_eq!(value(&decls, "--foo"), Some("2px"));
    }

    /// A vendor prefix is an ordinary property name, not a custom one: a
    /// single leading `-` does not opt out of the fold, only `--` does.
    #[test]
    fn one_leading_dash_is_not_a_custom_property() {
        let decls = split_declarations("-WEBKIT-line-clamp: 2; -webkit-line-clamp: 3");
        assert_eq!(names(&decls), ["-webkit-line-clamp"]);
        assert_eq!(value(&decls, "-webkit-line-clamp"), Some("3"));
    }

    /// The **value** that survives a collapse is the important one; the
    /// **slot** is still the last declaration's.
    ///
    /// Measured in Chrome 150, and the two halves disagree here on purpose:
    /// `margin: 1px; color: red !important; gap: 2px; color: blue` serialises
    /// as `margin: 1px; gap: 2px; color: red !important;` — `color` moved to
    /// the end (the last declaration's position) carrying the first
    /// declaration's value. A fixture with the duplicate already last could
    /// not tell the two rules apart.
    ///
    /// Kills the mutant `out.remove(at);` — the unconditional last-wins
    /// collapse, i.e. the `is_important` arm dropped.
    #[test]
    fn an_important_value_survives_a_later_plain_declaration() {
        let decls = split_declarations("margin: 1px; color: red !important; gap: 2px; color: blue");
        assert_eq!(names(&decls), ["margin", "gap", "color"]);
        assert_eq!(value(&decls, "color"), Some("red !important"));

        // Across cases, which is #711's own shape.
        let decls = split_declarations("COLOR: red !important; color: blue");
        assert_eq!(names(&decls), ["color"]);
        assert_eq!(value(&decls, "color"), Some("red !important"));
    }

    /// Two importants collapse to the **last**, as two plain ones do — so the
    /// rule is "important beats plain", not "the first important wins".
    /// Chrome 150 serialises `color: red !important; color: blue !important`
    /// as `color: blue !important;`.
    ///
    /// Kills the mutant `if is_important(&displaced)` — priority compared on
    /// the displaced value alone, without `&& !is_important(&value)`.
    #[test]
    fn two_important_declarations_collapse_to_the_last() {
        let decls = split_declarations("color: red !important; color: blue !important");
        assert_eq!(value(&decls, "color"), Some("blue !important"));
    }

    /// …and a plain declaration does not resurrect an earlier plain one.
    /// The positive control for the fixture above: without it, an
    /// `is_important` that answered `true` for everything would pass both.
    #[test]
    fn two_plain_declarations_still_collapse_to_the_last() {
        let decls = split_declarations("color: red; color: blue");
        assert_eq!(value(&decls, "color"), Some("blue"));
    }

    /// `!important` is recognised the loose way CSS spells it, and **only at
    /// the top level** — the case that matters, because everything here is on
    /// the round trip an author's whole attribute takes.
    ///
    /// All four measured in Chrome 150: `red ! IMPORTANT` and `red !ImPoRtAnT`
    /// both parse as important (`getPropertyPriority` answers `"important"`),
    /// `content: "a !important"` does not (it serialises back intact beside an
    /// overridable `color: blue`), and a custom property carries priority like
    /// any other (`--x: 1 !important; --x: 2` keeps the `1`).
    #[test]
    fn an_important_flag_is_read_the_way_css_spells_it() {
        let loose = split_declarations("color: red ! IMPORTANT; color: blue");
        assert_eq!(value(&loose, "color"), Some("red ! IMPORTANT"));
        let mixed = split_declarations("color: red !ImPoRtAnT; color: blue");
        assert_eq!(value(&mixed, "color"), Some("red !ImPoRtAnT"));
        let tight = split_declarations("color: red!important; color: blue");
        assert_eq!(value(&tight, "color"), Some("red!important"));

        // Inside a *closed* string or bracket it is text. These agree with
        // Chrome but do not discriminate — see
        // `important_is_read_only_at_the_top_level` for the reason and the
        // fixture that does.
        let quoted = split_declarations(r#"content: "a !important"; content: "b""#);
        assert_eq!(value(&quoted, "content"), Some(r#""b""#));
        // A `!` *after* the brackets counts, though.
        let inside = split_declarations("background: url(a!important); background: none");
        assert_eq!(value(&inside, "background"), Some("none"));
        let outside = split_declarations("background: url(a)!important; background: none");
        assert_eq!(value(&outside, "background"), Some("url(a)!important"));

        // A custom property carries priority too.
        let custom = split_declarations("--x: 1 !important; --x: 2");
        assert_eq!(value(&custom, "--x"), Some("1 !important"));
    }

    /// The top-level rule, asserted **on [`is_important`] directly**, because
    /// the collapse cannot see it.
    ///
    /// A `!` inside a *closed* string or bracket always has the delimiter
    /// after it, so a naive "does the value end in `!important`" answers
    /// `false` there by accident and passes every fixture above. An *un*closed
    /// one is the case that separates them — and it can never be the earlier
    /// half of a duplicate, because an unterminated string or `url(` swallows
    /// the `;` and everything after it, so the declaration carrying it is
    /// always the last one in the block. Measured in Chrome 150:
    /// `style='font-family: A, "b !important; font-family: Z'` is **one**
    /// declaration whose priority is `""`, not two.
    ///
    /// Kills the mutant `value.rfind('!')` — `is_important` without the
    /// [`Scanner`]. The escaped case in
    /// [`an_escaped_bang_is_not_a_priority_flag`] kills it through the
    /// collapse as well, which is the only route that reaches a computed
    /// value.
    #[test]
    fn important_is_read_only_at_the_top_level() {
        assert!(is_important("red !important"));
        assert!(is_important("url(a)!important"));
        assert!(!is_important("red"));

        // Closed delimiters — the naive suffix test agrees here, which is why
        // these are not on their own enough.
        assert!(!is_important(r#""a !important""#));
        assert!(!is_important("url(a!important)"));

        // Unclosed ones, where it does not.
        assert!(!is_important(r#"A, "b !important"#));
        assert!(!is_important("url(a!important"));
        assert!(!is_important("calc(1px !important"));
    }

    /// A `\` escapes the `!`, so this is not a priority flag and the later
    /// plain declaration wins — measured in Chrome 150, where
    /// `color: red \!important; color: blue` computes **blue** while the
    /// unescaped twin computes red.
    ///
    /// The escape is the one shape that is both non-top-level *and* able to
    /// stand as the earlier half of a duplicate, since escaping a byte closes
    /// nothing and swallows nothing. So this is the only fixture that kills
    /// `value.rfind('!')` through an observable collapse.
    #[test]
    fn an_escaped_bang_is_not_a_priority_flag() {
        let decls = split_declarations(r"color: red \!important; color: blue");
        assert_eq!(value(&decls, "color"), Some("blue"));

        let unescaped = split_declarations("color: red !important; color: blue");
        assert_eq!(
            value(&unescaped, "color"),
            Some("red !important"),
            "positive control: the same pair without the escape"
        );
    }

    /// A comment between the value and its `!important` leaves the space that
    /// keeps them two tokens, so the flag is still read. Chrome 150 agrees
    /// (`color: red /*c*/ !important` has priority `"important"`).
    #[test]
    fn a_comment_before_important_does_not_hide_it() {
        let decls = split_declarations("color: red /*c*/ !important; color: blue");
        assert_eq!(value(&decls, "color"), Some("red   !important"));
    }

    /// `StyleProp` composes across cases too — every name it compares comes out
    /// of the parser, so it inherits the rule rather than restating it.
    ///
    /// The **first** assertion is the discriminating one: without the fold, a
    /// `style:` prop spelled `COLOR` lands *beside* the element's own `color`
    /// instead of over it. The revert half is asserted after it because the
    /// revert alone is a fixed point — an unfolded `COLOR` is taken back by
    /// removing the declaration it added, which reaches the same string by a
    /// different route.
    #[test]
    fn a_style_prop_overwrites_the_other_case_of_its_own_property() {
        let (_doc, node) = node_with("color: red; gap: 4px");
        let mut prop = StyleProp::default();
        prop.apply(&node, "COLOR: blue");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("color: blue; gap: 4px")
        );
        prop.apply(&node, "");
        assert_eq!(
            node.get_attribute("style").as_deref(),
            Some("color: red; gap: 4px"),
            "and the value it displaced comes back, under the element's own \
             spelling"
        );
    }

    /// `normalize_property_name` borrows when there is nothing to change,
    /// which is the overwhelmingly common case and the reason it returns a
    /// `Cow`.
    #[test]
    fn a_lowercase_name_is_not_reallocated() {
        assert!(matches!(
            normalize_property_name("margin-left"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(normalize_property_name("--Foo"), Cow::Borrowed(_)));
        assert!(matches!(
            normalize_property_name("MARGIN-left"),
            Cow::Owned(_)
        ));
    }
}
