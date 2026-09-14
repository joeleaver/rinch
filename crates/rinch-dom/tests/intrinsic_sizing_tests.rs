//! #626 — `max-content`, `min-content`, `fit-content` and `stretch`.
//!
//! All four parse, and none of them lays out: each reaches Taffy as `auto`.
//! This file is the record of what that costs and of why it is not a one-line
//! mapping, so it is **two** kinds of test at once and they must not be confused
//! with each other:
//!
//! - `taffy_0_12_cannot_carry_an_intrinsic_keyword_in_a_dimension` and the
//!   computed-style fixtures assert behaviour this PR **established**. Against
//!   `main` this file does not compile at all — the API is new — so what was
//!   actually run is the mutant that keeps the API and puts the keyword arms
//!   back to `DimensionValue::Auto`. Two of the twelve fail there, exactly
//!   `the_keyword_survives_into_the_computed_style` and
//!   `the_two_function_and_alias_spellings_land_where_they_should`.
//! - `every_intrinsic_keyword_lays_out_exactly_like_auto` and the two headline
//!   fixtures under it are **deviation records**. They pass on `main` too,
//!   because this PR does not change layout. They exist so the day someone
//!   implements the keywords they have to delete or flip them deliberately,
//!   with Chrome's number sitting right there to flip them to.
//!
//! # Why `auto`, and why that is not a missing match arm
//!
//! `taffy::Dimension` is a newtype over `CompactLength`. That type *does* carry
//! `MIN_CONTENT_TAG`, `MAX_CONTENT_TAG`, `FIT_CONTENT_PX_TAG` and
//! `FIT_CONTENT_PERCENT_TAG` — the issue's premise that "Taffy has `Dimension`
//! variants for these" comes from reading that list — but nothing reads them on
//! a box's size:
//!
//! - The helper traits that construct them (`TaffyMinContent`,
//!   `TaffyMaxContent`, `TaffyFitContent`) are implemented for `AvailableSpace`,
//!   for `MinTrackSizingFunction` / `MaxTrackSizingFunction` /
//!   `TrackSizingFunction` (grid), and for `CompactLength` itself.
//!   **Not for `Dimension`**, which implements only `TaffyAuto`, `FromLength`
//!   and `FromPercent` (taffy-0.12.2, `src/style/dimension.rs:233`).
//! - `impl MaybeResolve for Dimension` (`src/util/resolve.rs:57`) matches
//!   `AUTO`/`LENGTH`/`PERCENT`/calc and ends `_ => unreachable!()`, so a
//!   `size`/`min_size`/`max_size` holding one **panics** during layout. The
//!   probe below constructs exactly that, through the `unsafe from_raw` escape
//!   hatch, and catches the panic.
//! - Every consumer of those tags under `src/compute/` is under
//!   `src/compute/grid/` (`track_sizing.rs` and `types/grid_track.rs`).
//!
//! The vendored `crates/stylo-taffy/src/convert.rs` — upstream Blitz's
//! stylo↔taffy converter, not rinch's code — maps all four keywords to
//! `Dimension::AUTO` as well, for the same reason. So implementing them needs a
//! rinch-side measurement pass, which is deliberately not in this PR.
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
//! Bold is where Chrome and `auto` disagree, i.e. where rinch is wrong today.
//! The two halves are **mirror images**: the three intrinsic keywords are
//! already correct wherever `auto` is content-sized, and `stretch` is already
//! correct wherever `auto` fills. That is the useful thing to know when reading
//! the diagnostic `from_stylo` prints, and it is why the diagnostic says "which
//! matches a browser only where `auto` already gives the same used size" rather
//! than claiming the layout is wrong.
//!
//! `min-content` and `max-content` coincide in this table because a single
//! declared block offers no break opportunity. A second Chrome run with two
//! 100px `inline-block`s in a `font-size: 0` parent — so the space between them
//! has no width, and the numbers stay font-free — separates them: `max-content`
//! 200, `min-content` 100, `fit-content` 200 in an 800px containing block and
//! 150 in a 150px one. No fixture uses that shape, because rinch answers the
//! containing block's width for all of them either way.
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

