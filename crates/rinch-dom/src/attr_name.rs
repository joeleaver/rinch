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

/// The subset of [`SVG_CONTENT_TAGS`] whose canonical spelling is not already
/// all-lowercase, keyed by that lowercase spelling and sorted by it for
/// `binary_search_by_key` (#739). This is HTML's own "adjust SVG tag names"
/// step: a browser's tokenizer lowercases every tag name it reads, *then*
/// the tree builder restores a fixed set of names back to their mixed-case
/// spelling — so the restoration is keyed on the lowercase form regardless of
/// how the author originally cased it. `svg_tag_name_adjustments_match_the_canonical_list`
/// below is what keeps this derived rather than hand-drifted from the list
/// above.
const SVG_TAG_NAME_ADJUSTMENTS: &[(&str, &str)] = &[
    ("animatemotion", "animateMotion"),
    ("animatetransform", "animateTransform"),
    ("clippath", "clipPath"),
    ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"),
    ("fecomponenttransfer", "feComponentTransfer"),
    ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"),
    ("fediffuselighting", "feDiffuseLighting"),
    ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"),
    ("fedropshadow", "feDropShadow"),
    ("feflood", "feFlood"),
    ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"),
    ("fefuncg", "feFuncG"),
    ("fefuncr", "feFuncR"),
    ("fegaussianblur", "feGaussianBlur"),
    ("feimage", "feImage"),
    ("femerge", "feMerge"),
    ("femergenode", "feMergeNode"),
    ("femorphology", "feMorphology"),
    ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"),
    ("fespecularlighting", "feSpecularLighting"),
    ("fespotlight", "feSpotLight"),
    ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"),
    ("foreignobject", "foreignObject"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("textpath", "textPath"),
];

fn adjust_svg_tag_name(lower: &str) -> Option<&'static str> {
    SVG_TAG_NAME_ADJUSTMENTS
        .binary_search_by_key(&lower, |&(key, _)| key)
        .ok()
        .map(|idx| SVG_TAG_NAME_ADJUSTMENTS[idx].1)
}

