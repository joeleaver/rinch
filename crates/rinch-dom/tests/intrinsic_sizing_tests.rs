//! #626 / #691 — `max-content`, `min-content`, `fit-content` and `stretch`.
//!
//! All four parse (#626). Since #691 they **lay out** on `width`, `height` and
//! `flex-basis`: `DimensionValue::to_taffy` hands Taffy 0.14 its own keyword,
//! and rinch resolves the keyword itself on the two kinds of box Taffy cannot
//! see it on — an atomic inline, which rinch computes as a Taffy *root*
//! (`ifc.rs`, `resolve_root_width_keyword`), and an out-of-flow box whose
//! containing block is the viewport (`out_of_flow.rs`, `stretch` only).
//! `intrinsic_sizing_twin_tests.rs` is the Chrome 153 twin for all of that.
//!
//! This file keeps what the twin does not cover:
//!
//! - the **computed-style** fixtures — the keyword survives conversion, the two
//!   function/alias spellings land where they should;
//! - the **`min-*`/`max-*`** half, which still lays out as `auto`: Taffy 0.14
//!   stores `min_size`/`max_size` as `LengthPercentageAuto`, which has no
//!   keyword at all (`min_and_max_keywords_still_lay_out_exactly_like_auto`
//!   is the deviation record, with Chrome's numbers in the table below);
//! - two used-size gates that must treat a keyword as "no declared length"
//!   (the IFC wrap tolerance and the `<textarea rows>` floor);
//! - the oracle table for the eleven #626 contexts, Chrome 150, now asserted
//!   for every keyword row rinch implements (`width_and_height_rows_match_the_oracle`).
//!
//! # All four spellings really do parse
//!
//! Worth stating because it is not obvious from the source and it was got wrong
//! once while writing this file. Stylo guards `stretch`,
//! `-webkit-fill-available` and `fit-content(<length-percentage>)` on
//! `static_prefs::pref!("layout.css.stretch-size-keyword.enabled")` and two
//! siblings, which *looks* like the runtime preference store rinch pokes in
//! `RinchDocument::new` (`stylo_config::set_bool`, which answers `false` for
//! every key nobody set). It is not: `static_prefs` is the separate
//! `stylo_static_prefs` crate, and its `pref!` is a **compile-time macro** that
//! hard-codes those three keys — and `layout.css.fit-content-function.enabled` —
//! to `true`, falling through to `false` only for a key it does not list.
//!
//! So nothing is gated here, and `the_keyword_survives_into_the_computed_style`
//! is what fails if a stylo bump ever flips one of those macro arms.
//!
//! # The oracle
//!
//! Chrome 150, headless, standards mode, `file://`. The box under test holds
//! **one block child** declared `width: 300px; height: 20px`. Block children
//! stack, so the box holds no line box at all and every number here is
//! declaration-derived — nothing in this file pins the local font set.
//!
//! Containing block 800px wide (150px where the table says so), 300px tall
//! where a definite height is needed. `cb` below is that containing block.
//!
//! | context | property | `auto` | `max-content` | `min-content` | `fit-content` | `stretch` |
//! |---|---|---|---|---|---|---|
//! | block in block, cb 800 | `width` | 800 | **300** | **300** | **300** | 800 |
//! | block in block, cb 150 | `width` | 150 | **300** | **300** | **300** | 150 |
//! | block in block, cb 800 | `max-width` | 800 | **300** | **300** | **300** | 800 |
//! | block in block, cb 150 | `min-width` | 150 | **300** | **300** | **300** | 150 |
//! | block in block, cb h 300 | `height` | 20 | 20 | 20 | 20 | **300** |
//! | block in block, cb h 300 | `min-height` | 20 | 20 | 20 | 20 | **300** |
//! | block in block, cb h 300 | `max-height` | 20 | 20 | 20 | 20 | 20 |
//! | `inline-block`, cb 800 | `width` | 300 | 300 | 300 | 300 | **800** |
//! | flex-row item, cb 800 | `width` | 300 | 300 | 300 | 300 | **800** |
//! | flex-column item, cb 800 | `width` | 800 | **300** | **300** | **300** | 800 |
//! | grid item, cb 800 | `width` | 800 | **300** | **300** | **300** | 800 |
//!
//! Bold is where Chrome and `auto` disagree. The two halves are **mirror
//! images**: the three content keywords agree with `auto` wherever `auto` is
//! content-sized, and `stretch` wherever `auto` fills. Since #691 rinch gives
//! the bold numbers on every `width`/`height` row; on the `min-*`/`max-*` rows
//! it still gives `auto`'s, and that is what the diagnostic `from_stylo` prints
//! for those properties is about.
//!
//! `min-content` and `max-content` coincide in this table because a single
//! declared block offers no break opportunity. A second Chrome run with two
//! 100px `inline-block`s in a `font-size: 0` parent — so the space between them
//! has no width, and the numbers stay font-free — separates them: `max-content`
//! 200, `min-content` 100, `fit-content` 200 in an 800px containing block and
//! 150 in a 150px one. That shape is the `two` content of
//! `intrinsic_sizing_twin_tests.rs`.
//!
//! # The fixed point stepped off on purpose
//!
//! The child is **300px wide inside an 800px containing block** rather than
//! wider than it or equal to it. At 800 the shrink-wrapped and filled answers
//! would coincide and every fixture below would pass under any implementation;
//! at more than 800 `max-content` would overflow and the block-in-block cases
//! would read as a clamping bug instead. 300 versus 800 is a 500px gap that no
//! rounding or font can close.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{DimensionValue, IntrinsicSize};

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// One block child of declared size — see the module doc on why it is a block.
const CHILD: &str = "width: 300px; height: 20px";

