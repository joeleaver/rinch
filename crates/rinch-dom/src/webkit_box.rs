//! `display: -webkit-box` / `-webkit-inline-box`, `-webkit-box-orient` and
//! `-webkit-line-clamp` (#1703).
//!
//! Stylo's servo build parses none of them: the two display values are
//! `#[cfg(feature = "gecko")]`, and the two properties are Gecko-only
//! longhands, so the declarations are dropped at parse and never reach the
//! cascade. rinch carries them through it as declarations Stylo keeps:
//! [`rewrite`] runs on every author sheet and every inline `style` (and
//! presentational hint) before Stylo parses it, and turns
//!
//! - `-webkit-line-clamp: N` into `--rinch-webkit-line-clamp: N`,
//! - `-webkit-box-orient: vertical` into `--rinch-webkit-box-orient: vertical`,
//! - `display: -webkit-box` into `display: flow-root; --rinch-webkit-display:
//!   -webkit-box` (`inline-block` for `-webkit-inline-box`): Chrome computes
//!   a box that clamps `flow-root`, and blockifies no child of one that does
//!   not (a `<span>` in a vertical `-webkit-box` stays inline, Chrome 155),
//!   so Stylo is given a display that blockifies nothing,
//!
//! each with the declaration's own `!important`. The rewritten declarations
//! sit in the rule they were written in, so specificity, origin, order and
//! importance are the cascade's own. Custom properties inherit, and these
//! three must not, so [`apply`] does not read their computed values: it walks
//! the element's own matched declarations in cascade order ([`own_winner`]),
//! the way Stylo's cascade iterates them, and takes the first. The display
//! marker only counts where it beats every `display` declaration.
//!
//! What Chrome 155 does, and so what [`apply`] makes of them:
//! - `-webkit-box` with `-webkit-box-orient: vertical` and a
//!   `-webkit-line-clamp` of N computes `display: flow-root`, and its lines
//!   stop at the Nth: the box is as tall as N lines, the Nth ends in "…",
//!   and the lines after it are still laid out and painted below (an
//!   `overflow: hidden` clips them);
//! - otherwise Chrome lays `-webkit-box` out as a flex container (a column
//!   under `vertical`) whose run of inline content is one anonymous item, so
//!   `text-overflow` draws no "…". rinch has no such anonymous item (#2581):
//!   it keeps the block Stylo was given, which matches Chrome for inline
//!   content (the common case, HTML email among it), and suppresses the
//!   `text-overflow` "…" (`ComputedStyle::webkit_box`);
//! - `-webkit-line-clamp` or `-webkit-box-orient` on a box that is not
//!   `-webkit-box` does nothing.
//!
//! Not done (#2581): the clamp counts only the lines of the box's own inline
//! formatting context, not those of block descendants; a legacy box that
//! does not clamp lays block children out as a block does (stacked), not as
//! flex items (a row unless `vertical`, not shrinking); a `var()` in either
//! property is not read. Known small divergences: `getComputedStyle`-style
//! readers see `flow-root`/`inline-block`, not `-webkit-box`; in nested CSS,
//! declarations before a nested rule in the same block are not rewritten; a
//! comment after the value makes the declaration unrecognised (Stylo then
//! drops it, as before).

use std::borrow::Cow;

use crate::computed_style::{ComputedStyle, DisplayValue};

const DISPLAY: &str = "rinch-webkit-display";
const CLAMP: &str = "rinch-webkit-line-clamp";
const ORIENT: &str = "rinch-webkit-box-orient";

/// Whether `haystack` contains `needle` (ASCII lower case), ignoring ASCII
/// case.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `value` without a trailing `!important`, and whether it had one.
fn split_important(value: &str) -> (&str, bool) {
    let t = value.trim_end();
    if let Some(bang) = t.rfind('!')
        && t[bang + 1..].trim().eq_ignore_ascii_case("important")
    {
        return (&t[..bang], true);
    }
    (value, false)
}

fn is_wide_keyword(v: &str) -> bool {
    ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
}

