//! A line box's height is the **maximum** over every inline-level box on it
//! and the strut (CSS 2.1 §10.8.1) — #577 and #624.
//!
//! rinch does not do that today, and the defect is **not in rinch**. It is in
//! parley, which is why this file is mostly `#[ignore]`d fixtures carrying
//! Chrome's numbers rather than a fix. **#656** is the root-cause record and
//! carries the reference patch. Read there at both revisions — a read, not a
//! run — the defect is present verbatim at parley `v0.11.1` as well as at our
//! pinned `f6a8485`, so the upgrade does not close these on its own.
//!
//! # What was measured, and where the defect is
//!
//! rinch hands parley every span's `line-height`: `Ifc::inline_style_props`
//! pushes a `StyleProperty::LineHeight` for any element whose computed
//! `line-height` resolves (instrumented — a `line-height: 60px` span arrives as
//! `Absolute(60.0)`). rinch does **not** post-process line metrics; the IFC
//! root's measured height is `parley::Layout::height()` verbatim
//! (`layout_engine.rs`'s `NodeContext::InlineRoot` arm). Parley does take a
//! per-line maximum (`layout/line_break.rs`'s `add_line_height`). So all three
//! halves of the obvious story are already in place, and the line is still
//! wrong.
//!
//! The max is taken over **shaping runs**, and at this revision a shaping run
//! gets its line-height from a single, arbitrary style:
//!
//! - `parley/src/shape/mod.rs` breaks a run on `font_size`, `locale`,
//!   `font_variations`, `font_features`, `letter_spacing`, `word_spacing`, the
//!   bidi level, the script and an inline box — **not** on `line_height`. So
//!   `text <span> text` is one run with one line-height.
//! - and that run's `style_index` is **off by one**: `item.style_index` is
//!   advanced at the character where the style changes, *before* the pending
//!   run is handed to `shape_item`, so a run reports the style of the run
//!   *after* it. (Upstream parley has since fixed this half — its `HEAD` reads
//!   `char_style_indices[shaped_run.range.char_range.start]`.)
//!
//! The off-by-one is why the symptom reads as incoherent rather than as "the
//! span is ignored", and `a_span_on_the_second_line_raises_only_the_second` /
//! `a_span_on_the_first_line_raises_only_the_first` are the pair that shows
//! it: the *same* declaration on the *same* two-line shape gives 120 in one
//! order and 40 in the other, where Chrome gives 80 for both. No rule of the
//! form "take the maximum" applied on rinch's side can produce both numbers,
//! because rinch is not the thing choosing.
//!
//! # Chrome 150, standards mode (`CSS1Compat`), served over HTTP
//!
//! Container `width: 400px; font-size: 16px` throughout; `line-height` as
//! noted. `rinch` is this tree's measurement at the time of writing.
//!
//! | shape | container `line-height` | Chrome | rinch |
//! |---|---|---|---|
//! | `before <span lh:60>MID</span> after` | 20 | 60 | **20** |
//! | `before <span>MID</span> after` (control) | 20 | 20 | 20 |
//! | `before <span fs:30>MID</span> after` | `1.5` | 45 | 45 |
//! | `before <span>MID</span> after` (control) | `1.5` | 24 | 24 |
//! | `<span lh:60>MID</span>` alone | 20 | 60 | 60 |
//! | `<span contents lh:60>MID</span>` beside text | 20 | 60 | **20** |
//! | `x<span lh:35>A</span><span lh:60>B</span>y` | 20 | 60 | **20** |
//! | `x<span lh:60>A<span lh:10>B</span></span>y` | 20 | 60 | **20** |
//! | `before <span lh:10>MID</span> after` | 40 | 40 | 40 |
//! | `aaa<br><span lh:60>bbb</span>` | 20 | 80 | **120** |
//! | `<span lh:60>aaa</span><br>bbb` | 20 | 80 | **40** |
//! | `x<span lh:60>A</span>y<br>zzz` | 20 | 80 | **40** |
//! | `<span inline-block 30x10>` alone | 40 | 40 | **10** |
//! | `<span inline-flex 30x10>` alone | 40 | 40 | **10** |
//! | `<span inline-grid 30x10>` alone | 40 | 40 | **10** |
//! | `A<span inline-block 30x10>` | 40 | 40 | 40 |
//! | `A<span inline-block 30x55>` | 40 | 70 | **55** |
//! | `<span inline-block 30x55>` alone | 40 | 70 | **55** |
//!
//! **Every number asserted below is a declaration, not a glyph measurement.**
//! Line heights are declared lengths (or a unitless multiple of a declared
//! `font-size`), and box heights are declared. Two rows above are the
//! exception and are deliberately *not* asserted as equalities: the 70s come
//! from the strut's descent and half-leading below the baseline, which is a
//! property of the local font. Those two are asserted as inequalities. Nothing
//! here wraps text, either — the multi-line shapes use `<br>`, so no assertion
//! depends on where a line would break.
//!
//! # What is live and what is ignored
//!
//! The live tests are the shapes rinch already gets right. They are here so a
//! fix for the ignored ones cannot buy the maximum by breaking them. Each of
//! the three obvious wrong rules is refused by one of them:
//!
//! - *ignore the spans, report the container's own* reads 60 as 20 at
//!   `a_span_alone_on_a_line_sets_the_line_height` and 45 as 24 at
//!   `a_larger_font_size_raises_a_shared_line`;
//! - *report the styled span's own* reads 40 as 10 at
//!   `a_smaller_span_line_height_does_not_shrink_the_line`;
//! - *report the last style on the line*, which is close to what rinch does
//!   today, reads 45 as 24 at `a_larger_font_size_raises_a_shared_line`.
//!
//! The ignored ones fail today and carry Chrome's number. **Un-ignore them
//! when the parley upgrade lands** (#656) — they are its acceptance test.
//! Running them with `--ignored` before it is the fail-first.
//!
//! #624 (the strut) is tracked separately from #577 (the per-span maximum) and
//! measured to be a separate fix: with the run-splitting repaired, a line
//! holding only an atomic inline still has no text run to carry a line-height,
//! so it is still the box's own height. It needs a strut carrying an ascent
//! and a descent around the baseline, which parley expresses only from `main`
//! onwards — see #656.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 300.0;