/// The four keywords plus `auto`, which is the control every one of them is
/// compared against.
const KEYWORDS: [&str; 5] = [
    "auto",
    "max-content",
    "min-content",
    "fit-content",
    "stretch",
];

/// `<cb><t {prop}: {keyword}><child 300x20></t></cb>` laid out at 800x600.
/// Returns the document and the box under test.
fn build(cb_style: &str, t_style: &str, prop: &str, keyword: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cb = doc.create_element("div");
    doc.set_attribute(cb, "style", cb_style);
    doc.append_child(body, cb);
    let t = doc.create_element("div");
    doc.set_attribute(t, "style", &format!("{t_style}; {prop}: {keyword}"));
    doc.append_child(cb, t);
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", CHILD);
    doc.append_child(t, c);
    doc.resolve_layout(VW, VH);
    (doc, t)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let l = &doc.tree.get(id.0).unwrap().layout;
    (l.width, l.height)
}

/// The computed `width` of a box that declares `width: <value>`.
fn computed_width(value: &str) -> DimensionValue {
    let (doc, t) = build("width: 800px", "", "width", value);
    doc.tree.get(t.0).unwrap().computed_style.width
}

/// Every context in the oracle table above, as (label, containing-block style,
/// box style, property).
const CASES: &[(&str, &str, &str, &str)] = &[
    ("block in block, cb 800", "width: 800px", "", "width"),
    ("block in block, cb 150", "width: 150px", "", "width"),
    ("block in block, cb 800", "width: 800px", "", "max-width"),
    ("block in block, cb 150", "width: 150px", "", "min-width"),
    (
        "block in block, cb h 300",
        "width: 800px; height: 300px",
        "",
        "height",
    ),
    (
        "block in block, cb h 300",
        "width: 800px; height: 300px",
        "",
        "min-height",
    ),
    (
        "block in block, cb h 300",
        "width: 800px; height: 300px",
        "",
        "max-height",
    ),
    (
        "inline-block, cb 800",
        "width: 800px",
        "display: inline-block",
        "width",
    ),
    (
        "flex-row item, cb 800",
        "display: flex; width: 800px",
        "",
        "width",
    ),
    (
        "flex-column item, cb 800",
        "display: flex; flex-direction: column; width: 800px; height: 300px",
        "",
        "width",
    ),
    (
        "grid item, cb 800",
        "display: grid; grid-template-columns: auto; width: 800px",
        "",
        "width",
    ),
];

// ---------------------------------------------------------------------------
// What this PR established
// ---------------------------------------------------------------------------

