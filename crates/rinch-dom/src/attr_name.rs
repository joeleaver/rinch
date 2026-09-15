//! Attribute-name folding: HTML's names are ASCII case-insensitive, SVG's are
//! not (#688).
//!
//! A browser decides this at parse time, from the namespace the element was
//! created in. rinch carries no namespace — `stylo_impl.rs` hands Stylo the
//! XHTML namespace for every element — so the question is answered from the
//! **tag name** instead, by [`is_svg_content_tag`].
//!
//! ## Why the tag and not the parent
//!
//! The obvious spelling is a walk up to the nearest `<svg>`. It does not work,
//! because of *when* attributes are written. Both writers that build a tree set
//! every attribute on a fresh element and append it to its parent afterwards:
//!
//! - the `rsx!` codegen (`rinch-macros`' `dom_codegen/html.rs`) emits
//!   `create_element` → attributes → `append_child` for the children, so an
//!   element's own attributes land before it is attached to anything;
//! - `RinchDocument::create_node_from_parsed` — the `Element::Html` /
//!   `set_inner_html` path — does the same, recursively.
//!
//! So at the moment of the write the element has **no parent**, and a walk
//! would answer "not SVG" for every element of a freshly built `<svg>`,
//! silently lowercasing `viewBox`. The tag is the only thing available, and it
//! is enough.
//!
//! A *tag-local* `tag != "svg"` test would pass today, because the two
//! camelCase attributes rinch reads (`viewBox`, `preserveAspectRatio`, in
//! `paint/svg.rs`) both sit on the `<svg>` element itself. It would stop
//! passing at the first SVG feature that needed `gradientUnits`,
//! `stdDeviation`, `markerWidth` or `startOffset`, all of which live on
//! children. Hence a list.
//!
//! ## The two sibling predicates, deliberately not merged
//!
//! - `stylo_impl.rs`'s `TElement::is_svg_element` answers `tag() == "svg"`.
//!   That is Stylo's question, not this one, and its only consumer outside
//!   `#[cfg(feature = "gecko")]` code is a shadow-host `<use>` check rinch
//!   never reaches. Widening it would change style resolution for no gain
//!   here, so it is left alone.
//! - `rinch-web`'s `web_document.rs::is_svg_tag` picks `createElementNS` over
//!   `createElement`. The browser then folds attribute names itself, correctly
//!   and per namespace, which is why the web backend needs no fold of its own.
//!   That list includes `title` and excludes several of the ones below; the two
//!   lists answer different questions and neither is derived from the other.

use std::borrow::Cow;

/// Whether an element with this tag name is **SVG content**, and therefore has
/// case-**sensitive** attribute names.
///
/// Exact match, not case-insensitive: these are the canonical SVG spellings,
/// which is what `rsx!` validates against (`rinch-macros`' `SVG_TAGS`) and what
/// an HTML parser produces after its own name adjustment.
///
/// **Four SVG element names are deliberately absent** — `a`, `script`, `style`
/// and `title`. Each is also an HTML element name, rinch has no namespace to
/// tell the two apart, and the HTML reading is overwhelmingly the likely one
/// (`<a ID="x">` is real markup; an `<a>` inside an `<svg>` is rare).
///
/// For **three** of them the choice is free: SVG's `script`, `style` and
/// `title` carry no camelCase attribute, so folding their names changes no
/// spelling that matters. **`a` is not free**, and saying otherwise would be a
/// convenient falsehood: SVG's conditional-processing attributes
/// `requiredExtensions` and `systemLanguage` apply to `<a>` along with every
/// other container element, and inside an `<svg>` this folds them to
/// `requiredextensions` / `systemlanguage`. It costs nothing *today* — nothing
/// in the workspace reads either name, and `paint/svg.rs`'s child dispatch
/// handles `path`, `rect`, `circle`, `line`, `polyline` and `polygon` and drops
/// everything else (`a` included) through a `_ => {}` arm — but it is a real
/// deviation to weigh again if conditional processing ever arrives, not an
/// absence of one.
///
/// `foreignObject` **is** here — the element itself is SVG — but its HTML
/// descendants are not, and they fall out correctly without a special case,
/// because their own tags are HTML ones.
pub fn is_svg_content_tag(tag: &str) -> bool {
    SVG_CONTENT_TAGS.binary_search(&tag).is_ok()
}

/// The SVG element names, minus the four that collide with HTML — see
/// [`is_svg_content_tag`]. Sorted by ASCII order (uppercase sorts *before*
/// lowercase, so `feBlend` precedes `feColorMatrix`), which
/// `svg_content_tags_are_sorted` pins for the `binary_search`.
const SVG_CONTENT_TAGS: &[&str] = &[
    "animate",
    "animateMotion",
    "animateTransform",
    "circle",
    "clipPath",
    "defs",
    "desc",
    "discard",
    "ellipse",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "filter",
    "foreignObject",
    "g",
    "hatch",
    "hatchpath",
    "image",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "metadata",
    "mpath",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "set",
    "stop",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "tspan",
    "use",
    "view",
];