/// A `-webkit-line-clamp` value as Chrome parses it: `none` or an integer
/// of at least 1.
fn valid_clamp(v: &str) -> bool {
    v.eq_ignore_ascii_case("none")
        || is_wide_keyword(v)
        || v.strip_prefix('+')
            .unwrap_or(v)
            .parse::<u32>()
            .is_ok_and(|n| n >= 1)
}

fn valid_orient(v: &str) -> bool {
    ["horizontal", "vertical", "inline-axis", "block-axis"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
        || is_wide_keyword(v)
}

/// Split a declaration list at its top-level `;`s (outside strings,
/// brackets and comments). The pieces keep their text; the `;`s are not in
/// them.
fn split_declarations(list: &str) -> Vec<&str> {
    let bytes = list.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut depth, mut i) = (0, 0i32, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = list[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 1);
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b';' if depth <= 0 => {
                out.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&list[start.min(list.len())..]);
    out
}

/// Rewrite one declaration list (a rule's body or an inline `style`).
fn rewrite_list(list: &str, out: &mut String) {
    let decls = split_declarations(list);
    // `(lead, name, value)`: the lead is the text before the name (white
    // space, a comment), kept as it is.
    let parsed: Vec<Option<(&str, String, &str)>> = decls
        .iter()
        .map(|d| {
            let colon = d.find(':')?;
            let pre = &d[..colon];
            let after_comment = pre.rfind("*/").map_or(0, |e| e + 2);
            let name = pre[after_comment..].trim_start();
            let lead = &d[..colon - name.len()];
            Some((lead, name.trim_end().to_ascii_lowercase(), &d[colon + 1..]))
        })
        .collect();
    for (i, (decl, parsed)) in decls.iter().zip(&parsed).enumerate() {
        if i > 0 {
            out.push(';');
        }
        let Some((lead, name, value)) = parsed else {
            out.push_str(decl);
            continue;
        };
        let (v, important) = split_important(value);
        let v = v.trim();
        let bang = if important { " !important" } else { "" };
        match name.as_str() {
            "-webkit-line-clamp" if valid_clamp(v) => {
                out.push_str(&format!("{lead}--{CLAMP}: {v}{bang}"));
            }
            "-webkit-box-orient" if valid_orient(v) => {
                out.push_str(&format!("{lead}--{ORIENT}: {v}{bang}"));
            }
            "display"
                if v.eq_ignore_ascii_case("-webkit-box")
                    || v.eq_ignore_ascii_case("-webkit-inline-box") =>
            {
                let (stylo, marker) = if v.eq_ignore_ascii_case("-webkit-inline-box") {
                    ("inline-block", "-webkit-inline-box")
                } else {
                    ("flow-root", "-webkit-box")
                };
                out.push_str(&format!(
                    "{lead}display: {stylo}{bang}; --{DISPLAY}: {marker}{bang}"
                ));
            }
            _ => out.push_str(decl),
        }
    }
}

/// An `@supports` prelude with the spellings [`rewrite_list`] rewrites
/// rewritten the same way, so `@supports (-webkit-line-clamp: 2)` holds as
/// it does in Chrome; any other prelude as it is.
fn supports_prelude(prelude: &str) -> Cow<'_, str> {
    if !contains_ci(prelude, "@supports") || !contains_ci(prelude, "-webkit-") {
        return Cow::Borrowed(prelude);
    }
    Cow::Owned(
        // The property names first, held apart so the display value's
        // `-webkit-box` does not match inside them.
        prelude
            .replace("-webkit-line-clamp", "\u{0}C")
            .replace("-webkit-box-orient", "\u{0}O")
            .replace("-webkit-inline-box", "inline-block")
            .replace("-webkit-box", "flow-root")
            .replace("\u{0}C", &format!("--{CLAMP}"))
            .replace("\u{0}O", &format!("--{ORIENT}")),
    )
}