/// The load-bearing claim behind "this is not a mapping".
///
/// A `Dimension` carrying `CompactLength::max_content()` can only be built
/// through `unsafe from_raw` — no safe constructor exists, because `Dimension`
/// does not implement `TaffyMaxContent` — and handing one to a block layout
/// **panics** at `MaybeResolve for Dimension`'s `_ => unreachable!()`.
///
/// If a future Taffy makes this work, this test fails, and most of #626's
/// follow-up evaporates: the fix would become the match arm the issue first
/// assumed it was. The grid half is asserted beside it so the failure message
/// distinguishes "Taffy grew box-level support" from "the grid API moved".
#[test]
fn taffy_0_12_cannot_carry_an_intrinsic_keyword_in_a_dimension() {
    use taffy::prelude::*;
    use taffy::{CompactLength, Dimension, Style};

    // Grid track sizing functions *do* take the keyword, safely. This is the
    // whole of Taffy 0.12's intrinsic-sizing support.
    assert!(MaxTrackSizingFunction::MAX_CONTENT.is_max_content());
    assert!(MinTrackSizingFunction::MIN_CONTENT.is_min_content());

    #[allow(unsafe_code)]
    let intrinsic = unsafe { Dimension::from_raw(CompactLength::max_content()) };
    assert!(
        !intrinsic.is_auto(),
        "a max-content Dimension is not auto, so nothing routes it to the auto path"
    );

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(|| {
        let mut t: TaffyTree<()> = TaffyTree::new();
        #[allow(unsafe_code)]
        let width = unsafe { Dimension::from_raw(CompactLength::max_content()) };
        let child = t
            .new_leaf(Style {
                size: Size {
                    width,
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
    });
    std::panic::set_hook(hook);

    assert!(
        outcome.is_err(),
        "Taffy laid out an intrinsic Dimension instead of panicking — it gave {outcome:?}. \
         If Taffy now supports this, #626 becomes a mapping after all: revisit \
         `DimensionValue::to_taffy` and delete this test."
    );
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

/// `is_auto` is the *specified* question and `lays_out_as_auto` the *used* one.
/// Every caller that means the second was switched to it, so this PR changes no
/// layout; the split exists so the day the keywords are implemented, the call
/// sites needing review are the ones this method names.
#[test]
fn is_auto_and_lays_out_as_auto_are_different_questions() {
    let k = DimensionValue::Intrinsic(IntrinsicSize::MaxContent);
    assert!(!k.is_auto(), "the author did not write `auto`");
    assert!(k.lays_out_as_auto(), "but it reaches Taffy as `auto`");

    assert!(DimensionValue::Auto.is_auto());
    assert!(DimensionValue::Auto.lays_out_as_auto());
    assert!(!DimensionValue::Length(10.0).lays_out_as_auto());
    assert!(!DimensionValue::Percent(0.5).lays_out_as_auto());
    assert!(!DimensionValue::Calc { px: 1.0, pct: 0.5 }.lays_out_as_auto());
}

/// The one place where telling the two apart would silently change layout.
///
/// `<textarea rows=N>` gets an intrinsic `min-height` of N lines because its
/// value lives in an attribute and gives it no content height. That is gated on
/// the author having left the height auto — and a height of `max-content` is
/// *laid out as* auto, so the gate must still open. Reading `is_auto()` there
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
            "`height: {k}` lays out as auto, so the rows minimum must still apply"
        );
    }
}

// ---------------------------------------------------------------------------
// Deviation records — these pass on `main` too. See the module doc.
// ---------------------------------------------------------------------------

/// The defect, stated once: in every context, all four keywords give **exactly
/// what `auto` gives**. Compare with the oracle table in the module doc, where
/// seven of those eleven contexts disagree with `auto` in Chrome.
///
/// This is a rinch-against-rinch comparison, so it pins no absolute number and
/// cannot drift with a font or a Taffy bump. It is the test a future
/// implementation must flip.
#[test]
fn every_intrinsic_keyword_lays_out_exactly_like_auto() {
    let mut same = 0;
    for (label, cb, ts, prop) in CASES {
        let (doc, t) = build(cb, ts, prop, "auto");
        let control = size(&doc, t);
        for k in &KEYWORDS[1..] {
            let (doc, t) = build(cb, ts, prop, k);
            assert_eq!(
                size(&doc, t),
                control,
                "{label}, `{prop}: {k}`: rinch does not implement the keyword, so it must \
                 still be indistinguishable from `auto`. If this now differs, #626 is being \
                 fixed — flip this file against the oracle table in its module doc."
            );
            same += 1;
        }
    }
    assert_eq!(
        same,
        CASES.len() * 4,
        "every case and keyword was exercised"
    );
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

/// The issue's own headline, with the number Chrome gives beside it.
///
/// `width: max-content` on a block box fills its containing block (800) where a
/// browser shrink-wraps it to its content (300).
#[test]
fn deviation_width_max_content_on_a_block_fills_its_container() {
    let (doc, t) = build("width: 800px", "", "width", "max-content");
    assert_eq!(size(&doc, t).0, 800.0, "rinch today; Chrome 150 gives 300");
}

/// The mirror-image half, which the issue did not cover: `stretch` is wrong in
/// exactly the places the other three are right.
///
/// `height: stretch` in a 300px-tall containing block is the content height (20)
/// where a browser fills the block (300). This is the `-webkit-fill-available`
/// use case, and it is the one of the four with no workaround short of
/// `height: 100%` plus a zero margin/border/padding on that axis.
#[test]
fn deviation_height_stretch_does_not_fill_a_definite_containing_block() {
    let (doc, t) = build("width: 800px; height: 300px", "", "height", "stretch");
    assert_eq!(size(&doc, t).1, 20.0, "rinch today; Chrome 150 gives 300");
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