/// What Taffy 0.14 does with a keyword, and what rinch hands it (#691).
///
/// Taffy 0.12 panicked on this tree (`MaybeResolve for Dimension`'s
/// `_ => unreachable!()`, reached through `unsafe from_raw` — there was no safe
/// constructor). 0.14 constructs the keyword safely and shrink-wraps a non-root
/// block child to its 300px content where `auto` fills the 800px container —
/// Chrome's answer in the oracle table's first row.
///
/// The second half pins the **root** limit that `resolve_root_width_keyword`
/// exists for: the same keyword on the box Taffy computes as a root is
/// ignored, and the root fills its definite available space as `auto` does.
///
/// The last half is rinch's mapping: `to_taffy` (`size`, `flex_basis`) hands
/// over the keyword, `to_taffy_lpa` (`min_size`, `max_size`) cannot — Taffy's
/// `LengthPercentageAuto` has no keyword at all.
#[test]
fn taffy_lays_out_a_keyword_on_a_child_not_a_root_and_rinch_hands_it_over() {
    use taffy::prelude::*;
    use taffy::{Dimension, Style};

    fn child_width(width: Dimension) -> f32 {
        let mut t: TaffyTree<()> = TaffyTree::new();
        let content = t
            .new_leaf(Style {
                size: Size {
                    width: Dimension::length(300.0),
                    height: Dimension::length(20.0),
                },
                ..Default::default()
            })
            .unwrap();
        let child = t
            .new_with_children(
                Style {
                    display: Display::Block,
                    size: Size {
                        width,
                        height: Dimension::auto(),
                    },
                    ..Default::default()
                },
                &[content],
            )
            .unwrap();
        let root = t
            .new_with_children(
                Style {
                    display: Display::Block,
                    size: Size {
                        width: Dimension::length(800.0),
                        height: Dimension::length(600.0),
                    },
                    ..Default::default()
                },
                &[child],
            )
            .unwrap();
        t.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(600.0),
            },
        )
        .unwrap();
        t.layout(child).unwrap().size.width
    }

    assert_eq!(child_width(Dimension::auto()), 800.0, "control: auto fills");
    assert_eq!(
        child_width(Dimension::max_content()),
        300.0,
        "Taffy 0.14 shrink-wraps a max-content block child, as Chrome does"
    );
    assert_eq!(child_width(Dimension::min_content()), 300.0);
    assert_eq!(child_width(Dimension::fit_content()), 300.0);

    // The same keyword on a root: ignored, so a definite available width
    // stretches the block root exactly as `auto` would.
    let root_width = |width: Dimension| {
        let mut t: TaffyTree<()> = TaffyTree::new();
        let content = t
            .new_leaf(Style {
                size: Size {
                    width: Dimension::length(300.0),
                    height: Dimension::length(20.0),
                },
                ..Default::default()
            })
            .unwrap();
        let root = t
            .new_with_children(
                Style {
                    display: Display::Block,
                    size: Size {
                        width,
                        height: Dimension::auto(),
                    },
                    ..Default::default()
                },
                &[content],
            )
            .unwrap();
        t.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::MaxContent,
            },
        )
        .unwrap();
        t.layout(root).unwrap().size.width
    };
    assert_eq!(root_width(Dimension::auto()), 800.0);
    assert_eq!(
        root_width(Dimension::max_content()),
        800.0,
        "Taffy ignores a root's own size keyword — why rinch resolves it for an atomic inline"
    );

    for k in [
        IntrinsicSize::MaxContent,
        IntrinsicSize::MinContent,
        IntrinsicSize::FitContent,
        IntrinsicSize::Stretch,
    ] {
        let v = DimensionValue::Intrinsic(k);
        assert_eq!(
            v.to_taffy(),
            k.to_taffy(),
            "{k:?}: rinch hands Taffy the keyword itself"
        );
        assert!(v.to_taffy().is_sizing_keyword(), "{k:?}");
        assert!(
            v.to_taffy_lpa().is_auto(),
            "{k:?}: a min/max size has no keyword representation in Taffy 0.14"
        );
    }
}