/// `width: 400px; font-size: 16px; line-height: 20px` — the container every
/// fixture whose declaration is a length is built in.
const C20: &str = "width: 400px; font-size: 16px; line-height: 20px";
/// The same at `line-height: 40px`, for the shapes whose subject is larger
/// than 20 in the *other* direction (a smaller span, a short atomic inline).
const C40: &str = "width: 400px; font-size: 16px; line-height: 40px";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// Lay out one container built by `build` and return its border-box height.
fn container_height(container_style: &str, build: impl Fn(&mut RinchDocument, NodeId)) -> f32 {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", container_style);
    build(&mut doc, c);
    doc.resolve_layout(VW, VH);
    doc.tree.get(c.0).unwrap().layout.height
}

/// `before <span style="{decl}">MID</span> after` in `container_style`.
fn span_between_text(container_style: &str, decl: &str) -> f32 {
    container_height(container_style, |d, c| {
        txt(d, c, "before ");
        let s = el(d, c, "span", decl);
        txt(d, s, "MID");
        txt(d, c, " after");
    })
}

/// `<span style="display: {display}; width: 30px; height: {h}px"></span>`,
/// optionally preceded by a text run.
fn atomic_inline(container_style: &str, display: &str, h: u32, with_text: bool) -> f32 {
    container_height(container_style, |d, c| {
        if with_text {
            txt(d, c, "A");
        }
        el(
            d,
            c,
            "span",
            &format!("display: {display}; width: 30px; height: {h}px"),
        );
    })
}

// ── live: the shapes rinch already gets right ─────────────────────────────

