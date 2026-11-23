//! `prefers-reduced-motion` (#2209).
//!
//! Stylo's servo build evaluates no `prefers-reduced-motion` media feature: it
//! is in Gecko's feature table only, so a query naming it fails to parse and
//! the whole query becomes `not all`. `(prefers-reduced-motion: reduce)` never
//! matched on desktop, and neither did `(… no-preference)` or
//! `not (… reduce)`, which a browser matches whenever the OS asks for nothing.
//!
//! `rewrite` runs on every author sheet before Stylo parses it, after
//! `webkit_box::rewrite`. In each `@media` prelude it replaces a
//! `(prefers-reduced-motion)`, `(prefers-reduced-motion: reduce)` or
//! `(prefers-reduced-motion: no-preference)` with a condition Stylo evaluates
//! to the same answer, given the document's preference:
//! `ALWAYS` (`(min-width: 0px)`, which every viewport matches) or `NEVER`
//! (`(scan)`, which servo never matches: it supports no `tv` media). Any other
//! value is left alone, and stays the parse error it is in Chrome too.
//!
//! The answer is baked into the parsed sheet, so a change of preference
//! re-parses the sheets that name the feature
//! (`RinchDocument::set_prefers_reduced_motion`), which keeps the source text
//! of each for that. Nothing else about a sheet changes: the rewritten
//! condition sits where the original was, in the same rule.
//!
//! The preference a new document starts with is the process's
//! ([`prefers_reduced_motion`]); rinch's shell sets it from the OS setting or
//! `App::reduced_motion`, and applies a change to every open document.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

/// A condition every viewport matches.
pub(crate) const ALWAYS: &str = "(min-width: 0px)";
/// A condition servo never matches.
pub(crate) const NEVER: &str = "(scan)";

const FEATURE: &str = "prefers-reduced-motion";

static REDUCE: AtomicBool = AtomicBool::new(false);

/// Set the process's reduced-motion preference: what a document created from
/// now on starts with, and what rinch's shell applies to the open ones.
pub fn set_prefers_reduced_motion(reduce: bool) {
    REDUCE.store(reduce, Ordering::Relaxed);
}

/// The process's reduced-motion preference (`false` until set).
pub fn prefers_reduced_motion() -> bool {
    REDUCE.load(Ordering::Relaxed)
}

/// Whether `css` names the feature anywhere (ASCII case-insensitively): the
/// sheets whose source has to be kept for a re-parse.
pub(crate) fn mentions(css: &str) -> bool {
    find_ci(css.as_bytes(), FEATURE.as_bytes(), 0).is_some()
}

/// `css` with every reduced-motion condition in an `@media` prelude replaced
/// by one Stylo evaluates to the answer for `reduce`.
pub(crate) fn rewrite(css: &str, reduce: bool) -> Cow<'_, str> {
    if !mentions(css) {
        return Cow::Borrowed(css);
    }
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_comment(bytes, i),
            b'"' | b'\'' => i = skip_string(bytes, i),
            b'@' if is_media_at(bytes, i) => {
                let start = i + "@media".len();
                let end = prelude_end(bytes, start);
                let prelude = &css[start..end];
                if let Cow::Owned(rewritten) = rewrite_prelude(prelude, reduce) {
                    out.push_str(&css[copied..start]);
                    out.push_str(&rewritten);
                    copied = end;
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    if copied == 0 {
        return Cow::Borrowed(css);
    }
    out.push_str(&css[copied..]);
    Cow::Owned(out)
}

/// One `@media` prelude, rewritten.
fn rewrite_prelude(prelude: &str, reduce: bool) -> Cow<'_, str> {
    let bytes = prelude.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_comment(bytes, i),
            b'"' | b'\'' => i = skip_string(bytes, i),
            b'(' => {
                // The innermost group only: one with no `(` before its `)`.
                let Some(close) = prelude[i + 1..].find([')', '(']).map(|p| i + 1 + p) else {
                    break;
                };
                if bytes[close] == b'(' {
                    i = close;
                    continue;
                }
                if let Some(matches) = answer(&prelude[i + 1..close], reduce) {
                    out.push_str(&prelude[copied..i]);
                    out.push_str(if matches { ALWAYS } else { NEVER });
                    copied = close + 1;
                }
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    if copied == 0 {
        return Cow::Borrowed(prelude);
    }
    out.push_str(&prelude[copied..]);
    Cow::Owned(out)
}

/// What a `( … )` group's content answers, when it is a reduced-motion
/// condition rinch evaluates; `None` for anything else.
fn answer(inner: &str, reduce: bool) -> Option<bool> {
    let squeezed: String = inner
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let value = squeezed.strip_prefix(FEATURE)?;
    match value {
        "" | ":reduce" => Some(reduce),
        ":no-preference" => Some(!reduce),
        _ => None,
    }
}

/// Whether an `@media` keyword starts at `i` (case-insensitive, and not the
/// start of a longer at-keyword).
fn is_media_at(bytes: &[u8], i: usize) -> bool {
    let kw = b"@media";
    bytes.len() >= i + kw.len()
        && bytes[i..i + kw.len()].eq_ignore_ascii_case(kw)
        && bytes
            .get(i + kw.len())
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_'))
}

/// The index of the `{` or `;` that ends a prelude starting at `i`, skipping
/// comments and strings (the end of the input when there is none).
fn prelude_end(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'{' | b';' => return i,
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_comment(bytes, i),
            b'"' | b'\'' => i = skip_string(bytes, i),
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The index just past the comment starting at `i`.
fn skip_comment(bytes: &[u8], i: usize) -> usize {
    find_ci(bytes, b"*/", i + 2).map_or(bytes.len(), |p| p + 2)
}

/// The index just past the string starting at `i` (its quote included).
fn skip_string(bytes: &[u8], i: usize) -> usize {
    let quote = bytes[i];
    let mut j = i + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'\n' => return j,
            b if b == quote => return j + 1,
            _ => j += 1,
        }
    }
    bytes.len()
}