/// The declaration now survives style conversion. **Fails against `main`**,
/// where every arm answered `DimensionValue::Auto` and nothing downstream —
/// including the MCP's `get_computed_styles` — could tell a declared
/// `max-content` from an undeclared width.
#[test]
fn the_keyword_survives_into_the_computed_style() {
    for (css, expected) in [
        ("max-content", IntrinsicSize::MaxContent),
        ("min-content", IntrinsicSize::MinContent),
        ("fit-content", IntrinsicSize::FitContent),
        ("stretch", IntrinsicSize::Stretch),
    ] {
        assert_eq!(
            computed_width(css).intrinsic(),
            Some(expected),
            "width: {css} should read back as {expected:?}"
        );
    }
}

/// The two spellings that are *not* the bare keywords, and land on opposite
/// sides of the keyword arm.
///
/// `-webkit-fill-available` is a separate stylo variant that **means**
/// `stretch` — Chrome 150 answers `stretch` for `getComputedStyle(el).minWidth`
/// when the declaration is `min-width: -webkit-fill-available` — so it folds
/// into that keyword rather than falling through the silent `_ =>` catch-all it
/// used to take.
///
/// `fit-content(<length-percentage>)` is `clamp(min-content, <lp>, max-content)`,
/// which is a different value from the bare `fit-content` keyword, so it does
/// **not** borrow that variant. It stays `Auto` — but it is now reported rather
/// than dropped in silence, which is the half of #626 that applies to it.
#[test]
fn the_two_function_and_alias_spellings_land_where_they_should() {
    assert_eq!(
        computed_width("-webkit-fill-available").intrinsic(),
        Some(IntrinsicSize::Stretch),
        "the prefixed alias is the `stretch` keyword"
    );

    let f = computed_width("fit-content(200px)");
    assert_eq!(
        f.intrinsic(),
        None,
        "`fit-content()` is not the `fit-content` keyword and must not borrow its variant"
    );
    assert!(matches!(f, DimensionValue::Auto), "got {f:?}");
}

/// A garbled value is not a keyword and must not be reported as one — the
/// declaration is dropped by the CSS parser long before `from_stylo` sees it.
#[test]
fn an_invalid_value_is_still_just_auto() {
    assert!(matches!(computed_width("wibble"), DimensionValue::Auto));
    assert_eq!(computed_width("wibble").intrinsic(), None);
}

/// `is_auto` asks what the author wrote, `is_auto_or_keyword` whether a length
/// was declared at all, and `fills_between_insets` whether a positioned box
/// with both insets fills them. A keyword answers each differently.
#[test]
fn the_three_auto_questions_differ_for_a_keyword() {
    let k = DimensionValue::Intrinsic(IntrinsicSize::MaxContent);
    assert!(!k.is_auto(), "the author did not write `auto`");
    assert!(k.is_auto_or_keyword(), "nor a length");
    assert!(
        !k.fills_between_insets(),
        "a content keyword keeps its measure"
    );

    let s = DimensionValue::Intrinsic(IntrinsicSize::Stretch);
    assert!(s.fills_between_insets(), "`stretch` fills, as `auto` does");

    assert!(DimensionValue::Auto.is_auto());
    assert!(DimensionValue::Auto.is_auto_or_keyword());
    assert!(DimensionValue::Auto.fills_between_insets());
    for v in [
        DimensionValue::Length(10.0),
        DimensionValue::Percent(0.5),
        DimensionValue::Calc { px: 1.0, pct: 0.5 },
    ] {
        assert!(!v.is_auto_or_keyword(), "{v:?}");
        assert!(!v.fills_between_insets(), "{v:?}");
    }
}