/// `css` with the three Gecko-only spellings rewritten into declarations
/// Stylo's servo build keeps (see the module docs). `css` is a style sheet,
/// or with `inline` an inline `style` declaration list. Borrowed unchanged
/// when it has no `-webkit-` in it.
pub(crate) fn rewrite(css: &str, inline: bool) -> Cow<'_, str> {
    if !contains_ci(css, "-webkit-") {
        return Cow::Borrowed(css);
    }
    let mut out = String::with_capacity(css.len() + 64);
    if inline {
        rewrite_list(css, &mut out);
        return Cow::Owned(out);
    }
    // A sheet: each run of text up to a `}` (or the end) after the last
    // `{` is a declaration list; up to a `{`, a prelude, copied as it is.
    let bytes = css.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 1);
            }
            b'{' => {
                out.push_str(&supports_prelude(&css[start..=i]));
                start = i + 1;
            }
            b'}' => {
                let list = &css[start..i];
                if contains_ci(list, "-webkit-") {
                    rewrite_list(list, &mut out);
                } else {
                    out.push_str(list);
                }
                out.push('}');
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let rest = &css[start.min(css.len())..];
    if contains_ci(rest, "-webkit-") {
        rewrite_list(rest, &mut out);
    } else {
        out.push_str(rest);
    }
    Cow::Owned(out)
}

/// What wins the cascade on the element itself among the declarations
/// `pick` recognises: Stylo's own order (`DeclarationIterator` in its
/// `cascade.rs`) — the rule nodes from the most to the least important, and
/// in a block the later declaration first, each at its node's importance.
fn own_winner<T>(
    cv: &style::properties::ComputedValues,
    guard: &style::shared_lock::SharedRwLockReadGuard,
    mut pick: impl FnMut(&style::properties::PropertyDeclaration) -> Option<T>,
) -> Option<T> {
    for node in cv.rules().self_and_ancestors() {
        let Some(source) = node.style_source() else {
            continue;
        };
        let important = node.importance().important();
        for (decl, importance) in source.read(guard).declaration_importance_iter().rev() {
            if importance.important() != important {
                continue;
            }
            if let Some(t) = pick(decl) {
                return Some(t);
            }
        }
    }
    None
}

/// A rewritten custom property's declared text, or its CSS-wide keyword.
enum Declared {
    Value(String),
    Keyword(style::properties::CSSWideKeyword),
}

fn custom(decl: &style::properties::PropertyDeclaration, name: &str) -> Option<Declared> {
    use style::properties::{CustomDeclarationValue, PropertyDeclaration};
    let PropertyDeclaration::Custom(c) = decl else {
        return None;
    };
    if &*c.name != name {
        return None;
    }
    Some(match &c.value {
        CustomDeclarationValue::Unparsed(v) => Declared::Value(v.css.trim().to_owned()),
        CustomDeclarationValue::Parsed(_) => return None,
        CustomDeclarationValue::CSSWideKeyword(k) => Declared::Keyword(*k),
    })
}

thread_local! {
    static NAMES: [style::Atom; 3] = [DISPLAY, CLAMP, ORIENT].map(style::Atom::from);
}

/// Whether the element or an ancestor declares any of the three: custom
/// properties inherit, so one declared on the element is in its computed
/// map. Most elements have none, and [`apply`] is skipped for them.
pub(crate) fn may_declare(cv: &style::properties::ComputedValues) -> bool {
    let props = cv.custom_properties();
    if props.inherited.is_empty() && props.non_inherited.is_empty() {
        return false;
    }
    NAMES.with(|names| {
        names.iter().any(|name| {
            props
                .get(
                    style::properties_and_values::rule::Descriptors::unregistered(),
                    name,
                )
                .is_some()
        })
    })
}