/// The name an element's tag is stored under (#739).
///
/// HTML tag names are ASCII case-insensitive and a parser lowercases them;
/// SVG's are not, and keep the author's canonical spelling. Deciding which
/// rule applies uses the same tag-only test #688 uses for attributes
/// ([`is_svg_content_tag`]), checked against the tag **as given** first — an
/// already-correctly-spelled SVG tag (`svg`, `linearGradient`, `path`, …) is
/// returned untouched, which is also what lets a plain lowercase tag skip
/// every other check. Anything else is ASCII-lowercased, then restored to its
/// canonical SVG spelling if [`adjust_svg_tag_name`] has one for it — the
/// "adjust SVG tag names" step above, applied to a tag that was not already
/// exactly right (`<LINEARGRADIENT>`, `<lineargradient>`, `<ClipPath>`, …).
///
/// **Deliberately not case-insensitive against the whole SVG list.** Most of
/// [`SVG_CONTENT_TAGS`] is already lowercase (`text`, `g`, `use`, `path`, …),
/// so a case-insensitive match of e.g. `<TEXT>` against that list would read
/// it as SVG's `<text>` — wrong for a plain uppercase HTML tag with no SVG
/// context to justify it, and `TEXT` is exactly as ambiguous as the lowercase
/// `text` #688 already accepts as HTML by default. Restricting the
/// case-insensitive step to [`SVG_TAG_NAME_ADJUSTMENTS`] — only the names
/// whose canonical spelling actually differs from its lowercase form — avoids
/// that: `TEXT` lowercases to `text`, finds no adjustment entry (there is
/// none; canonical already equals lowercase) and stays the plain HTML tag
/// `text`, exactly as `<TEXT>` would with no SVG list in the picture at all.
///
/// Must run **before** any attribute is written on the element — attributes
/// are folded by [`fold_attribute_name`] against the tag the node already
/// carries, so a tag normalised after the fact folds every one of its
/// attributes by the wrong rule (`create_element` is the one call site that
/// needs to apply this first).
pub fn fold_tag_name(tag: &str) -> Cow<'_, str> {
    if is_svg_content_tag(tag) {
        return Cow::Borrowed(tag);
    }
    if !tag.bytes().any(|b| b.is_ascii_uppercase()) {
        return match adjust_svg_tag_name(tag) {
            Some(canonical) => Cow::Borrowed(canonical),
            None => Cow::Borrowed(tag),
        };
    }
    let lower = tag.to_ascii_lowercase();
    match adjust_svg_tag_name(&lower) {
        Some(canonical) => Cow::Borrowed(canonical),
        None => Cow::Owned(lower),
    }
}

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

    /// Every weird-cased SVG name in [`SVG_CONTENT_TAGS`] (canonical spelling
    /// differs from its own lowercase form) has exactly one entry in
    /// [`SVG_TAG_NAME_ADJUSTMENTS`], and nothing else does — a hand-transcribed
    /// second list is exactly where this drifts.
    #[test]
    fn svg_tag_name_adjustments_match_the_canonical_list() {
        let expected: std::collections::BTreeSet<&str> = SVG_CONTENT_TAGS
            .iter()
            .copied()
            .filter(|tag| tag.to_ascii_lowercase() != *tag)
            .collect();
        let actual: std::collections::BTreeSet<&str> = SVG_TAG_NAME_ADJUSTMENTS
            .iter()
            .map(|&(_, canonical)| canonical)
            .collect();
        assert_eq!(actual, expected);
        for &(lower, canonical) in SVG_TAG_NAME_ADJUSTMENTS {
            assert_eq!(canonical.to_ascii_lowercase(), lower);
        }
        let mut sorted = SVG_TAG_NAME_ADJUSTMENTS.to_vec();
        sorted.sort_unstable_by_key(|&(key, _)| key);
        assert_eq!(SVG_TAG_NAME_ADJUSTMENTS, sorted.as_slice());
    }

    /// `<DIV>` and `<Button>`: an HTML tag folds to lowercase whatever case it
    /// was written in, which is the whole point of #739 — `div { … }` then
    /// matches both `<div>` and `<DIV>`.
    #[test]
    fn html_tags_fold_to_lowercase() {
        assert_eq!(fold_tag_name("DIV"), "div");
        assert_eq!(fold_tag_name("Button"), "button");
        assert_eq!(fold_tag_name("div"), "div");
    }

    /// An already-correctly-cased SVG tag is untouched and borrowed — the
    /// fast path, and what keeps `viewBox`/`gradientUnits`/etc. folding
    /// decisions (keyed on the tag the node carries) unaffected by #739.
    #[test]
    fn correctly_cased_svg_tags_are_untouched_and_borrowed() {
        for tag in ["svg", "linearGradient", "path", "foreignObject"] {
            assert!(matches!(fold_tag_name(tag), Cow::Borrowed(_)));
            assert_eq!(fold_tag_name(tag), tag);
        }
    }

    /// A mis-cased SVG tag — written in all caps, or in plain lowercase — is
    /// restored to its canonical mixed-case spelling, matching a browser's
    /// own "adjust SVG tag names" step, which runs off the *lowercased* name
    /// regardless of how the author originally cased it.
    #[test]
    fn miscased_svg_tags_are_adjusted_to_canonical() {
        assert_eq!(fold_tag_name("LINEARGRADIENT"), "linearGradient");
        assert_eq!(fold_tag_name("lineargradient"), "linearGradient");
        assert_eq!(fold_tag_name("ClipPath"), "clipPath");
        assert_eq!(fold_tag_name("FOREIGNOBJECT"), "foreignObject");
    }

    /// The one case the #738 review flagged by name: `<TEXT>` must stay a
    /// plain HTML tag, not become SVG's `<text>`, because `text`'s canonical
    /// spelling is already all-lowercase and so carries no adjustment entry.
    #[test]
    fn uppercase_text_stays_html_not_svg() {
        assert_eq!(fold_tag_name("TEXT"), "text");
        assert!(matches!(fold_tag_name("TEXT"), Cow::Owned(_)));
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