/// The one place where telling the two apart would silently change layout.
///
/// `<textarea rows=N>` gets an intrinsic `min-height` of N lines because its
/// value lives in an attribute and gives it no content height. That is gated on
/// the author having declared no height length — and a height of `max-content`
/// declares none, so the gate must still open. Reading `is_auto()` there
/// instead collapses the control.
///
/// `line-height` and `font-size` are declared and padding and border zeroed, so
/// 4 rows is 80px by declaration rather than by font measurement.
#[test]
fn a_textarea_with_an_intrinsic_height_still_gets_its_rows() {
    fn height_of(decl: &str) -> f32 {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let ta = doc.create_element("textarea");
        doc.set_attribute(ta, "rows", "4");
        doc.set_attribute(
            ta,
            "style",
            &format!("line-height: 20px; font-size: 16px; padding: 0; border: 0; {decl}"),
        );
        doc.append_child(body, ta);
        doc.resolve_layout(VW, VH);
        doc.tree.get(ta.0).unwrap().layout.height
    }
    let control = height_of("height: auto");
    assert_eq!(control, 80.0, "4 rows at a declared 20px line-height");
    for k in ["max-content", "min-content", "fit-content", "stretch"] {
        assert_eq!(
            height_of(&format!("height: {k}")),
            control,
            "`height: {k}` declares no length, so the rows minimum must still apply"
        );
    }
}

// ---------------------------------------------------------------------------
// The oracle table, asserted
// ---------------------------------------------------------------------------