/// A larger `font-size` on a span raises the line it shares with smaller text,
/// because the container's `line-height` is a unitless multiple and therefore
/// recomputed against each element's own font size.
///
/// Chrome: 45 (`30px x 1.5`) against a 24 control (`16px x 1.5`). rinch agrees
/// on both, and the reason it agrees here and not for a declared length is the
/// whole subject of this file: a `font_size` change *does* split a parley
/// shaping run, so the 30px text gets a run of its own carrying its own
/// line-height.
///
/// The control is what makes this an assertion rather than a coincidence —
/// without it, "always report the container's line-height" would pass.
#[test]
fn a_larger_font_size_raises_a_shared_line() {
    let rel = "width: 400px; font-size: 16px; line-height: 1.5";
    let control = span_between_text(rel, "");
    assert_eq!(control, 24.0, "control: 16px x 1.5");
    assert_eq!(
        span_between_text(rel, "font-size: 30px"),
        45.0,
        "a 30px span under line-height: 1.5 raises the line to 30 x 1.5 \
         (Chrome 45); the undeclared control is {control}"
    );
}

/// A span that **is** the whole line sets the line's height.
///
/// Chrome 60 against a 20 control. This is the shape #577 calls out as already
/// working, and it is live so that a fix for the shared-line shape cannot be
/// bought by dropping the per-span line-height altogether.
#[test]
fn a_span_alone_on_a_line_sets_the_line_height() {
    let control = container_height(C20, |d, c| {
        let s = el(d, c, "span", "");
        txt(d, s, "MID");
    });
    assert_eq!(control, 20.0, "control: an undeclared span");
    assert_eq!(
        container_height(C20, |d, c| {
            let s = el(d, c, "span", "line-height: 60px");
            txt(d, s, "MID");
        }),
        60.0,
        "a line-height: 60px span that is the whole line makes the line 60 \
         (Chrome 60); the undeclared control is {control}"
    );
}

/// A **smaller** `line-height` on a span must not shrink the line below the
/// container's own.
///
/// Chrome 40, and the discriminating control is a container declaring 10 with
/// no span at all, which really is 10 — so a rule reaching for the styled
/// span's own declaration would read 10 here and be caught, where it would sail
/// through `a_span_raises_a_line_it_shares_with_text` below.
#[test]
fn a_smaller_span_line_height_does_not_shrink_the_line() {
    let ten = span_between_text("width: 400px; font-size: 16px; line-height: 10px", "");
    assert_eq!(
        ten, 10.0,
        "control: a container declaring 10px really is 10"
    );
    assert_eq!(
        span_between_text(C40, "line-height: 10px"),
        40.0,
        "a line-height: 10px span on a line-height: 40px container leaves the \
         line at 40 (Chrome 40) — the maximum, not the last declaration seen. \
         A container declaring 10px measures {ten}"
    );
}

/// An atomic inline shorter than the strut leaves the line at the strut, and a
/// taller one raises it.
///
/// Chrome: 40 for a 10px-tall box and 70 for a 55px-tall one, on a
/// `line-height: 40px` container. The 70 is **not** asserted — the part below
/// the baseline is the strut's descent plus half-leading, which is a property
/// of the local font. What is asserted is the ordering, which is not: the box
/// participates in the line's maximum, and the short one does not lower it.
#[test]
fn an_atomic_inline_beside_text_participates_in_the_maximum() {
    let short = atomic_inline(C40, "inline-block", 10, true);
    let tall = atomic_inline(C40, "inline-block", 55, true);
    assert_eq!(
        short, 40.0,
        "a 10px box beside text leaves the line at the 40px strut (Chrome 40)"
    );
    assert!(
        tall >= 55.0,
        "a 55px box beside text raises the line to at least its own height \
         (Chrome 70, which additionally carries the strut's descent below the \
         baseline); got {tall}"
    );
    assert!(
        tall > short,
        "the taller box must make a taller line ({tall} vs {short})"
    );
}

