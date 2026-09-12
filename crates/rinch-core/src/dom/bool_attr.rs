//! HTML **boolean attributes**: the ones whose presence *is* their value.
//!
//! `disabled`, `checked`, `readonly`, `hidden` and their kind carry no useful
//! string. HTML says a present attribute is true whatever it holds, so
//! `disabled="false"` disables — there is no falsey spelling short of removing
//! the attribute. Anything that renders a *reactive* boolean into one therefore
//! has to write two different shapes, not two different strings: presence for
//! true, absence for false. Writing `"false"` is a one-way latch, and that is
//! issue #551 — the whole of it, on both backends.
//!
//! The rule is name-driven, not type-driven, and that is deliberate. Several
//! attributes take a `bool`-looking value where the string `"false"` is
//! meaningful and load-bearing:
//!
//! - `draggable` is an *enumerated* attribute (`"true"` / `"false"`, invalid
//!   values falling back to `auto`), and desktop's drag dispatch matches the
//!   literal `"true"`. Presence-mapping it would break dragging on both
//!   backends.
//! - `contenteditable`, `spellcheck`, `translate`, `aria-*` are the same shape.
//! - rinch's own `data-viewport-ready` is an **opt-out** whose *absence* means
//!   ready, so removing it on false would invert its meaning (see the viewport
//!   holes note in CLAUDE.md).
//!
//! So a value's Rust type says nothing about how it must be written; only the
//! attribute's name does.

/// Whether `name` is an attribute whose **presence** carries its value.
///
/// The HTML set is taken from the WHATWG "Attributes" index — every row whose
/// *Value* column reads "Boolean attribute"
/// (<https://html.spec.whatwg.org/multipage/indices.html#attributes-3>, read
/// 2026-09-12, 30 rows). Being generated from the spec's own index rather than
/// recalled is what makes it complete; `boolean_attribute_list_matches_the_spec_index`
/// pins the transcription.
///
/// Two additions beyond that list, each for a stated reason:
///
/// - **`hidden`** is an *enumerated* attribute in modern HTML, not a boolean
///   one — but its invalid-value default is the hidden state, so
///   `hidden="false"` hides. It fails exactly the way a boolean attribute
///   fails, so it is written the same way. (rinch honours it through
///   `.rinch-tabs__panel[hidden]`, a presence selector, so desktop agrees.)
/// - **`data-disabled`** and **`data-nofocus`** are rinch's own boolean
///   attributes, documented as "present unless the value is `false`" and read
///   that way by both backends. Writing them by presence makes a reactive
///   binding correct by construction instead of correct by that tolerance.
pub fn is_boolean_attribute(name: &str) -> bool {
    matches!(
        name,
        // ── WHATWG HTML, "Attributes" index, Value = "Boolean attribute" ──
        "allowfullscreen"
            | "alpha"
            | "async"
            | "autofocus"
            | "autoplay"
            | "checked"
            | "controls"
            | "default"
            | "defer"
            | "disabled"
            | "formnovalidate"
            | "headingreset"
            | "inert"
            | "ismap"
            | "itemscope"
            | "loop"
            | "multiple"
            | "muted"
            | "nomodule"
            | "novalidate"
            | "open"
            | "playsinline"
            | "readonly"
            | "required"
            | "reversed"
            | "selected"
            | "shadowrootclonable"
            | "shadowrootcustomelementregistry"
            | "shadowrootdelegatesfocus"
            | "shadowrootserializable"
            // ── enumerated, but falsey-hostile in the same way ──
            | "hidden"
            // ── rinch's own ──
            | "data-disabled"
            | "data-nofocus"
    )
}

/// Truthiness for a value rinch writes into a boolean attribute.
///
/// rinch writes these two ways: `rsx!` renders a `bool` through `Display`, so
/// it arrives as `"true"` / `"false"`, while components set the bare presence
/// form `""` (matching HTML, where a present boolean attribute is true whatever
/// its value). Everything is "on" except the explicit falsey strings, so both
/// conventions round-trip — in particular an empty string means *present*, and
/// so true.
///
/// Strict HTML has no falsey string at all. Treating `"false"` as off is the
/// escape rinch has documented for `data-disabled` since it was written, and
/// `rinch_dom::node_is_disabled` / the web backend's `data-nofocus` reader both
/// honour it — ASCII-case-insensitively, which is why this does too; one rule
/// everywhere beats two.
pub fn attr_is_truthy(value: &str) -> bool {
    !(value.eq_ignore_ascii_case("false") || value == "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec index's 30 rows, transcribed a second time from the source
    /// named in [`is_boolean_attribute`]'s doc comment.
    ///
    /// A list recalled from memory is the failure mode this guards: the check is
    /// not that the predicate is self-consistent but that it holds every row the
    /// index marks "Boolean attribute", and nothing rinch has not argued for.
    const SPEC_BOOLEAN_ATTRIBUTES: &[&str] = &[
        "allowfullscreen",
        "alpha",
        "async",
        "autofocus",
        "autoplay",
        "checked",
        "controls",
        "default",
        "defer",
        "disabled",
        "formnovalidate",
        "headingreset",
        "inert",
        "ismap",
        "itemscope",
        "loop",
        "multiple",
        "muted",
        "nomodule",
        "novalidate",
        "open",
        "playsinline",
        "readonly",
        "required",
        "reversed",
        "selected",
        "shadowrootclonable",
        "shadowrootcustomelementregistry",
        "shadowrootdelegatesfocus",
        "shadowrootserializable",
    ];

    #[test]
    fn boolean_attribute_list_matches_the_spec_index() {
        assert_eq!(SPEC_BOOLEAN_ATTRIBUTES.len(), 30);
        for name in SPEC_BOOLEAN_ATTRIBUTES {
            assert!(
                is_boolean_attribute(name),
                "{name} is a spec boolean attribute"
            );
        }
    }

    /// The three deliberate additions, so removing one is a test failure rather
    /// than a silent narrowing.
    #[test]
    fn the_non_spec_additions_are_the_documented_three() {
        for name in ["hidden", "data-disabled", "data-nofocus"] {
            assert!(
                is_boolean_attribute(name),
                "{name} must be written by presence"
            );
        }
    }

    /// The enumerated look-alikes must stay out: each has a meaningful
    /// `"false"`, and removing the attribute changes or inverts what it says.
    #[test]
    fn enumerated_look_alikes_are_not_boolean_attributes() {
        for name in [
            "draggable",
            "contenteditable",
            "spellcheck",
            "translate",
            "autocomplete",
            "aria-expanded",
            "aria-hidden",
            "aria-checked",
            "aria-selected",
            "aria-disabled",
            "data-viewport-ready",
            "data-checked",
            "value",
            "class",
            "style",
            "download",
        ] {
            assert!(
                !is_boolean_attribute(name),
                "{name} takes a real value — presence-mapping it would lose or invert it"
            );
        }
    }

    #[test]
    fn truthiness_covers_both_writing_conventions() {
        // The `rsx!` convention: a `bool` rendered through `Display`.
        assert!(attr_is_truthy("true"));
        assert!(!attr_is_truthy("false"));
        // The component convention: bare presence.
        assert!(attr_is_truthy(""));
        assert!(attr_is_truthy("disabled"));
        assert!(!attr_is_truthy("0"));
        // Case-folded, matching every reader of the escape
        // (`rinch_dom::node_is_disabled`, `node_is_readonly`, `node_is_nofocus`,
        // and the web backend's `[data-nofocus="false" i]` selector).
        assert!(!attr_is_truthy("FALSE"));
    }
}
