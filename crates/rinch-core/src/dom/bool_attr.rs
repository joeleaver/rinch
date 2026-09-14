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
/// - **`data-disabled`**, **`data-nofocus`** and **`data-trap-focus`** are
///   rinch's own boolean attributes, documented as "present unless the value is
///   `false`" — desktop reads all three that way, and the web reads
///   `data-nofocus` and `data-trap-focus` that way (it has no `data-disabled`
///   reader at all). Writing them by presence makes a reactive binding correct
///   by construction instead of correct by that tolerance.
///
///   `data-trap-focus` (issue #474) is the one that most needs it: it is
///   written from an overlay's *open* state, so it is the reactive case by
///   construction, and the failure it avoids is not cosmetic. A closed overlay
///   left carrying `data-trap-focus="false"` would be read as a live trap by
///   anything that tests presence, and Tab would circle inside an invisible
///   dialog for the rest of the session.
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
            | "data-trap-focus"
    )
}

/// Truthiness for a value **being written into** a boolean attribute.
///
/// This is the *writer's* rule, and it is the only thing it is for: deciding
/// which of the two shapes — presence or absence — a value asks for. The two
/// production callers are [`super::NodeHandle::write_attribute`] and, in the web
/// backend, `sync_reflected_property`'s arm for `indeterminate` — a property-only
/// IDL flag with no content attribute, so a string is all it ever has.
///
/// It is **not** what [`super::NodeHandle::set_attribute`] does. That is the
/// literal primitive on both backends: it writes the string it is given, and for
/// a boolean attribute the result is *presence*, whatever the string says. The
/// web backend used to route `checked` / `selected` through here instead, so the
/// same call unchecked a box on web and checked it on desktop; issue #622 made
/// that arm literal.
///
/// rinch writes these two ways: `rsx!` renders a `bool` through `Display`, so
/// it arrives as `"true"` / `"false"`, while components set the bare presence
/// form `""` (matching HTML, where a present boolean attribute is true whatever
/// its value). Everything is "on" except the explicit falsey strings, so both
/// conventions round-trip — in particular an empty string means *present*, and
/// so true.
///
/// **It is not a reader's rule.** Strict HTML has no falsey string at all: a
/// present `disabled` / `readonly` disables whatever it holds, `"false"`
/// included, measured in Chrome 150 both as the IDL property and as a
/// `:disabled` / `:read-only` match. Desktop's readers say the same since issue
/// #612 — see [`data_attr_is_on`] for the one family that keeps an escape, and
/// note that this function is *wider* than that escape anyway (it also treats
/// `"0"` as off, which no reader does).
pub fn attr_is_truthy(value: &str) -> bool {
    !(value.eq_ignore_ascii_case("false") || value == "0")
}

/// Whether one of **rinch's own** `data-` boolean attributes is on.
///
/// `data-disabled`, `data-nofocus` and `data-trap-focus` are rinch inventions,
/// not HTML, and rinch gives them an escape HTML has no equivalent of: present
/// means on *unless* the value is the literal `false`, ASCII-case-insensitively.
/// It is a rinch convention rather than a desktop quirk, and the latter two are
/// what show that: desktop reads them through this function and the web through
/// `event_delegation.rs`'s `[data-nofocus]:not([data-nofocus="false" i])` and
/// `[data-trap-focus]:not([data-trap-focus="false" i])`. (`data-disabled` has no
/// web reader, so there is nothing on that side to agree or disagree with.)
///
/// The plain HTML `disabled` / `readonly` deliberately do **not** go through
/// here: they are read by presence alone, the way a browser reads them (issue
/// #612). Spelling the two rules as two functions is what stops the next reader
/// from picking the wrong one by copying its neighbour, which is how the
/// divergence #612 closed came to exist.
///
/// Note the deliberate narrowness against [`attr_is_truthy`]: `"0"` is **on**
/// here, because the web selector matches only `"false"` and one rule across
/// backends beats a tidier one on either.
pub fn data_attr_is_on(value: &str) -> bool {
    !value.eq_ignore_ascii_case("false")
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

    /// The four deliberate additions, so removing one is a test failure rather
    /// than a silent narrowing.
    #[test]
    fn the_non_spec_additions_are_the_documented_four() {
        for name in ["hidden", "data-disabled", "data-nofocus", "data-trap-focus"] {
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
        // Case-folded, like the `data-` escape beside it and the web backend's
        // `[data-nofocus="false" i]` selector.
        assert!(!attr_is_truthy("FALSE"));
    }

    /// The rinch-only escape, and the one value where it deliberately parts
    /// company with the writer's rule above.
    #[test]
    fn the_data_escape_is_false_only() {
        assert!(data_attr_is_on(""), "presence is on");
        assert!(data_attr_is_on("true"));
        assert!(!data_attr_is_on("false"));
        assert!(!data_attr_is_on("FALSE"), "ASCII-case-insensitive");
        // Narrower than `attr_is_truthy` on purpose: the web selector matches
        // only `"false"`, so `"0"` must stay on for the two backends to agree.
        assert!(data_attr_is_on("0"));
        assert!(!attr_is_truthy("0"));
    }
}
