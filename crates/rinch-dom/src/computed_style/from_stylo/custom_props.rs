//! Values rinch carries through custom properties, and reading them back.
//!
//! Properties Stylo's servo build does not parse or compute as rinch needs
//! (`hyphens`, `word-break`, `text-indent`'s keywords, the text-paint
//! properties, the scrollbar colour and width) are rewritten into custom
//! properties before the cascade (`crate::webkit_box`). This reads each
//! back off a cascaded style ([`custom_property`]), converts the text paint
//! and `text-indent`, and gives a value a `var()` left invalid its
//! parent's, as Chrome does (#2820, #2870).

use super::*;

thread_local! {
    /// The two custom-property names, interned once per thread rather than on
    /// every node: `from_stylo` runs for each dirty node on each style pass,
    /// and `Atom::from(&str)` is an atom-table lookup, not free.
    ///
    /// No leading `--`: Stylo's `custom_properties::Name` is the atom *without*
    /// the prefix.
    pub(super) static SCROLLBAR_COLOR: style::Atom = style::Atom::from("rinch-scrollbar-color");
    pub(super) static SCROLLBAR_WIDTH: style::Atom = style::Atom::from("rinch-scrollbar-width");
    /// `hyphens`, rewritten into a custom property (#2621).
    pub(super) static HYPHENS: style::Atom = style::Atom::from(crate::webkit_box::HYPHENS);
    /// Every `word-break`, mirrored into a custom property (#2628).
    pub(super) static WORD_BREAK: style::Atom = style::Atom::from(crate::webkit_box::WORD_BREAK);
    /// `text-indent`'s keywords, mirrored into a custom property (#2628).
    static TEXT_INDENT: style::Atom = style::Atom::from(crate::webkit_box::TEXT_INDENT);
    /// The text-paint properties, rewritten into custom properties (#2719).
    static TEXT_FILL_COLOR: style::Atom = style::Atom::from(crate::webkit_box::TEXT_FILL_COLOR);
    static TEXT_STROKE_WIDTH: style::Atom =
        style::Atom::from(crate::webkit_box::TEXT_STROKE_WIDTH);
    static TEXT_STROKE_COLOR: style::Atom =
        style::Atom::from(crate::webkit_box::TEXT_STROKE_COLOR);
    static PAINT_ORDER: style::Atom = style::Atom::from(crate::webkit_box::PAINT_ORDER);
}

/// `-webkit-text-fill-color`, `-webkit-text-stroke-*` and `paint-order`
/// (#2719), from the custom properties they were rewritten into. Each
/// inherits as a custom property does, as its token string: `currentcolor`
/// and an `em` width are resolved here, against this element's `color` and
/// `font-size` (Chrome resolves an `em` width where it is declared and
/// inherits the length). [`TextPaintValue::PLAIN`] on a page that never
/// wrote one, with no look-up.
///
/// A value that is no value of its property — what a `var()` substituted
/// into it can leave (#2820) — is read as the initial value here; the
/// cascade then gives such an element its parent's value instead, as Chrome
/// does for a declaration invalid at computed-value time
/// ([`ComputedStyle::inherit_invalid_text_paint`]).
pub(super) fn text_paint_from_stylo(
    cv: &ComputedValues,
    color: Option<peniko::Color>,
    font_size: f32,
) -> TextPaintValue {
    if !crate::webkit_box::text_paint_seen() {
        return TextPaintValue::PLAIN;
    }
    let props = cv.custom_properties();
    if props.inherited.is_empty() && props.non_inherited.is_empty() {
        return TextPaintValue::PLAIN;
    }
    text_paint_with(cv, color, font_size, |_| false, &TextPaintValue::PLAIN)
}