/// The name an attribute is stored under, for an element whose tag is `tag`.
///
/// HTML content folds to ASCII lowercase, matching a browser's parse-time
/// normalisation; SVG content keeps the author's spelling. `tag` is `None` only
/// for a node that is not an element, which cannot carry attributes anyway —
/// treated as HTML.
///
/// The **value** is never touched: `id="CamelId"` still matches `#CamelId`
/// case-sensitively outside quirks mode.
///
/// Borrows whenever it can, which is essentially always: a name already free of
/// ASCII uppercase is returned untouched without so much as a tag lookup, and
/// every name rinch itself writes is in that shape.
pub fn fold_attribute_name<'a>(tag: Option<&str>, name: &'a str) -> Cow<'a, str> {
    // The common case, and the cheapest test available: nothing to fold.
    if !name.bytes().any(|b| b.is_ascii_uppercase()) {
        return Cow::Borrowed(name);
    }
    if tag.is_some_and(is_svg_content_tag) {
        return Cow::Borrowed(name);
    }
    Cow::Owned(name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `binary_search` is only a search on a sorted slice, and the ordering
    /// that matters here is ASCII's, where `feBlend` < `feColorMatrix` because
    /// `B` < `C` and `linearGradient` sorts after `line` because a prefix comes
    /// first. A hand-maintained list is exactly where that goes wrong.
    #[test]
    fn svg_content_tags_are_sorted() {
        let mut sorted = SVG_CONTENT_TAGS.to_vec();
        sorted.sort_unstable();
        assert_eq!(SVG_CONTENT_TAGS, sorted.as_slice());
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(sorted, deduped, "no duplicates");
    }

    /// The four names left out, and the reason they are left out: each is an
    /// HTML element name too. This is a transcription pin — if one is ever
    /// added, this test is where the trade-off has to be re-argued.
    ///
    /// The trade-off is **not** uniform across the four, and `a` is the one
    /// that costs something: SVG's `requiredExtensions` / `systemLanguage`
    /// apply to it, so inside an `<svg>` those two names fold. Nothing reads
    /// them and the SVG painter never descends into an `<a>`, so it is inert
    /// rather than free — see [`is_svg_content_tag`].
    #[test]
    fn the_html_ambiguous_svg_names_are_absent() {
        for name in ["a", "script", "style", "title"] {
            assert!(
                !is_svg_content_tag(name),
                "{name} is an HTML element name as well; see is_svg_content_tag"
            );
        }
    }

    #[test]
    fn html_content_folds_and_svg_content_does_not() {
        assert_eq!(fold_attribute_name(Some("div"), "ID"), "id");
        assert_eq!(fold_attribute_name(Some("div"), "Data-Thing"), "data-thing");
        assert_eq!(fold_attribute_name(Some("my-widget"), "ID"), "id");
        assert_eq!(fold_attribute_name(None, "ID"), "id");

        assert_eq!(fold_attribute_name(Some("svg"), "viewBox"), "viewBox");
        assert_eq!(
            fold_attribute_name(Some("linearGradient"), "gradientUnits"),
            "gradientUnits"
        );
        assert_eq!(
            fold_attribute_name(Some("feGaussianBlur"), "stdDeviation"),
            "stdDeviation"
        );
    }

    /// A name with no uppercase is returned borrowed, whatever the tag — the
    /// fast path that keeps the fold off the cost of every ordinary write.
    ///
    /// Sampled **off** the fixed point on both axes: an all-lowercase name on
    /// an SVG tag would be borrowed by either arm, so the discriminating pair
    /// is an uppercase name on an HTML tag (owned) against the same name on an
    /// SVG tag (borrowed).
    #[test]
    fn only_a_folded_name_allocates() {
        assert!(matches!(
            fold_attribute_name(Some("div"), "class"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            fold_attribute_name(Some("svg"), "viewBox"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            fold_attribute_name(Some("div"), "CLASS"),
            Cow::Owned(_)
        ));
    }

    /// Only the ASCII range folds. `İ` (U+0130) lowercases to `i̇` in Unicode,
    /// which would turn a two-byte name into a three-byte one and is not what
    /// HTML asks for — attribute names fold in the ASCII range only.
    #[test]
    fn folding_is_ascii_only() {
        assert_eq!(
            fold_attribute_name(Some("div"), "data-\u{130}"),
            "data-\u{130}"
        );
        assert_eq!(
            fold_attribute_name(Some("div"), "DATA-\u{130}"),
            "data-\u{130}"
        );
    }
}