// ── #577: ignored until the per-span maximum is real ──────────────────────

/// A span's larger `line-height` raises a line it **shares** with plain text.
///
/// Chrome 60, rinch 20. This is #577's headline shape.
#[test]
#[ignore = "#577 (#656): rinch 20, Chrome 60 — un-ignore when the parley upgrade lands"]
fn a_span_raises_a_line_it_shares_with_text() {
    let control = span_between_text(C20, "");
    assert_eq!(control, 20.0, "control: an undeclared span");
    assert_eq!(span_between_text(C20, "line-height: 60px"), 60.0);
}

/// A `display: contents` wrapper carrying only `line-height`, beside text.
///
/// Chrome 60, rinch 20 — identical to the real `<span>` above, which is
/// exactly #574's claim and exactly why #574's own `line-height` fixture had
/// to be built with the styled element as the whole line. Once the shared-line
/// shape works, that fixture's shape stops being forced.
#[test]
#[ignore = "#577 (#656): rinch 20, Chrome 60, same as the real <span> — un-ignore when the parley upgrade lands"]
fn a_contents_wrapper_raises_a_line_it_shares_with_text() {
    assert_eq!(
        span_between_text(C20, "display: contents; line-height: 60px"),
        60.0
    );
}

/// Two spans on one line with different `line-height`s: the larger wins.
///
/// Chrome 60, rinch 20. The two spans differ from each other and from the
/// container, so the correct 60 is distinct from the first span's 35, from the
/// container's own 20, and from a doubled strut's 40 — no wrong rule here lands
/// on the right answer by arithmetic.
#[test]
#[ignore = "#577 (#656): rinch 20, Chrome 60 — un-ignore when the parley upgrade lands"]
fn the_larger_of_two_spans_on_a_line_wins() {
    assert_eq!(
        container_height(C20, |d, c| {
            txt(d, c, "x");
            let a = el(d, c, "span", "line-height: 35px");
            txt(d, a, "A");
            let b = el(d, c, "span", "line-height: 60px");
            txt(d, b, "B");
            txt(d, c, "y");
        }),
        60.0
    );
}

/// A nested inline declaring a *smaller* `line-height` inside one declaring a
/// larger one: the outer still raises the line.
///
/// Chrome 60, rinch 20. This is the shape that refuses "the innermost
/// declaration wins".
#[test]
#[ignore = "#577 (#656): rinch 20, Chrome 60 — un-ignore when the parley upgrade lands"]
fn an_outer_span_wins_over_a_smaller_nested_one() {
    assert_eq!(
        container_height(C20, |d, c| {
            txt(d, c, "x");
            let outer = el(d, c, "span", "line-height: 60px");
            txt(d, outer, "A");
            let inner = el(d, outer, "span", "line-height: 10px");
            txt(d, inner, "B");
            txt(d, c, "y");
        }),
        60.0
    );
}

/// A span on the **second** line must not raise the first.
///
/// Chrome 80 (20 + 60), rinch **120** — rinch raises *both* lines. Half of the
/// witness pair: this is the direction where rinch over-applies.
#[test]
#[ignore = "#577 (#656): rinch 120, Chrome 80, the value leaks onto the line above — un-ignore when the parley upgrade lands"]
fn a_span_on_the_second_line_raises_only_the_second() {
    assert_eq!(
        container_height(C20, |d, c| {
            txt(d, c, "aaa");
            el(d, c, "br", "");
            let s = el(d, c, "span", "line-height: 60px");
            txt(d, s, "bbb");
        }),
        80.0
    );
}

