//! The set of names `rsx!` treats as markup rather than as a component.
//!
//! `rsx!` decides what an element name means from its *case*: PascalCase is a
//! component, anything else is a tag. That leaves lowercase component functions
//! — which `#[component]` supports and documents alongside PascalCase ones —
//! with no way to be recognised, so `todo_input {}` used to become an empty
//! `<todo_input>` element and the function was never called (issue #528).
//!
//! This list is what makes the difference sayable. A lowercase name in it is
//! markup; a lowercase name outside it is a mistake worth a compile error,
//! because nothing else in the pipeline can tell the two apart — the DOM will
//! happily create an element of any name, style it with the UA sheet's
//! defaults, and lay it out at zero height.
//!
//! It is deliberately generous: every HTML element including the obsolete ones,
//! plus the SVG elements rinch paints. Erring wide costs a missed diagnostic on
//! a tag nobody uses; erring narrow breaks a build that was correct. Hyphenated
//! custom elements (`<my-widget>`) need no entry — an rsx element name parses as
//! a Rust `Ident`, which cannot contain `-`, so they are unrepresentable anyway.

/// Every HTML element name, including the obsolete ones.
///
/// Sorted: [`is_known_tag`] binary-searches it, and a misplaced entry would
/// make every tag after it unfindable — a valid element turned into a compile
/// error. The test below is what holds that.
const HTML_TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "basefont",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "marquee",
    "menu",
    "meta",
    "meter",
    "nav",
    "noframes",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "param",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// The SVG elements rinch paints, plus the structural and gradient elements an
/// icon set emits.
///
/// A separate slice rather than more entries in [`HTML_TAGS`] for two reasons:
/// the grouping is worth keeping, and SVG's camelCase names (`clipPath`,
/// `linearGradient`) do not interleave with the HTML names under the ASCII
/// ordering `binary_search` needs. Each slice is sorted on its own.
const SVG_TAGS: &[&str] = &[
    "circle",
    "clipPath",
    "defs",
    "ellipse",
    "g",
    "line",
    "linearGradient",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "svg",
    "symbol",
    "text",
    "textPath",
    "tspan",
    "use",
];

/// Whether `rsx!` should treat `name` as markup.
pub fn is_known_tag(name: &str) -> bool {
    HTML_TAGS.binary_search(&name).is_ok() || SVG_TAGS.binary_search(&name).is_ok()
}

/// The known tag closest to `name`, if one is close enough to be worth
/// suggesting — for the ordinary typo (`dvi`, `spam`) as distinct from the
/// lowercase-component case that motivated this module.
///
/// "Close enough" is an edit distance of 1 for short names and 2 from five
/// characters up, which keeps `todo_input` from being reported as a misspelt
/// `output` while still catching a slipped keystroke.
pub fn closest_tag(name: &str) -> Option<&'static str> {
    let budget = if name.len() >= 5 { 2 } else { 1 };
    HTML_TAGS
        .iter()
        .chain(SVG_TAGS)
        .map(|tag| (*tag, edit_distance(name, tag)))
        .filter(|(_, d)| *d <= budget)
        .min_by_key(|(_, d)| *d)
        .map(|(tag, _)| tag)
}

/// Optimal string alignment distance — Levenshtein plus **transposition**.
///
/// Transposition has to count as one edit, not two, because it is the typo
/// people actually make: `dvi` for `div` is a single slipped keystroke, and
/// under plain Levenshtein it scores 2, which puts it outside the budget for a
/// name that short and suggests nothing at all.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // Three rows: the transposition case needs the row before last.
    let mut prev2: Vec<usize> = vec![0; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];

    for i in 0..a.len() {
        cur[0] = i + 1;
        for j in 0..b.len() {
            let cost = usize::from(a[i] != b[j]);
            let mut best = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            if i > 0 && j > 0 && a[i] == b[j - 1] && a[i - 1] == b[j] {
                best = best.min(prev2[j - 1] + 1);
            }
            cur[j + 1] = best;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `binary_search` is only valid on a sorted slice, and both lists are
    /// hand-maintained. An entry added in the wrong place does not fail
    /// loudly — it makes every name after it in that slice unfindable, so a
    /// perfectly valid `<video>` becomes "unknown element". This is the only
    /// thing standing between that and a very confusing afternoon.
    #[test]
    fn both_tag_lists_are_sorted_for_binary_search() {
        for (label, list) in [("HTML_TAGS", HTML_TAGS), ("SVG_TAGS", SVG_TAGS)] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            assert_eq!(
                list,
                sorted.as_slice(),
                "{label} must be sorted or `is_known_tag` will miss valid tags"
            );
        }
    }

    #[test]
    fn every_tag_the_repository_uses_is_known() {
        for tag in [
            "div", "span", "button", "p", "style", "img", "h1", "h2", "em", "strong", "video",
            "select", "input", "br", "body", "svg", "path", "textarea", "table", "li", "ul", "a",
            "code", "pre", "option",
        ] {
            assert!(is_known_tag(tag), "`{tag}` must be recognised as markup");
        }
    }

    #[test]
    fn a_lowercase_component_name_is_not_a_tag() {
        for name in ["todo_input", "filter_buttons", "todo_list", "my_widget"] {
            assert!(!is_known_tag(name), "`{name}` must not be taken for markup");
        }
    }

    /// A snake_case component must not be reported as a misspelt tag — the
    /// suggestion would send the reader looking for a typo that is not there,
    /// away from the actual fix.
    #[test]
    fn a_snake_case_component_gets_no_tag_suggestion() {
        for name in ["todo_input", "filter_buttons", "todo_list"] {
            assert_eq!(closest_tag(name), None, "`{name}` should suggest nothing");
        }
    }

    /// But an ordinary typo still gets one.
    #[test]
    fn a_misspelt_tag_suggests_the_real_one() {
        assert_eq!(closest_tag("dvi"), Some("div"));
        assert_eq!(closest_tag("spam"), Some("span"));
        assert_eq!(closest_tag("buton"), Some("button"));
    }
}