/// The first index at or after `from` where `needle` occurs in `hay`, ASCII
/// case-insensitively.
fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len())
        .find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_is_answered_for_either_preference() {
        for (reduce, query, want) in [
            (true, "(prefers-reduced-motion: reduce)", ALWAYS),
            (false, "(prefers-reduced-motion: reduce)", NEVER),
            (true, "(prefers-reduced-motion)", ALWAYS),
            (false, "(prefers-reduced-motion)", NEVER),
            (true, "(prefers-reduced-motion: no-preference)", NEVER),
            (false, "(prefers-reduced-motion: no-preference)", ALWAYS),
            (true, "( Prefers-Reduced-Motion : REDUCE )", ALWAYS),
        ] {
            let css = format!("@media {query} {{ a {{ b: c }} }}");
            assert_eq!(
                rewrite(&css, reduce),
                format!("@media {want} {{ a {{ b: c }} }}"),
                "{query}, reduce = {reduce}"
            );
        }
    }

    #[test]
    fn combinations_keep_everything_else() {
        let css = "@media screen and (min-width: 3px) and (not (prefers-reduced-motion: reduce)), print { x {} }";
        assert_eq!(
            rewrite(css, false),
            "@media screen and (min-width: 3px) and (not (scan)), print { x {} }"
        );
    }

    #[test]
    fn an_unknown_value_comments_strings_and_other_rules_are_left_alone() {
        for css in [
            "@media (prefers-reduced-motion: always) { x {} }",
            "/* @media (prefers-reduced-motion) */ x {}",
            "x { content: \"@media (prefers-reduced-motion)\" }",
            "x { --prefers-reduced-motion: 1 }",
            "@supports (prefers-reduced-motion: reduce) { x {} }",
            "@mediaX (prefers-reduced-motion) { x {} }",
        ] {
            assert!(matches!(rewrite(css, true), Cow::Borrowed(_)), "{css}");
        }
    }

    #[test]
    fn a_sheet_that_never_names_the_feature_is_borrowed() {
        assert!(matches!(
            rewrite("@media (min-width: 1px) { x {} }", true),
            Cow::Borrowed(_)
        ));
        assert!(!mentions("x {}"));
        assert!(mentions("@media (PREFERS-reduced-motion) {}"));
    }

    /// The re-parse list holds only sheets still in the document: a theme
    /// replaced 50 times leaves one entry, not 50 (the theme names the
    /// feature by default, so every `set_theme_css` used to keep the old
    /// sheet and its source alive until the preference changed).
    #[test]
    fn a_replaced_theme_sheet_is_forgotten() {
        use crate::RinchDocument;
        let mut doc = RinchDocument::new();
        for i in 0..50 {
            doc.set_theme_css(&format!(
                "@media (prefers-reduced-motion: reduce) {{ .t{i} {{ width: 1px }} }}"
            ));
        }
        assert_eq!(doc.motion_sheets.len(), 1);
    }

    /// The same for a `<style>` whose text keeps changing.
    #[test]
    fn a_style_elements_old_sheets_are_forgotten() {
        use crate::RinchDocument;
        use rinch_core::dom::DomDocument;
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let style = doc.create_element("style");
        doc.append_child(body, style);
        for i in 0..50 {
            doc.set_text_content(
                style,
                &format!("@media (prefers-reduced-motion: reduce) {{ .t{i} {{ width: 1px }} }}"),
            );
            doc.resolve_layout(800.0 + i as f32, 600.0);
        }
        assert_eq!(doc.motion_sheets.len(), 1);
        // And a removed `<style>` leaves none.
        doc.remove_child(body, style);
        doc.resolve_layout(700.0, 600.0);
        assert_eq!(doc.motion_sheets.len(), 0);
    }
}