/// The text-paint values, each read off its custom property: a computed
/// value that parses is taken; one that does not, or a property `inherits`
/// says the element takes from its parent though it has no value, is
/// `parent`'s.
fn text_paint_with(
    cv: &ComputedValues,
    color: Option<peniko::Color>,
    font_size: f32,
    inherits: impl Fn(&str) -> bool,
    parent: &TextPaintValue,
) -> TextPaintValue {
    use crate::webkit_box::{
        PAINT_ORDER as ORDER, TEXT_FILL_COLOR as FILL, TEXT_STROKE_COLOR as STROKE,
        TEXT_STROKE_WIDTH as WIDTH,
    };
    let current = color.unwrap_or(peniko::Color::BLACK);
    // A substituted `initial` (`var(--x, initial)`, #2870) is the initial
    // value; any other CSS-wide keyword parses as no value, and inherits.
    let initial = |v: &str| v.eq_ignore_ascii_case("initial");
    // `Some(Some(v))` parsed, `Some(None)` is no value of the property, `None`
    // no computed value at all.
    let fill = match custom_property(cv, &TEXT_FILL_COLOR, |v| {
        if v.eq_ignore_ascii_case("currentcolor") || initial(v) {
            Some(None)
        } else {
            crate::layout::parse_color_with_current(v, current).map(Some)
        }
    }) {
        Some(Some(fill)) => fill,
        Some(None) => parent.fill,
        None if inherits(FILL) => parent.fill,
        None => None,
    };
    // Each `-webkit-text-stroke-*` value is the longhand's, or its half of a
    // shorthand that held a `var()` (#2820, `webkit_box::stroke_half`).
    let stroke_width = match custom_property(cv, &TEXT_STROKE_WIDTH, |v| {
        if initial(v) {
            return Some(0.0);
        }
        crate::webkit_box::stroke_half(v, true)
            .and_then(|v| crate::webkit_box::parse_stroke_width(&v, font_size))
    }) {
        Some(Some(w)) => w,
        Some(None) => parent.stroke_width,
        None if inherits(WIDTH) => parent.stroke_width,
        None => 0.0,
    };
    // The colour, and whether it is `currentcolor`: the parent's
    // `currentcolor` is resolved against this element's `color` (#2870).
    let inherited = || match parent.stroke_color {
        Some(c) if !parent.stroke_currentcolor => (c, false),
        _ => (current, true),
    };
    let stroke = (stroke_width > 0.0).then(|| {
        match custom_property(cv, &TEXT_STROKE_COLOR, |v| {
            if initial(v) {
                return Some((current, true));
            }
            crate::webkit_box::stroke_half(v, false).and_then(|v| {
                if v.eq_ignore_ascii_case("currentcolor") {
                    Some((current, true))
                } else {
                    crate::layout::parse_color_with_current(&v, current).map(|c| (c, false))
                }
            })
        }) {
            Some(Some(c)) => c,
            Some(None) => inherited(),
            None if inherits(STROKE) => inherited(),
            None => (current, true),
        }
    });
    let stroke_first = stroke_width > 0.0
        && match custom_property(cv, &PAINT_ORDER, |v| {
            if initial(v) {
                Some(false)
            } else {
                crate::paint::parse_paint_order(v)
            }
        }) {
            Some(Some(first)) => first,
            Some(None) => parent.stroke_first,
            None if inherits(ORDER) => parent.stroke_first,
            None => false,
        };
    TextPaintValue {
        fill,
        stroke_width,
        stroke_color: stroke.map(|(c, _)| c),
        stroke_currentcolor: stroke.is_some_and(|(_, current)| current),
        stroke_first,
    }
}

impl ComputedStyle {
    /// Give the text-paint values a `var()` left invalid their parent's
    /// (#2820), as Chrome does for a declaration invalid at computed-value
    /// time (the four properties inherit): a computed value that is no value
    /// of its property, or none at all where the element's own winning
    /// declaration is not `initial` (a substitution that failed leaves the
    /// custom property unset). `self` was just converted from `cv`; `parent`
    /// is the parent element's style. Only a parent whose text is painted
    /// otherwise than plainly can make a difference, so nothing is read
    /// for any other.
    pub(crate) fn inherit_invalid_text_paint(
        &mut self,
        cv: &ComputedValues,
        guard: &style::shared_lock::SharedRwLockReadGuard,
        parent: &ComputedStyle,
    ) {
        if !crate::webkit_box::text_paint_seen() || parent.text_paint.is_plain() {
            return;
        }
        self.text_paint = text_paint_with(
            cv,
            self.color,
            self.font_size,
            |name| crate::webkit_box::own_declaration_inherits(cv, guard, name),
            &parent.text_paint,
        );
    }
}

/// `text-indent`: Stylo's length, and the `hanging` / `each-line` keywords
/// from `--rinch-text-indent` (#2628). A declaration holding a `var()` is
/// mirrored behind `webkit_box::VAR_MARKER` and split here, substituted
/// (#2870): where it has a keyword, Stylo's servo build could not parse it
/// (and gave the parent's indent), so the length is read here too. One that
/// substituted no `text-indent` value is left as Stylo made it, for
/// [`ComputedStyle::inherit_invalid_text`].
pub(super) fn text_indent_with(
    cv: &ComputedValues,
    ti: &style::properties::longhands::text_indent::computed_value::T,
    font_size: f32,
) -> TextIndentValue {
    let mut out = text_indent_from_stylo(ti);
    custom_property(cv, &TEXT_INDENT, |v| {
        let has = |v: &str, k: &str| v.split_whitespace().any(|w| w.eq_ignore_ascii_case(k));
        let Some(sub) = v.strip_prefix(crate::webkit_box::VAR_MARKER) else {
            out.hanging |= has(v, "hanging");
            out.each_line |= has(v, "each-line");
            return;
        };
        let Some((length, keywords)) = crate::webkit_box::split_text_indent(sub.trim()) else {
            return;
        };
        if keywords.eq_ignore_ascii_case("initial") {
            out = TextIndentValue::default();
            return;
        }
        if keywords.eq_ignore_ascii_case("none") {
            return;
        }
        if let Some((px, pct)) = crate::webkit_box::parse_length_percentage(&length, font_size) {
            out.length = match (px, pct) {
                (px, 0.0) => LengthPercentageValue::Length(px),
                (0.0, pct) => LengthPercentageValue::Percent(pct),
                (px, pct) => LengthPercentageValue::Calc { px, pct },
            };
        }
        out.hanging = has(&keywords, "hanging");
        out.each_line = has(&keywords, "each-line");
    });
    out
}