/// Resolve the three properties on `style`, just converted from `cv`;
/// `parent` is the parent element's style (for `inherit`).
pub(crate) fn apply(
    cv: &style::properties::ComputedValues,
    guard: &style::shared_lock::SharedRwLockReadGuard,
    parent: Option<&ComputedStyle>,
    style: &mut ComputedStyle,
) {
    use style::properties::CSSWideKeyword;
    let inherit = |k: &CSSWideKeyword| matches!(k, CSSWideKeyword::Inherit);
    let clamp = own_winner(cv, guard, |d| custom(d, CLAMP)).and_then(|d| match d {
        Declared::Value(v) => v.strip_prefix('+').unwrap_or(&v).parse::<u32>().ok(),
        Declared::Keyword(k) if inherit(&k) => parent.and_then(|p| p.webkit_line_clamp),
        Declared::Keyword(_) => None,
    });
    let vertical = own_winner(cv, guard, |d| custom(d, ORIENT)).is_some_and(|d| match d {
        Declared::Value(v) => {
            v.eq_ignore_ascii_case("vertical") || v.eq_ignore_ascii_case("block-axis")
        }
        Declared::Keyword(k) => inherit(&k) && parent.is_some_and(|p| p.webkit_box_vertical),
    });
    style.webkit_line_clamp = clamp;
    style.webkit_box_vertical = vertical;
    style.line_clamp = None;
    // The marker counts only where it beats every `display` declaration.
    let marker = own_winner(cv, guard, |d| {
        if matches!(
            d.id(),
            style::properties::PropertyDeclarationId::Longhand(
                style::properties::LonghandId::Display
            )
        ) {
            return Some(None);
        }
        custom(d, DISPLAY).map(|d| match d {
            Declared::Value(v) => Some(v.eq_ignore_ascii_case("-webkit-inline-box")),
            Declared::Keyword(_) => None,
        })
    })
    .flatten();
    let Some(inline) = marker else {
        return;
    };
    // `display: none` and the like were decided by Stylo; only a box Stylo
    // made of the rewrite is changed.
    if !matches!(
        style.display,
        DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Block
            | DisplayValue::InlineBlock
    ) {
        return;
    }
    style.webkit_box = true;
    // Chrome lays a legacy box that does not clamp out with the flex
    // algorithm, a run of inline content as one anonymous item; rinch has no
    // such item (#2581), and as a flex container here each inline child was
    // an item of its own (`Hello <b>world</b> and more` in a vertical one:
    // 72 tall, Chrome 24). Laid out as the block Stylo was given it matches
    // Chrome for inline content, and its text still draws no `text-overflow`
    // "…", as an anonymous flex item's does not (`ComputedStyle::webkit_box`).
    if let Some(n) = clamp.filter(|_| vertical) {
        style.display = if inline {
            DisplayValue::InlineBlock
        } else {
            DisplayValue::Block
        };
        style.line_clamp = Some(n);
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn a_sheet_without_webkit_is_borrowed() {
        assert!(matches!(
            rewrite("a { display: flex }", false),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn the_clamp_pattern_becomes_flow_root_and_custom_properties() {
        let got = rewrite(
            ".c { display: -webkit-box; -webkit-box-orient: vertical; -WEBKIT-line-clamp: 2 !important }",
            false,
        );
        assert_eq!(
            got,
            ".c { display: flow-root; --rinch-webkit-display: -webkit-box; \
             --rinch-webkit-box-orient: vertical; --rinch-webkit-line-clamp: 2 !important}"
        );
    }

    #[test]
    fn an_inline_box_becomes_inline_block_and_keeps_its_importance() {
        assert_eq!(
            rewrite("display:-webkit-inline-box !important", true),
            "display: inline-block !important; --rinch-webkit-display: -webkit-inline-box !important"
        );
    }

    #[test]
    fn an_invalid_clamp_is_left_for_stylo_to_drop() {
        assert_eq!(
            rewrite("a{-webkit-line-clamp: 0}b{-webkit-line-clamp: 1.5}", false),
            "a{-webkit-line-clamp: 0}b{-webkit-line-clamp: 1.5}"
        );
    }

    #[test]
    fn preludes_strings_and_comments_are_copied() {
        let css = "@media (min-width: 1px) { .a[title=\"x;y{\"] { /* display: -webkit-box */ \
                   content: \"display: -webkit-box\"; color: red } } .b{display:-webkit-box}";
        assert_eq!(
            rewrite(css, false),
            "@media (min-width: 1px) { .a[title=\"x;y{\"] { /* display: -webkit-box */ \
             content: \"display: -webkit-box\"; color: red } } .b{display: flow-root; --rinch-webkit-display: -webkit-box}"
        );
    }
}