/// The same declaration, the same two lines, the span on the **first** line.
///
/// Chrome 80 (60 + 20), rinch **40** — rinch loses it entirely. The other half
/// of the witness pair, and together with the test above it is why no uniform
/// correction applied to parley's answer can work: one input shape, one
/// declaration, and the two orders need opposite corrections.
#[test]
#[ignore = "#577 (#656): rinch 40, Chrome 80, the value is lost — un-ignore when the parley upgrade lands"]
fn a_span_on_the_first_line_raises_only_the_first() {
    assert_eq!(
        container_height(C20, |d, c| {
            let s = el(d, c, "span", "line-height: 60px");
            txt(d, s, "aaa");
            el(d, c, "br", "");
            txt(d, c, "bbb");
        }),
        80.0
    );
}

/// A span mid-line on the first of two lines: that line grows, the next does
/// not.
///
/// Chrome 80 (60 + 20), rinch 40.
#[test]
#[ignore = "#577 (#656): rinch 40, Chrome 80 — un-ignore when the parley upgrade lands"]
fn a_mid_line_span_grows_its_line_and_not_the_next() {
    assert_eq!(
        container_height(C20, |d, c| {
            txt(d, c, "x");
            let s = el(d, c, "span", "line-height: 60px");
            txt(d, s, "A");
            txt(d, c, "y");
            el(d, c, "br", "");
            txt(d, c, "zzz");
        }),
        80.0
    );
}

// ── #624: ignored until a line with no text run gets the strut ────────────

/// A line holding only an atomic inline still gets the strut.
///
/// Chrome 40 for all three atomic-inline spellings on a `line-height: 40px`
/// container; rinch 10, the box's own height, for all three. All three are
/// asserted together because #624's own measurement is that they behave
/// identically — a fix that reached only one of them would be fixing the wrong
/// thing.
#[test]
#[ignore = "#624 (#656): rinch 10, Chrome 40, a line with no text run gets no strut — un-ignore when the parley upgrade lands"]
fn an_atomic_inline_alone_on_a_line_still_gets_the_strut() {
    for display in ["inline-block", "inline-flex", "inline-grid"] {
        assert_eq!(
            atomic_inline(C40, display, 10, false),
            40.0,
            "a 30x10 {display} alone on a line-height: 40px line (Chrome 40)"
        );
    }
}

/// An atomic inline **taller** than the strut still gets the strut's descent
/// below the baseline.
///
/// Chrome 70 for a 30x55 box on a `line-height: 40px` container: the box's
/// baseline is its bottom margin edge, so it contributes 55 above the baseline
/// and the strut contributes its descent plus half-leading below. rinch gives
/// exactly 55 — nothing below the baseline at all.
///
/// Asserted as a strict inequality rather than as 70, because what is below
/// the baseline is font-derived. That is enough to fail today and enough to
/// distinguish a real strut from a scalar floor: a floor of 40 leaves this at
/// 55, which is measured, not forecast.
#[test]
#[ignore = "#624 (#656): rinch exactly 55, Chrome 70, no strut below the baseline — un-ignore when the parley upgrade lands"]
fn a_tall_atomic_inline_alone_still_gets_the_struts_descent() {
    let h = atomic_inline(C40, "inline-block", 55, false);
    assert!(
        h > 55.0,
        "a 30x55 inline-block alone on a line-height: 40px line must be taller \
         than the box itself (Chrome 70); got {h}"
    );
}

/// The same, with a text run on the line.
///
/// Chrome 70, rinch 55. Kept separate from the case above because the two are
/// not the same defect shape: this line *has* a text run carrying the strut's
/// line-height, and it is still 55 — so the shortfall here is not "no strut"
/// but "the strut contributes a scalar height rather than an ascent and a
/// descent around the baseline".
#[test]
#[ignore = "#624 (#656): rinch exactly 55, Chrome 70, the strut has no descent — un-ignore when the parley upgrade lands"]
fn a_tall_atomic_inline_beside_text_still_gets_the_struts_descent() {
    let h = atomic_inline(C40, "inline-block", 55, true);
    assert!(
        h > 55.0,
        "a 30x55 inline-block beside text on a line-height: 40px line must be \
         taller than the box itself (Chrome 70); got {h}"
    );
}