impl ComputedStyle {
    /// Give `hyphens`, `word-break` and `text-indent` values a `var()` left
    /// invalid their parent's (#2870), as Chrome does for a declaration
    /// invalid at computed-value time (the three inherit): the custom
    /// property each is carried in has no value of its property, or none at
    /// all where the element's own winning declaration does not reset it (a
    /// substitution that failed leaves the custom property unset). A
    /// `text-indent` substituted on an ancestor, with no declaration of the
    /// element's own, is the parent's too: its `em` is the ancestor's.
    /// `self` was just converted from `cv`; `parent` is the parent element's
    /// style. Only a value that differs from the parent's is looked at.
    pub(crate) fn inherit_invalid_text(
        &mut self,
        cv: &ComputedValues,
        guard: &style::shared_lock::SharedRwLockReadGuard,
        parent: &ComputedStyle,
    ) {
        use crate::webkit_box::{own_declaration_inherits, own_declares};
        if !crate::webkit_box::var_rewrite_seen() {
            return;
        }
        let one_of =
            |v: &str, keywords: &[&str]| keywords.iter().any(|k| v.eq_ignore_ascii_case(k));
        if self.hyphens_none != parent.hyphens_none
            && !custom_property(cv, &HYPHENS, |v| {
                one_of(v, &["none", "manual", "auto", "initial"])
            })
            .unwrap_or(false)
            && own_declaration_inherits(cv, guard, crate::webkit_box::HYPHENS)
        {
            self.hyphens_none = parent.hyphens_none;
        }
        if (self.word_break != parent.word_break
            || self.word_break_break_word != parent.word_break_break_word)
            && !custom_property(cv, &WORD_BREAK, |v| {
                one_of(
                    v,
                    &["normal", "break-all", "keep-all", "break-word", "initial"],
                )
            })
            .unwrap_or(false)
            && own_declaration_inherits(cv, guard, crate::webkit_box::WORD_BREAK)
        {
            self.word_break = parent.word_break;
            if parent.word_break_break_word {
                self.overflow_wrap = OverflowWrapValue::Anywhere;
            }
            self.word_break_break_word = parent.word_break_break_word;
        }
        if self.text_indent != parent.text_indent {
            use crate::webkit_box::TEXT_INDENT as NAME;
            // `Some(true)` a `var()` substituted, `Some(false)` one that
            // substituted no value, `None` no `var()`.
            let substituted = custom_property(cv, &TEXT_INDENT, |v| {
                v.strip_prefix(crate::webkit_box::VAR_MARKER).map(|sub| {
                    crate::webkit_box::split_text_indent(sub.trim()).is_some_and(|(l, k)| {
                        k.eq_ignore_ascii_case("initial")
                            || (!crate::webkit_box::is_wide_keyword(&k)
                                && crate::webkit_box::parse_length_percentage(&l, 16.0).is_some())
                    })
                })
            });
            let inherits = match substituted {
                Some(Some(true)) => !own_declares(cv, guard, NAME),
                Some(None) => false,
                Some(Some(false)) | None => own_declaration_inherits(cv, guard, NAME),
            };
            if inherits {
                self.text_indent = parent.text_indent;
            }
        }
    }
}

/// Read one custom property's post-`var()` token string off a cascaded style.
///
/// Unregistered custom properties are declared to inherit
/// (`PropertyRegistrationData::unregistered()`), and each element's map is
/// seeded from its parent's, so a declaration on `:root` is readable here from
/// any descendant — which is what makes a single app-wide
/// `--rinch-scrollbar-color` work.
///
/// The value is always the universal (untyped) form for an unregistered
/// property, but `to_variable_value` covers the registered case too rather than
/// relying on that.
pub(super) fn custom_property<R>(
    cv: &ComputedValues,
    key: &'static std::thread::LocalKey<style::Atom>,
    f: impl FnOnce(&str) -> R,
) -> Option<R> {
    key.with(|name| {
        cv.custom_properties()
            .get(
                style::properties_and_values::rule::Descriptors::unregistered(),
                name,
            )
            .map(|v| f(v.to_variable_value().css.trim()))
    })
}