/// Chrome 150's numbers from the module doc's table, per case, for
/// `KEYWORDS` in order: (width, height) of the box under test.
const ORACLE: [[(f32, f32); 5]; 11] = [
    // block in block, cb 800, width
    [
        (800.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (800.0, 20.0),
    ],
    // block in block, cb 150, width
    [
        (150.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (150.0, 20.0),
    ],
    // max-width, cb 800
    [
        (800.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (800.0, 20.0),
    ],
    // min-width, cb 150
    [
        (150.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (150.0, 20.0),
    ],
    // height, cb h 300
    [
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 300.0),
    ],
    // min-height, cb h 300
    [
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 300.0),
    ],
    // max-height, cb h 300
    [
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
        (800.0, 20.0),
    ],
    // inline-block, cb 800, width
    [
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (800.0, 20.0),
    ],
    // flex-row item, cb 800, width
    [
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (800.0, 20.0),
    ],
    // flex-column item, cb 800, width
    [
        (800.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (800.0, 20.0),
    ],
    // grid item, cb 800, width
    [
        (800.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (300.0, 20.0),
        (800.0, 20.0),
    ],
];

/// Every `width`/`height` row of the table, every keyword, Chrome's number.
/// The 15 bold cells among these 35 were `auto`'s number before #691.
#[test]
fn width_and_height_rows_match_the_oracle() {
    let mut checked = 0;
    for ((label, cb, ts, prop), want) in CASES.iter().zip(ORACLE) {
        if prop.starts_with("min-") || prop.starts_with("max-") {
            continue;
        }
        for (k, want) in KEYWORDS.iter().zip(want) {
            let (doc, t) = build(cb, ts, prop, k);
            assert_eq!(size(&doc, t), want, "{label}, `{prop}: {k}`");
            checked += 1;
        }
    }
    assert_eq!(checked, 7 * 5, "seven width/height rows, five values each");
}

/// The half #691 could not implement through Taffy: on `min-*`/`max-*` every
/// keyword still lays out **exactly like `auto`**, because Taffy 0.14 stores
/// `min_size`/`max_size` as `LengthPercentageAuto`, which has no keyword.
/// A deviation record: three of these four rows disagree with Chrome (bold in
/// the module doc's table), and this is the test that flips when they are
/// implemented — a rinch-side measurement pass, #1275.
#[test]
fn min_and_max_keywords_still_lay_out_exactly_like_auto() {
    let mut same = 0;
    for (label, cb, ts, prop) in CASES {
        if !(prop.starts_with("min-") || prop.starts_with("max-")) {
            continue;
        }
        let (doc, t) = build(cb, ts, prop, "auto");
        let control = size(&doc, t);
        for k in &KEYWORDS[1..] {
            let (doc, t) = build(cb, ts, prop, k);
            assert_eq!(
                size(&doc, t),
                control,
                "{label}, `{prop}: {k}`: a min/max keyword reaches Taffy as `auto`. If this \
                 now differs, the min/max half is being implemented — flip this against the \
                 oracle table."
            );
            same += 1;
        }
    }
    assert_eq!(same, 4 * 4, "four min/max rows, four keywords each");
}

/// rinch's `auto` agrees with Chrome's `auto` in **all eleven** contexts.
///
/// This is what makes the oracle table readable as "the keyword column minus
/// the `auto` column": the deviation is the substitution and nothing else. It
/// also rules out the mutant where a fixture passes because rinch's `auto` is
/// wrong in a way that happens to look like the keyword.
///
/// Every number is declaration-derived — a containing block's declared width,
/// a declared child height, or a declared containing-block height — so this
/// pins no font.
#[test]
fn rinch_auto_agrees_with_chrome_auto_everywhere_in_the_table() {
    // (index into CASES, Chrome's `auto` width, Chrome's `auto` height)
    let expected: [(f32, f32); 11] = [
        (800.0, 20.0), // block in block, cb 800, width
        (150.0, 20.0), // block in block, cb 150, width
        (800.0, 20.0), // max-width, cb 800
        (150.0, 20.0), // min-width, cb 150
        (800.0, 20.0), // height, cb h 300
        (800.0, 20.0), // min-height, cb h 300
        (800.0, 20.0), // max-height, cb h 300
        (300.0, 20.0), // inline-block, cb 800 — shrink-to-fit
        (300.0, 20.0), // flex-row item, cb 800 — content-sized
        (800.0, 20.0), // flex-column item, cb 800 — cross-axis stretch
        (800.0, 20.0), // grid item, cb 800 — stretched into an auto track
    ];
    assert_eq!(CASES.len(), expected.len());
    for ((label, cb, ts, prop), want) in CASES.iter().zip(expected) {
        let (doc, t) = build(cb, ts, prop, "auto");
        assert_eq!(size(&doc, t), want, "{label}, `{prop}: auto`");
    }
}

/// The one place the substitution really did change layout, and the reason
/// nothing else in this file could see it.
///
/// `ifc.rs` gives a **content-sized** box's text-layout pass 1px of slack
/// (#120): such a box's width is its text's max-content width measured with no
/// wrap and then floored to an integer pixel, so the pass that re-lays the text
/// must be allowed up to 1px more than the content box or it re-wraps at a space
/// inside a box that was sized for one line — the box unchanged, the glyphs on
/// two.
///
/// That gate asks "was this box content-sized", which is a **used size**
/// question, so an intrinsic keyword must answer yes. It read the *specified*
/// value (`matches!(cs.width, DimensionValue::Auto)`) until this PR, which was
/// harmless while `max-content` became `Auto` at conversion and became a live
/// regression the moment the keyword survived. Found in review of #690, not by
/// this file — every other case here holds a **declared block child**, so no
/// inline formatting context is built and the tolerance is never consulted.
///
/// Both assertions are rinch against itself, so neither pins a font: `auto` and
/// `max-content` shrink-wrap to the same box, so they must be handed the same
/// `max_width`; and that `max_width` must be the content box plus exactly the
/// 1px tolerance. Against the `is_auto()` spelling the two differ by 1.0.
#[test]
fn a_content_sized_box_keeps_its_one_pixel_wrap_tolerance() {
    /// `(text_layout.max_width, the IFC root's content-box width)`.
    fn measure(width_decl: &str) -> (f32, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let cb = doc.create_element("div");
        doc.set_attribute(cb, "style", "width: 800px");
        doc.append_child(body, cb);
        let t = doc.create_element("span");
        doc.set_attribute(
            t,
            "style",
            &format!(
                "display: inline-block; line-height: 20px; font-size: 16px;                  padding: 0; border: 0; width: {width_decl}"
            ),
        );
        doc.append_child(cb, t);
        let txt = doc.create_text("the quick brown fox jumps over the lazy dog");
        doc.append_child(t, txt);
        doc.resolve_layout(VW, VH);

        // Which node owns the Parley layout depends on the anonymous-box
        // machinery (#592 made an inline-block a block container), so find it
        // rather than assume — but only inside the box under test. The
        // containing block is an IFC root too (the inline-block is an atomic
        // inline on its line), and its own width is declared, so it is
        // *correctly* given no tolerance and would mask the one being measured.
        let mut stack = vec![t.0];
        let mut roots: Vec<(usize, f32, f32)> = Vec::new();
        while let Some(id) = stack.pop() {
            let n = &doc.tree.nodes[id];
            if let Some(tl) = n.text_layout.as_ref() {
                roots.push((id, tl.max_width, n.layout.width));
            }
            stack.extend(n.children.iter().copied());
        }
        assert_eq!(
            roots.len(),
            1,
            "expected exactly one IFC text root inside the inline-block, got {roots:?}"
        );
        let (_, max_width, box_width) = roots[0];
        (max_width, box_width)
    }

    let (auto_mw, auto_box) = measure("auto");
    let (kw_mw, kw_box) = measure("max-content");

    assert!(
        (auto_box - kw_box).abs() < 0.01,
        "control: both spellings shrink-wrap to the same box ({auto_box} vs {kw_box})"
    );
    assert!(
        (auto_mw - kw_mw).abs() < 0.01,
        "`width: max-content` is content-sized exactly as `auto` is, so it must get the \
         same wrap tolerance: auto gave max_width {auto_mw}, max-content gave {kw_mw}"
    );
    // And state the tolerance itself, so this cannot pass by both sides being 0.
    assert!(
        (kw_mw - kw_box - 1.0).abs() < 0.01,
        "the content-sized tolerance is 1px: max_width {kw_mw} over a {kw_box} content box"
    );
}

/// A witness for one sentence in the guide, which would otherwise be prose
/// nothing checks.
///
/// `docs/src/guide/theming.md`'s "when `auto` happens to be the right answer"
/// table lists the boxes where `auto` already shrink-wraps. A browser would put
/// **floats** in that row; rinch must not, because it implements no CSS float
/// at all — a floated box fills its containing block where Chrome 150 gives it
/// 300 for the same content. So a float is not a way to reach shrink-to-fit
/// here, and `display: inline-block` is.
///
/// Not a #626 behaviour. It guards the guidance #626's docs give.
#[test]
fn rinch_has_no_css_float_so_a_float_does_not_shrink_wrap() {
    let (doc, t) = build("width: 800px", "float: left", "width", "auto");
    assert_eq!(
        size(&doc, t).0,
        800.0,
        "rinch implements no float, so a floated box fills; Chrome 150 gives 300"
    );
}

/// The contrast that shows this is Taffy's shape rather than a rinch oversight:
/// an intrinsic **grid track** works.
///
/// `from_stylo/grid.rs` maps `TrackBreadth::MaxContent` to
/// `MaxTrackSizingFunction::MAX_CONTENT`, and Taffy's track sizing does read
/// those tags — so the same keyword that is discarded on a box's `width` sizes a
/// column. Chrome 150 with the same 300px block child in an 800px grid: an
/// `auto` track gives the item 800, `max-content` and `min-content` give it 300.
///
/// The `auto` control is what makes this discriminating: at 800 versus 300 a
/// grid that ignored the track keyword could not pass.
#[test]
fn an_intrinsic_grid_track_does_work() {
    fn item_width(tracks: &str) -> f32 {
        let (doc, t) = build(
            &format!("display: grid; grid-template-columns: {tracks}; width: 800px"),
            "",
            "width",
            "auto",
        );
        size(&doc, t).0
    }
    assert_eq!(
        item_width("auto"),
        800.0,
        "control: an auto track stretches"
    );
    assert_eq!(item_width("max-content"), 300.0);
    assert_eq!(item_width("min-content"), 300.0);
}

/// Prints the whole measured table. Not an assertion — it is how the oracle in
/// the module doc gets re-derived on the rinch side after a layout change:
/// `cargo test -p rinch-dom --test intrinsic_sizing_tests -- --nocapture rinch_table`.
#[test]
fn rinch_table() {
    println!("context | property | value | width | height");
    for (label, cb, ts, prop) in CASES {
        for k in KEYWORDS {
            let (doc, t) = build(cb, ts, prop, k);
            let (w, h) = size(&doc, t);
            println!("{label} | {prop} | {k} | {w:.2} | {h:.2}");
        }
    }
}
