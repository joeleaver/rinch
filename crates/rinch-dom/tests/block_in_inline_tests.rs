//! #513 — block-level content inside an inline element, and everything after it.
//!
//! ```html
//! <div><a>text<div>block</div>tail</a></div>
//! ```
//!
//! rinch used to draw `text` and nothing else. It now draws all three, by
//! splitting the inline box around the block-level child (CSS 2.1 §9.2.1.1,
//! "block-in-inline") — the element is flattened out of the box tree, an anonymous
//! block box lays out each run of inline content, and the block becomes their
//! sibling.
//!
//! This file was written as the **falsifier**, before any fix existed, so that it
//! could not be shaped by one: it pinned what the defect was, what a browser
//! actually does, and — just as important — the neighbouring shapes a candidate
//! fix must not change. It is now the acceptance suite for the fix that followed,
//! with the same fixtures and the same numbers, un-`#[ignore]`d.
//!
//! **What the fix does NOT do is model fragment identity.** CSS gives the inline
//! box one real fragment per side, each with its own border and padding; rinch
//! gives the *geometry* of those fragments and a background rectangle per
//! fragment, and keeps no box for the element itself. Do not read "block-in-inline
//! splitting landed" as "rinch has inline fragments".
//!
//! Two fixtures stay `#[ignore]`d, and neither is #513: **#591** (an out-of-flow
//! child of an inline is orphaned the same way, and CSS forbids splitting around
//! one, so the fix deliberately leaves the shape alone) and **#592** (an
//! `inline-block` with mixed content must generate anonymous boxes *inside*
//! itself, which is a different mechanism).
//!
//! # The oracle: delete the wrapper
//!
//! Every assertion below is "the same content with the inline wrapper deleted",
//! built in the same test. That oracle was **measured in Chrome 150 (standards
//! mode), not assumed** — for every shape in this file the wrapped and unwrapped
//! twins are identical to the pixel:
//!
//! | shape (400px wide, `line-height: 20px`, block child 40x30) | wrapped | unwrapped |
//! |---|---|---|
//! | `text <blk> tail`                        | H=70, blk\@y=20, tail\@y=51 | **identical** |
//! | `<blk> tail`                             | H=50, blk\@y=0,  tail\@y=31 | **identical** |
//! | `text <blk>`                             | H=50, blk\@y=20             | **identical** |
//! | `<blk>` alone                            | H=30, blk\@y=0              | **identical** |
//! | `out <b>in <blk> after</b> tailout`      | H=70, blk\@y=20             | **identical** |
//! | `text <blk> tail` + a following sibling  | H=70, sibling\@y=51         | **identical** |
//! | `t1 <blk> t2 <blk> t3`                   | H=120, blks\@y=20,70        | **identical** |
//! | `text <abs blk> tail`                    | H=20, tail on line 1        | **identical** |
//!
//! So the property is: **a bare inline wrapper around in-flow content changes
//! nothing.** That is a fact about the *input* — the wrapper's presence — which
//! no candidate implementation controls, so a fixture written on it bites under
//! any of them (`review-566-plan.md` §5g). It is also font-independent: nothing
//! here asserts a text width or an x coordinate, only line counts, block
//! positions and ink.
//!
//! An empty `<a>` fragment costs **no** line box either: `<a><blk></a>` is H=30
//! in Chrome, not 30+20+20. The zero-width fragments Chrome reports before and
//! after the block contribute nothing.
//!
//! # Fixed points stepped off on purpose
//!
//! - **Block position within the inline.** First, middle and last each have a
//!   fixture: "middle" is where "split into three" and "put the block after
//!   everything" agree on the block's y, and "last" is where "split" and
//!   "drop the tail" agree on the height.
//! - **Block cardinality.** One block is where "one split" and "split at every
//!   block" agree, so there is a two-block fixture (H=120, not 70).
//! - **Inline nesting depth.** One level is where "split the inline" and "split
//!   the outermost inline" agree, so the block is two inlines deep in one fixture.
//! - **Wrapper identity.** `<a>` alone would let a UA-stylesheet accident pass
//!   for a fix, so the same shape is built with `<span>` too.
//!
//! # What is NOT #513, and is asserted live here as a guard rail
//!
//! An **out-of-flow** child does not split an inline in CSS, and rinch agrees
//! (#406 classifies it `OutOfFlow`, which neither joins a run nor ends one).
//! Chrome: `<a>text<abs>tail</a>` is one 20px line with `tail` beside `text`.
//! That fixture is **not** `#[ignore]`d — it passed before the fix and still
//! passes, which is what says the fix did not widen its own predicate into a
//! shape CSS excludes. Its *pixels* are a different matter and are #591.
//!
//! # The three twins
//!
//! An inline wrapper is compared against **no wrapper** (the oracle Chrome was
//! measured on) and against a **`display: contents` wrapper** — a control of a
//! different kind, because it runs through the very machinery the fix reuses, and
//! it already produced Chrome's numbers on a base where the inline wrapper did
//! not. Where a shape's two markups genuinely differ (the hand-split `<b>` in
//! `nested`, whose block child inherits a different font weight), the difference
//! is declared away at the site and the measurement is recorded there.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;
/// One line is exactly this, so "one line versus three" is a 20px question
/// rather than a font question.
const LINE: f32 = 20.0;
/// The block child: 40x30, so it is never confusable with a line box.
const BLK_H: f32 = 30.0;
const CONTAINER: &str = "width: 400px; line-height: 20px; font-size: 16px";
const BLK: &str = "width: 40px; height: 30px; background: rgb(255, 0, 255)";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// A node's on-screen y, summed through its ancestors — `layout.y` is
/// parent-relative, and the wrapped and unwrapped twins do not have the same
/// ancestors, so comparing the raw field would compare two different things.
fn abs_y(doc: &RinchDocument, id: NodeId) -> f32 {
    let mut y = 0.0;
    let mut cur = Some(id.0);
    while let Some(c) = cur {
        let n = doc.tree.get(c).unwrap();
        y += n.layout.y;
        cur = n.parent;
    }
    y
}

fn assert_consistent(doc: &RinchDocument, what: &str) {
    let t = doc.taffy_tree_violations();
    assert!(
        t.is_empty(),
        "{what}: Taffy tree inconsistent:\n  {}",
        t.join("\n  ")
    );
    let r = doc.run_bookkeeping_violations();
    assert!(
        r.is_empty(),
        "{what}: run bookkeeping inconsistent:\n  {}",
        r.join("\n  ")
    );
}

/// The same, for a fixture that is **live against a defect that is still
/// open** — now **#591**, seen from the validator side.
///
/// `taffy_tree_violations`' `D` rule (#589) reports a subtree that is laid out by
/// no compute pass. An **out-of-flow** child of a bare inline is exactly that: CSS
/// 2.1 §9.4.2 does not split an inline around one, so the element is detached into
/// the IFC whole, the absolute stays in its Taffy child list and goes with it, and
/// nothing lays it out.
///
/// It asserts **which** violations it expects rather than waiving the check, and
/// the non-empty assertion is the half that matters: when #591 is fixed this fails,
/// names itself, and the fixture goes back to [`assert_consistent`]. A waiver would
/// simply go quiet and stale.
///
/// **It has already done that once.** This helper had two callers whose subject was
/// the *block* case; #513's fix made both clean and the non-empty assertion failed
/// at exactly that moment, which is the only evidence a construct like this can
/// ever give that it bites rather than decorates.
fn assert_only_known_591_detachment(doc: &RinchDocument, what: &str) {
    let t = doc.taffy_tree_violations();
    let unexpected: Vec<&String> = t.iter().filter(|l| !l.starts_with("D detached")).collect();
    assert!(
        unexpected.is_empty(),
        "{what}: Taffy tree inconsistent beyond #591's detachment:\n  {}",
        unexpected
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        !t.is_empty(),
        "{what}: nothing is detached any more — #591 appears to be fixed. \
         Replace this call with `assert_consistent`.",
    );
    let r = doc.run_bookkeeping_violations();
    assert!(
        r.is_empty(),
        "{what}: run bookkeeping inconsistent:\n  {}",
        r.join("\n  ")
    );
}

/// Build one shape twice: `wrapped` puts the whole content inside a bare inline
/// element, `!wrapped` puts it straight in the container. Returns the container
/// and every block child, in document order.
///
/// The two must agree — see the module docs; that identity is what Chrome does.
struct Twin {
    doc: RinchDocument,
    container: NodeId,
    blocks: Vec<NodeId>,
}

fn build(shape: &str, wrapped: bool, wrapper_tag: &str) -> Twin {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container_style = if shape == "abs" || shape == "abs_only" {
        format!("{CONTAINER}; position: relative")
    } else {
        CONTAINER.to_string()
    };
    let c = el(&mut doc, body, "div", &container_style);
    // The host every child is appended to: the wrapper, or the container.
    //
    // `wrapper_tag == "contents"` builds a `display: contents` wrapper instead of
    // an inline one — **a third twin, and a control of a different kind**. The
    // unwrapped twin says "the wrapper changes nothing"; this one says "an inline
    // wrapper changes nothing *that a boxless wrapper does not also change*", and
    // it runs through the very machinery #513's fix reuses (`collect_run_units`
    // flattens a split inline exactly as it flattens a `display: contents`
    // element). It was measured before the fix existed: the contents twin of
    // `middle` was already 70 and of `only` already 30 — Chrome's numbers — on a
    // base where the inline twins were 20 and 0. So it is a reachable control,
    // not an aspiration, and the fix's job was to make the inline wrapper agree
    // with it.
    let h = if wrapped {
        match wrapper_tag {
            "contents" => el(&mut doc, c, "span", "display: contents"),
            tag => el(&mut doc, c, tag, ""),
        }
    } else {
        c
    };

    let mut blocks = Vec::new();
    let block = |doc: &mut RinchDocument, parent: NodeId, label: &str, style: &str| -> NodeId {
        let b = el(doc, parent, "div", style);
        txt(doc, b, label);
        b
    };

    match shape {
        "middle" => {
            txt(&mut doc, h, "text");
            blocks.push(block(&mut doc, h, "block", BLK));
            txt(&mut doc, h, "tail");
        }
        "first" => {
            blocks.push(block(&mut doc, h, "block", BLK));
            txt(&mut doc, h, "tail");
        }
        "last" => {
            txt(&mut doc, h, "text");
            blocks.push(block(&mut doc, h, "block", BLK));
        }
        "only" => {
            blocks.push(block(&mut doc, h, "block", BLK));
        }
        "nested" => {
            // The block is two inlines deep when wrapped. Unwrapped, the outer
            // wrapper is deleted and the inner one is **split by hand** around
            // the block.
            //
            // **The hand split is the oracle, and deleting the `<b>` was a
            // fixture bug.** `oracle/twins.html` measures `t6p` as
            // `out<b>in</b><div/><b>after</b>tailout` — it keeps the `<b>` on
            // both sides — and that is why Chrome renders the pair identically.
            // This fixture used to delete the `<b>` entirely, which also deleted
            // its UA `font-weight: 700`, so the wrapped side was bold and the
            // unwrapped side was not: measured, `[281, 828, 649, 365, 0, 0]`
            // against `[240, 805, 629, 322, 0, 0]`. Every layout assertion was
            // blind to it, because boldness does not change a line count, so it
            // survived until #513's fix made the pixel comparison run at all.
            //
            // Matching the oracle also makes the property **stronger** rather
            // than weaker: rinch's *automatic* split must produce what a
            // *hand-written* split produces, which is exactly what CSS 2.1
            // §9.2.1.1 specifies the automatic one to mean.
            //
            // (An attempt to neutralise the weight instead — `<b style="font-
            // weight: normal">` — does **not** work on this base: that element
            // still computes 700, while `<strong style="font-weight: 300">`
            // computes 300. A separate defect, reported, not worked around here.)
            //
            // **The block declares its own `font-weight`, and that is the last
            // difference the two markups cannot avoid.** Wrapped, the block is a
            // child of the `<b>` and its label inherits `700`; hand-split, the
            // block sits between the two `<b>`s and inherits `400`. That is true
            // of Chrome as well — it is a property of the markup, not of either
            // implementation — so the label is pinned numerically and the
            // comparison is exact again. Measured: band 1 was the only one left
            // differing, 828 against 805. (`400`, not `normal`: see the note
            // above about which spellings this base honours.)
            const NESTED_BLK: &str = "width: 40px; height: 30px; \
                                      background: rgb(255, 0, 255); font-weight: 400";
            txt(&mut doc, h, "out");
            if wrapped {
                let inner = el(&mut doc, h, "b", "");
                txt(&mut doc, inner, "in");
                blocks.push(block(&mut doc, inner, "block", NESTED_BLK));
                txt(&mut doc, inner, "after");
            } else {
                let before = el(&mut doc, h, "b", "");
                txt(&mut doc, before, "in");
                blocks.push(block(&mut doc, h, "block", NESTED_BLK));
                let after = el(&mut doc, h, "b", "");
                txt(&mut doc, after, "after");
            }
            txt(&mut doc, h, "tailout");
        }
        "sibling_after" => {
            txt(&mut doc, h, "text");
            blocks.push(block(&mut doc, h, "block", BLK));
            txt(&mut doc, h, "tail");
            // Deliberately a child of the CONTAINER, not the wrapper: #513 says
            // content after the inline vanishes too.
            txt(&mut doc, c, "sibling");
        }
        "two_blocks" => {
            txt(&mut doc, h, "t1");
            blocks.push(block(&mut doc, h, "b1", BLK));
            txt(&mut doc, h, "t2");
            blocks.push(block(&mut doc, h, "b2", BLK));
            txt(&mut doc, h, "t3");
        }
        "abs" => {
            txt(&mut doc, h, "text");
            blocks.push(block(
                &mut doc,
                h,
                "block",
                &format!("{BLK}; position: absolute"),
            ));
            txt(&mut doc, h, "tail");
        }
        "float" => {
            txt(&mut doc, h, "text");
            blocks.push(block(&mut doc, h, "block", &format!("{BLK}; float: left")));
            txt(&mut doc, h, "tail");
        }
        other => panic!("unknown shape {other}"),
    }
    doc.resolve_layout(VW, VH);
    Twin {
        doc,
        container: c,
        blocks,
    }
}

/// The whole assertion, for a shape whose wrapped and unwrapped twins Chrome
/// renders identically: same container height, same block y, same tail y.
fn assert_wrapper_changes_nothing(shape: &str, wrapper_tag: &str, control_height: f32) {
    let w = build(shape, true, wrapper_tag);
    let p = build(shape, false, wrapper_tag);

    // Off the fixed point: the control must be the shape we think it is, or
    // "both are one line" would pass.
    assert_eq!(
        height_of(&p.doc, p.container),
        control_height,
        "control ({shape}, no wrapper) is not the expected shape",
    );

    assert_eq!(
        height_of(&w.doc, w.container),
        height_of(&p.doc, p.container),
        "{shape} in <{wrapper_tag}>: a bare inline wrapper must change nothing. \
         wrapped={} unwrapped={} (Chrome renders these identically)",
        height_of(&w.doc, w.container),
        height_of(&p.doc, p.container),
    );

    assert_eq!(
        w.blocks.len(),
        p.blocks.len(),
        "{shape}: fixture built different trees"
    );
    for (i, (&bw, &bp)) in w.blocks.iter().zip(p.blocks.iter()).enumerate() {
        assert_eq!(
            (abs_y(&w.doc, bw), height_of(&w.doc, bw)),
            (abs_y(&p.doc, bp), height_of(&p.doc, bp)),
            "{shape} in <{wrapper_tag}>: block {i} must land where it lands with the \
             wrapper deleted",
        );
    }
    // Where the *text* after the block lands is asserted in `painted` below,
    // not here. A text node that an anonymous block box has taken into a run
    // keeps `layout.y == 0` and reports its position through the box — an
    // implementation fact, and a fixture that read it would be testing the
    // implementation rather than the property (`review-566-plan.md` §5g).
    // Ink in a band is the same question asked of the output.
    assert_consistent(&w.doc, shape);
    assert_consistent(&p.doc, shape);
}

// ── The defect ────────────────────────────────────────────────────────────
//
// `#[ignore]`, not deleted or commented out: the suite stays green, and
// `cargo test -p rinch-dom --test block_in_inline_tests -- --ignored` is the
// acceptance run for the fix. Each message carries the browser's number, so a
// failure says what a browser would have done rather than what this file expects.

/// The canonical markup from #513. Chrome: H=70 — one line, the 30px block, one
/// line. rinch used to say H=20, with the block and `tail` drawn nowhere.
#[test]
fn a_block_inside_an_inline_lays_out_as_if_the_inline_were_not_there() {
    assert_wrapper_changes_nothing("middle", "a", LINE + BLK_H + LINE);
}

/// The same shape in a `<span>`, so a fix cannot be an `<a>`-specific accident.
#[test]
fn the_wrapper_being_a_span_rather_than_a_link_changes_nothing() {
    assert_wrapper_changes_nothing("middle", "span", LINE + BLK_H + LINE);
}

/// **The third twin**: an inline wrapper must agree with a `display: contents`
/// wrapper, for every shape.
///
/// This is the control that was measured *before* the fix was designed, and it is
/// what made the design more than a guess: a boxless wrapper already produced
/// Chrome's numbers on these shapes (70 for `middle`, 30 for `only`) on a base
/// where an inline wrapper produced 20 and 0. The fix routes a split inline
/// through that same flattening, so this pair is the assertion that it really did
/// rather than reaching the same heights some other way.
#[test]
fn an_inline_wrapper_agrees_with_a_display_contents_wrapper() {
    for (shape, h) in [
        ("middle", LINE + BLK_H + LINE),
        ("first", BLK_H + LINE),
        ("last", LINE + BLK_H),
        ("only", BLK_H),
        ("two_blocks", LINE + BLK_H + LINE + BLK_H + LINE),
    ] {
        assert_wrapper_changes_nothing(shape, "contents", h);
        let inline = build(shape, true, "a");
        let contents = build(shape, true, "contents");
        assert_eq!(
            height_of(&inline.doc, inline.container),
            height_of(&contents.doc, contents.container),
            "{shape}: a split inline and a `display: contents` wrapper must \
             flatten to the same thing — they go through the same collector",
        );
    }
}

/// Block **first**: Chrome H=50, and there is no empty line box before it.
/// Off the fixed point of "middle" — a fix that always emits a leading run
/// would give 70.
#[test]
fn a_block_first_inside_an_inline_costs_no_leading_line() {
    assert_wrapper_changes_nothing("first", "a", BLK_H + LINE);
}

/// Block **last**: Chrome H=50, no trailing empty line.
#[test]
fn a_block_last_inside_an_inline_costs_no_trailing_line() {
    assert_wrapper_changes_nothing("last", "a", LINE + BLK_H);
}

/// A block as the inline's **only** child: Chrome H=30. Arity 0 for the run
/// grouping — no run exists at all, and the empty inline contributes no line.
#[test]
fn an_inline_holding_only_a_block_is_exactly_that_block_tall() {
    assert_wrapper_changes_nothing("only", "a", BLK_H);
}

/// The block is two inlines deep. Chrome splits **both** — `<a>` reports five
/// fragments and `<b>` three — and the result is still identical to the
/// wrapper-free twin.
#[test]
fn a_block_two_inlines_deep_splits_every_inline_above_it() {
    assert_wrapper_changes_nothing("nested", "a", LINE + BLK_H + LINE);
}

/// "and so does everything after it" — the sibling here is a child of the
/// **container**, after the inline, and Chrome puts it on the last line beside
/// `tail`.
#[test]
fn content_after_the_inline_survives_the_block_inside_it() {
    assert_wrapper_changes_nothing("sibling_after", "a", LINE + BLK_H + LINE);
}

/// Two blocks, three runs: Chrome H=120. One block is where "split once" and
/// "split at every block" agree.
#[test]
fn two_blocks_inside_one_inline_make_three_runs() {
    assert_wrapper_changes_nothing("two_blocks", "a", LINE + BLK_H + LINE + BLK_H + LINE);
}

/// `display: inline-block` is a **block container**, so CSS does *not* split it
/// — it generates anonymous block boxes inside itself, exactly like a `<div>`
/// would. Chrome: the inline-block is 40x70 and its three pieces stack.
///
/// Separate from the shapes above on purpose: the four sites skip
/// `InlineBlock` by the same predicate they skip `Inline` by, so a fix that
/// drops only `Inline` leaves this one standing. rinch today lays the three
/// pieces out in a **row** (H=30).
#[test]
#[ignore = "#592: an inline-block with mixed content does not generate anonymous \
           block boxes inside itself — a sibling defect, not #513"]
fn an_inline_block_with_mixed_content_stacks_like_a_block_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let ib = el(&mut doc, c, "span", "display: inline-block");
    txt(&mut doc, ib, "text");
    let b = el(&mut doc, ib, "div", BLK);
    txt(&mut doc, b, "block");
    txt(&mut doc, ib, "tail");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, ib),
        LINE + BLK_H + LINE,
        "Chrome makes the inline-block 40x70: text, block, tail stacked",
    );
    assert_eq!(
        abs_y(&doc, b) - abs_y(&doc, ib),
        LINE,
        "the block sits below the first line, not beside it",
    );
    assert_consistent(&doc, "inline-block with mixed content");
}

/// `display: inline-flex` is an **inline-level** box — it takes part in the
/// line around it exactly as `inline-block` does, and only its *inside* is a
/// flex container. Chrome renders `before <x>mid</x> after <blk>` identically
/// for `x = inline-flex` and `x = inline-block`: H=50, all three text runs on
/// one line, the block below.
///
/// rinch used to lose the distinction **before any of the four sites could see
/// it**: `DisplayValue::InlineFlex` mapped to `DisplayMode::Flex`
/// (`style_resolution/mod.rs`), which is the same value plain `display: flex`
/// gets, so `is_inline()` answered `false` and `inline_flow_role()` answered
/// `InFlowBlock`. The box therefore *ended the inline run* it should have
/// joined: `before` and `after` landed in two anonymous boxes on two different
/// lines, and the container was 90px where its `inline-block` twin is 52. Fixed
/// by giving `DisplayMode` an `InlineFlex` variant that answers
/// `is_inline_level` and `is_atomic_inline`.
///
/// The oracle is the pair, not a number: the twin differs only in the
/// wrapper's `display`, and Chrome makes them equal, so neither candidate
/// implementation controls the comparison. (The absolute numbers are not
/// asserted — rinch gives the `inline-block` twin 52px rather than 50 because
/// of that box's own line height, which is a separate question.)
///
/// `inline-grid` was the **same symptom through a different mechanism**, fixed
/// separately in #607: Stylo folded it into `DisplayValue::Grid`, which
/// `style_resolution` mapped to `DisplayMode::Block`, so there was no value left
/// to classify and no `DisplayMode` arm that could have helped. Measured on this
/// very shape: Chrome gives `inline-grid` the same 50 as the two above, and
/// rinch gave 90 until `DisplayValue` grew an `InlineGrid` of its own. The
/// `inline-grid` fixtures live in `atomic_inline_tests.rs` beside
/// `inline-flex`'s.
#[test]
fn an_inline_flex_box_joins_the_line_exactly_as_an_inline_block_does() {
    fn build_with(display: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        txt(&mut doc, c, "before");
        let w = el(&mut doc, c, "span", &format!("display: {display}"));
        txt(&mut doc, w, "mid");
        txt(&mut doc, c, "after");
        let b = el(&mut doc, c, "div", BLK);
        txt(&mut doc, b, "block");
        doc.resolve_layout(VW, VH);
        (doc, c)
    }

    let (fdoc, fc) = build_with("inline-flex");
    let (bdoc, bc) = build_with("inline-block");

    // Off the fixed point: the control must be the one-line-plus-block shape,
    // or "both are three lines" would pass.
    assert!(
        height_of(&bdoc, bc) < LINE + LINE + BLK_H,
        "control (inline-block): the three text runs share one line, so the \
         container is one line plus the block, not two lines plus the block \
         (got {})",
        height_of(&bdoc, bc),
    );
    assert_eq!(
        height_of(&fdoc, fc),
        height_of(&bdoc, bc),
        "an inline-flex box must sit in the line exactly as an inline-block \
         does — Chrome renders these two identically. inline-flex={} \
         inline-block={}",
        height_of(&fdoc, fc),
        height_of(&bdoc, bc),
    );
    assert_consistent(&fdoc, "inline-flex in a run");
}

// ── Guard rails: correct today, and a fix must keep them correct ──────────

/// The unwrapped shape — a plain block container with mixed content — is what
/// rinch already gets right, which is what makes the oracle above *reachable*
/// rather than aspirational. If this ever fails, every `#[ignore]`d fixture
/// above is measuring the wrong thing.
#[test]
fn the_oracle_shape_itself_is_correct_today() {
    let p = build("middle", false, "a");
    assert_eq!(
        height_of(&p.doc, p.container),
        LINE + BLK_H + LINE,
        "a block container with text+block+text is three bands tall",
    );
    assert_eq!(
        abs_y(&p.doc, p.blocks[0]),
        LINE,
        "the block starts after the first line"
    );
    assert_consistent(&p.doc, "the oracle shape");
}

/// An **out-of-flow** child does not split an inline — CSS 2.1 §9.2.1.1, and
/// #406 in rinch. Chrome: one 20px line, `tail` beside `text`, the absolute box
/// contributing no height.
///
/// Live, not ignored: this is the shape a fix most easily breaks, because an
/// absolutely positioned `<div>` inside an `<a>` *looks* exactly like the
/// canonical case one field away.
///
/// **This height is right and the pixels are not** — see
/// `painted::an_inline_wrapper_around_an_out_of_flow_child_changes_no_pixel`.
/// 20px is a fixed point: "the absolute adds no height" and "the absolute is
/// lost entirely" agree on it. What this fixture pins is only that a fix must
/// not turn 20 into 70; it is no evidence that the shape renders correctly.
#[test]
fn an_absolutely_positioned_child_does_not_split_an_inline() {
    let w = build("abs", true, "a");
    let p = build("abs", false, "a");
    assert_eq!(
        height_of(&p.doc, p.container),
        LINE,
        "control: an absolute child adds no height to its container",
    );
    assert_eq!(
        height_of(&w.doc, w.container),
        LINE,
        "an absolute child inside an inline must not make the container three bands tall",
    );
    assert_only_known_591_detachment(&w.doc, "absolute inside an inline");
}

/// rinch implements **no floats** (there is no `float` handling anywhere in
/// `rinch-dom`), so a floated child is laid out as an ordinary in-flow block —
/// which means a floated child of an inline **splits** it, and the container is
/// three bands tall where Chrome says one.
///
/// # This fixture used to assert 20 and that was right by accident
///
/// Chrome says 20: the float is out of flow, so it adds no line. rinch said 20
/// too, and for a completely different reason — the floated child and `tail`
/// after it were **drawn nowhere at all**, so 20px was what was left rather than
/// what was computed. Two wrongs cancelling is a fixed point, and this one hid a
/// pre-existing divergence behind the very defect #513 was about.
///
/// The unwrapped twin is the measurement that settles it: **`text<blk float>tail`
/// with no wrapper is 70px on this base too**, and always was. So the wrapper was
/// never the thing costing 50px — the missing float implementation was, and the
/// wrapper was concealing it by losing content. Asserting the twin equality keeps
/// the property this file is built on (a bare inline wrapper changes nothing)
/// while stating the divergence from Chrome plainly instead of pocketing it.
///
/// So this is no longer a Chrome oracle at all; it is a self-consistency oracle
/// with a known, stated gap. When floats land, both twins should become 20 and
/// this fixture should fail — which is exactly the notice it ought to give.
#[test]
fn a_floated_child_of_an_inline_matches_its_wrapper_free_twin() {
    let w = build("float", true, "a");
    let p = build("float", false, "a");

    // Off the fixed point in the other direction: if the control were 20, this
    // would pass by agreeing with Chrome and prove nothing about the wrapper.
    assert_eq!(
        height_of(&p.doc, p.container),
        LINE + BLK_H + LINE,
        "control: with no floats implemented, an unwrapped floated block stacks \
         like any other block — 70px, not Chrome\u{2019}s 20. If this is 20, floats \
         have been implemented and this whole fixture should be rewritten \
         against Chrome.",
    );
    assert_eq!(
        height_of(&w.doc, w.container),
        height_of(&p.doc, p.container),
        "a bare inline wrapper around a floated block must change nothing. Both \
         twins diverge from Chrome (20px) because rinch implements no floats; \
         what this pins is that they diverge *identically*.",
    );
    assert_consistent(&w.doc, "float inside an inline");
    assert_consistent(&p.doc, "float inside an inline, unwrapped");
}

// ── What the split does that the twin oracle cannot see ──────────────────────
//
// The twins answer "is the wrapper invisible". These answer questions the twins
// are structurally blind to, each listed in the falsifier's own can't-answer
// section: whether the block's box is positioned by the right ancestor, whether
// the element keeps a box it should not, whether its style and its background
// survive the split, whether any of it holds at a scale other than 1.0, and
// whether the shape survives being entered and left at runtime.

/// **A split inline after a block sibling** — the shape that proves
/// `split_inline_boxes` has to exist.
///
/// `<a><div>card</div></a>` holds no inline content, so `has_inline` is false and
/// no anonymous box is minted, so `create_anonymous_block_boxes`' own Taffy
/// rebuild never runs for the container. Without the separate split pass the block
/// stays a Taffy **child of the `<a>`** — and then Taffy measures its position
/// from the `<a>` while paint reaches it directly from the container (the `<a>` is
/// not in the box tree and its `layout` is zeroed), so with a block sibling above
/// it, it is drawn over that sibling.
///
/// A container with the `<a>` first would pass either way: the `<a>` sits at
/// `y = 0`, so "relative to the `<a>`" and "relative to the container" agree. The
/// sibling is what steps off that fixed point, and it is why this fixture puts one
/// there.
#[test]
fn a_block_only_split_inline_is_positioned_by_its_container_not_by_the_inline() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let lead = el(&mut doc, c, "div", "height: 25px");
    txt(&mut doc, lead, "lead");
    let link = el(&mut doc, c, "a", "");
    let card = el(&mut doc, link, "div", BLK);
    txt(&mut doc, card, "card");
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.get(link.0).unwrap().is_split_inline(),
        "precondition: an inline holding only a block is still split"
    );
    assert_eq!(
        height_of(&doc, c),
        25.0 + BLK_H,
        "the container stacks the lead block and the card — 55px. Before #513 \
         this was 25: the card was laid out by nobody."
    );
    assert_eq!(
        abs_y(&doc, card),
        25.0,
        "and the card is below the lead block. If the split pass is missing, the \
         card's `layout.y` is measured from the <a> instead of the container and \
         this is 0 — drawn over the lead."
    );
    assert_consistent(&doc, "block-only split inline after a sibling");
}

/// **A split inline generates no box of its own, and the validator enforces it.**
///
/// CSS 2.1 §9.2.1.1 gives the element one fragment per side of the block; rinch
/// gives the fragments' geometry through the anonymous boxes and keeps no box for
/// the element. So its `layout` must be exactly zero — and `E ghost box` says so,
/// which is what turns "it happens to be 0x0" into an assertion. This region hides
/// mutants on exactly that fixed point.
#[test]
fn a_split_inline_carries_no_box_and_the_validator_says_so() {
    let w = build("middle", true, "a");
    let link = w.doc.tree.get(w.container.0).unwrap().children[0];
    assert!(
        w.doc.tree.get(link).unwrap().is_split_inline(),
        "precondition"
    );
    let l = w.doc.tree.get(link).unwrap().layout;
    assert_eq!(
        (l.x, l.y, l.width, l.height),
        (0.0, 0.0, 0.0, 0.0),
        "a split inline carries no box"
    );
    assert!(
        !w.doc
            .taffy_tree_violations()
            .iter()
            .any(|v| v.starts_with("E ghost box")),
        "…and `E` agrees, which is the point: {:?}",
        w.doc.taffy_tree_violations()
    );
}

/// **The element tree still reaches the split inline**, so a handler or a
/// `:hover` on it is not lost.
///
/// `box_tree_children` no longer yields the `<a>`, which is what stops it being
/// painted twice — but the click path walks `node.parent`, the *element* tree, and
/// must still find it from both fragments and from the block between them. Deciding
/// that by reading the dispatch rather than assuming it is the difference between
/// this being a decision and a guess (`click_handling.rs` walks `node.parent`).
#[test]
fn both_fragments_still_reach_the_split_inline_through_the_element_tree() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let link = el(&mut doc, c, "a", "");
    doc.set_attribute(link, "data-rid", "7");
    let text = txt(&mut doc, link, "text");
    let blk = el(&mut doc, link, "div", BLK);
    txt(&mut doc, blk, "block");
    let tail = txt(&mut doc, link, "tail");
    doc.resolve_layout(VW, VH);

    let reaches_link = |start: NodeId| {
        let mut cur = Some(start.0);
        while let Some(id) = cur {
            if id == link.0 {
                return true;
            }
            cur = doc.tree.get(id).and_then(|n| n.parent);
        }
        false
    };
    for (what, id) in [("text", text), ("the block", blk), ("tail", tail)] {
        assert!(
            reaches_link(id),
            "{what} must still reach the <a> by walking `parent` — that is how a \
             `data-rid` handler is dispatched, and the split must not orphan it"
        );
    }
    assert_eq!(
        doc.tree.get(link.0).unwrap().attributes.get("data-rid"),
        Some(&"7".to_string()),
        "and the attribute is still on the element the walk finds"
    );
}

/// **Entering and leaving the shape at runtime** — the only fixture that
/// exercises the restore path, and the failure mode this region has historically
/// had.
///
/// Splitting takes the element's boxes out of its own Taffy child list and into
/// its container's. Nothing else puts them back, so an element that *stops* being
/// split would be left with a parentless, childless Taffy node and a container
/// whose list does not name it — the missing direction #597 had to add for the
/// marking pass and #520 for `display: contents`, arrived at a third time.
/// `restore_split_inlines` is that direction, and only this shape reaches it:
/// every other transition fixture crosses by *restyling*, which goes through the
/// anonymous-box cleanup instead.
///
/// Each `resolve_layout` uses a **changed viewport**. `resolve_layout`
/// early-returns on `!layout_dirty` before the IFC block, so a repeat at the same
/// size would run neither the split nor the restore, and the fixture would assert
/// its way through a pass that never happened.
#[test]
fn a_block_appended_into_an_inline_and_removed_again_returns_to_the_start() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let link = el(&mut doc, c, "a", "");
    txt(&mut doc, link, "text");
    txt(&mut doc, link, "tail");
    doc.resolve_layout(VW, VH);

    assert!(!doc.tree.get(link.0).unwrap().is_split_inline());
    assert_eq!(
        height_of(&doc, c),
        LINE,
        "precondition: `text tail` share one line"
    );
    assert_consistent(&doc, "before the block arrives");

    // Enter the shape: append a block between the two text runs.
    let blk = doc.create_element("div");
    doc.set_attribute(blk, "style", BLK);
    let label = doc.create_text("block");
    doc.append_child(blk, label);
    let tail = NodeId(doc.tree.get(link.0).unwrap().children[1]);
    doc.insert_before(link, blk, tail);
    doc.resolve_layout(VW - 1.0, VH);

    assert!(
        doc.tree.get(link.0).unwrap().is_split_inline(),
        "appending a block must split the <a>"
    );
    assert_eq!(
        height_of(&doc, c),
        LINE + BLK_H + LINE,
        "…and the container becomes three bands"
    );
    assert_eq!(
        abs_y(&doc, blk),
        LINE,
        "the block sits after the first line"
    );
    assert_consistent(&doc, "after the block arrives");

    // Leave it again.
    doc.remove_child(link, blk);
    doc.resolve_layout(VW - 2.0, VH);

    assert!(
        !doc.tree.get(link.0).unwrap().is_split_inline(),
        "removing the block must un-split the <a>"
    );
    assert_eq!(
        height_of(&doc, c),
        LINE,
        "…and the container returns to one line. If the restore is missing, the \
         <a>'s Taffy node is parentless and childless here and this collapses to 0."
    );
    assert_consistent(&doc, "after the block is removed");

    // And it is genuinely ordinary inline content again, not merely the right
    // height.
    //
    // **Not "it has a Taffy parent again"** — it must not. An inline child of an
    // IFC root is detached from Taffy on purpose, because Parley lays it out; the
    // container is a root again now, so the marking pass takes the `<a>` straight
    // back out. `restore_split_inlines` puts the boxes back and the marking pass
    // re-detaches exactly the ones that are inline content, which is the same
    // handoff `reattach_departed_ifc_children` is documented to rely on. The
    // observable claim is the mark, and `assert_consistent` above is what says
    // nothing was stranded on the way.
    assert_eq!(
        doc.tree.get(link.0).unwrap().ifc_root,
        Some(c.0),
        "the un-split <a> is inline content of its container's IFC again"
    );
    let text_child = doc.tree.get(link.0).unwrap().children[0];
    assert_eq!(
        doc.tree.get(text_child).unwrap().ifc_root,
        Some(c.0),
        "…and so is its text, laid out by the container rather than by an \
         anonymous box"
    );
    assert!(
        doc.tree.get(text_child).unwrap().run_box.is_none(),
        "the anonymous box the split minted is gone, and its member no longer \
         points at one"
    );
}

/// **A block-only split inline restyled to a block container rejoins its
/// parent** — the shape that makes `restore_split_inlines` load-bearing, and the
/// only one that does.
///
/// `<a><div>card</div></a>` mints no anonymous box on either side of the
/// crossing: split, because it holds no inline content; un-split as a block
/// container, because it holds no inline content *then* either. So neither
/// `cleanup_anonymous_block_boxes` nor `create_anonymous_block_boxes` rebuilds
/// anything, and the restore pass is the only thing standing between this and a
/// stranded subtree.
///
/// **It was measured to fail in two different ways**, which is why this fixture
/// exists rather than a comment. With the restore pass deleted, the mutant
/// survived the whole suite. With the restore pass rebuilding only the element
/// and not its container — its first form — the element's rebuild adopted the
/// card back out of the container, leaving the container's Taffy list **empty**,
/// the element's node still parentless, `h = 0`, and a fatal `C orphan` reachable
/// by an ordinary restyle.
///
/// Both directions of the crossing are checked, and `display: flex` as well as
/// `display: block`, because a flex container is not a block container and so is
/// skipped by a different set of passes on the way back.
#[test]
fn a_block_only_split_inline_restyled_to_a_block_container_rejoins_its_parent() {
    for display in ["block", "flex"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let link = el(&mut doc, c, "a", "");
        let card = el(&mut doc, link, "div", BLK);
        txt(&mut doc, card, "card");
        doc.resolve_layout(VW, VH);

        assert!(
            doc.tree.get(link.0).unwrap().is_split_inline(),
            "{display}: precondition — split, with no run minted"
        );
        assert_eq!(height_of(&doc, c), BLK_H, "{display}: precondition");
        assert_consistent(&doc, "split, block-only");

        // Changed viewport: `resolve_layout` early-returns on `!layout_dirty`
        // before the IFC block, so a repeat at the same size tests nothing.
        doc.set_attribute(link, "style", &format!("display: {display}"));
        doc.resolve_layout(VW - 1.0, VH);

        assert!(!doc.tree.get(link.0).unwrap().is_split_inline());
        assert_eq!(
            height_of(&doc, c),
            BLK_H,
            "{display}: the card is still laid out. 0 here means the <a> came \
             back holding the card while itself being an orphan — which is what \
             rebuilding the element without its container does."
        );
        assert_consistent(&doc, "after the crossing");

        // And back, so a repair that heals once does not pass.
        doc.set_attribute(link, "style", "display: inline");
        doc.resolve_layout(VW - 2.0, VH);
        assert!(
            doc.tree.get(link.0).unwrap().is_split_inline(),
            "{display}: split again on the way back"
        );
        assert_eq!(height_of(&doc, c), BLK_H, "{display}: and still 30px");
        assert_consistent(&doc, "after crossing back");
    }
}

// ── The local pixel oracle ───────────────────────────────────────────────
//
// Layout numbers say a box exists. Only pixels say it is drawn — and this
// repo has measured that a whole-screen similarity score cannot see a defect
// this size. So each fixture asserts on a region where the correct output is
// provably non-empty and the broken output is provably empty.

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    fn pixels(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
        pixels_at(doc, 1.0)
    }

    /// The same, at an arbitrary device scale. Every other pixel assertion in
    /// this file runs at 1.0, which is the whole workspace's blind spot for a
    /// coordinate hoisted out of a `* scale`.
    fn pixels_at(doc: &mut RinchDocument, scale: f32) -> Vec<[u8; 4]> {
        let (w, h) = ((VW * scale) as u32, (VH * scale) as u32);
        let mut painter = TinySkiaPainter::new(w, h);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            scale.into(),
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        painter.pixels().as_chunks::<4>().0.to_vec()
    }

    /// How many pixels the block child's own magenta covers. The block is
    /// 40x30 = 1200px of `rgb(255, 0, 255)`, minus whatever its own text draws
    /// over — so "drawn" is a number in the high hundreds and "not drawn" is
    /// exactly 0. No threshold to tune.
    fn magenta(doc: &mut RinchDocument) -> usize {
        pixels(doc)
            .iter()
            .filter(|p| p[3] > 0 && p[0] > 200 && p[1] < 60 && p[2] > 200)
            .count()
    }

    /// Ink (anything not the white page) in the 20px band starting at `y`.
    fn ink_in_band(doc: &mut RinchDocument, band: usize) -> usize {
        let px = pixels(doc);
        let y0 = band * LINE as usize;
        (y0..y0 + LINE as usize)
            .flat_map(|y| {
                let row = y * VW as usize;
                px[row..row + VW as usize].iter()
            })
            .filter(|p| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
            .count()
    }

    /// Ink per 20px band, six bands deep — the whole visible output of a
    /// container as a comparable profile.
    fn band_profile(doc: &mut RinchDocument) -> Vec<usize> {
        (0..6).map(|b| ink_in_band(doc, b)).collect()
    }

    /// **The strongest form of the oracle.** A bare inline wrapper must change
    /// no pixel anywhere, which is exactly what Chrome does — and the technique
    /// is exact rather than approximate, measured on a shape rinch already
    /// handles: `text <span>mid</span> tail` beside a block gives
    /// `[176, 805, 474, 98, 0, 0]`, and `display: contents` and no wrapper at
    /// all give the same six numbers to the pixel.
    fn assert_wrapper_changes_no_pixel(shape: &str, tag: &str) {
        let mut w = build(shape, true, tag);
        let mut p = build(shape, false, tag);
        let (wb, pb) = (band_profile(&mut w.doc), band_profile(&mut p.doc));
        assert!(
            pb.iter().sum::<usize>() > 0,
            "control ({shape}, no wrapper) drew nothing at all — the fixture is broken",
        );
        assert_eq!(
            wb, pb,
            "{shape} in <{tag}>: a bare inline wrapper must change no pixel. \
             wrapped={wb:?} unwrapped={pb:?}",
        );
    }

    /// Every shape in this file, in pixels.
    #[test]
    fn an_inline_wrapper_changes_no_pixel_in_any_shape() {
        for shape in [
            "middle",
            "first",
            "last",
            "only",
            "nested",
            "sibling_after",
            "two_blocks",
        ] {
            assert_wrapper_changes_no_pixel(shape, "span");
        }
    }

    /// **An out-of-flow child of an inline element is drawn nowhere either.**
    ///
    /// This is not what #513's title says, and it was found by this fixture
    /// rather than assumed: the *heights* agree (20px wrapped and unwrapped,
    /// which is also Chrome's answer), so every layout assertion in this file
    /// passes — and the pixels are `[348, 0, 0, 0, 0, 0]` against the twin's
    /// `[348, 805, 400, 0, 0, 0]`. The absolute's own 40x30 box is simply gone.
    ///
    /// Measured mechanism, by probe rather than by reading: the absolute keeps
    /// its Taffy node parented to the `<span>`, and the `<span>`'s Taffy node is
    /// what `mark_inline_descendants` removes from the container. The subtree
    /// still exists; nothing lays it out.
    ///
    /// **#513's fix does not close this, deliberately, and that is the whole
    /// reason it stayed a separate issue.** The two are one *symptom* — a
    /// box-generating descendant of a detached inline is orphaned from Taffy —
    /// and they need opposite treatments: CSS 2.1 §9.2.1.1 splits an inline
    /// around an in-flow block, and §9.4.2 says an out-of-flow box neither
    /// breaks an inline formatting context nor forces anonymous-box generation.
    /// Splitting around one would encode a rule that is simply wrong. Measured
    /// after the fix: this shape's `<a>` reports `is_split_inline() == false` and
    /// the profile is still `[348, 0, 0, 0, 0, 0]`.
    ///
    /// (An out-of-flow child of an inline that *also* holds an in-flow block is
    /// a different story and does now get laid out, because the split makes it a
    /// unit of the container —
    /// `ifc_classifier_tests::mark_and_walk_agree_on_all_three_cases_of_the_rule`
    /// measures that. It does not generalise to this shape and is not claimed
    /// to.)
    #[test]
    #[ignore = "#591: an out-of-flow child of an inline is orphaned from Taffy and \
           drawn nowhere — CSS does not split an inline around one, so #513\x27s \
           fix deliberately leaves this shape alone"]
    fn an_inline_wrapper_around_an_out_of_flow_child_changes_no_pixel() {
        assert_wrapper_changes_no_pixel("abs", "span");
    }

    /// The headline of #513, in pixels: a 30px magenta block inside an `<a>`
    /// paints **nowhere**. The unwrapped twin paints it, so the number is not a
    /// property of the painter or of the colour filter.
    #[test]
    fn the_block_inside_an_inline_is_actually_drawn() {
        let mut p = build("middle", false, "a");
        let control = magenta(&mut p.doc);
        assert!(
            control > 500,
            "control: the unwrapped block must paint ({control} px)"
        );

        let mut w = build("middle", true, "a");
        assert_eq!(
            magenta(&mut w.doc),
            control,
            "the block inside an <a> must paint exactly what it paints without the <a>",
        );
    }

    /// And the text after it. Band 2 (y 40..60) is where the unwrapped twin
    /// draws `tail`; with the wrapper it is empty.
    #[test]
    fn the_text_after_the_block_is_actually_drawn() {
        let mut p = build("middle", false, "span");
        let control = ink_in_band(&mut p.doc, 2);
        assert!(
            control > 0,
            "control: the unwrapped tail must draw ink in band 2"
        );

        let mut w = build("middle", true, "span");
        assert_eq!(
            ink_in_band(&mut w.doc, 2),
            control,
            "`tail` must be drawn on the line after the block, exactly as it is \
             without the wrapper",
        );
    }

    /// **A split inline's own background paints on both fragments.**
    ///
    /// This is a **regression this change would otherwise introduce**, not a gap
    /// it inherits, which is why it is pinned rather than filed. Measured on
    /// `main` before the split: `<a style="background: cyan">text<div/>tail</a>`
    /// painted **436** cyan pixels — over `text` only, because the old walk broke
    /// at the block — and the unsplit twin paints **761**. With the split and no
    /// bridge it painted **0**: the element is flattened out of the box tree, so
    /// the arm that records an inline box's background never runs for it.
    ///
    /// Equality with the twin is deliberately **not** asserted. The fragments sit
    /// on different lines from the twin's single line, so the covered area
    /// genuinely differs and Chrome's would too. What is asserted is that both
    /// fragments are painted, which is what "nothing" failed and what a
    /// one-rectangle-spanning-everything fix would also fail (band 1 would be
    /// cyan, and it must not be — the block is opaque over it).
    #[test]
    fn a_split_inlines_background_paints_on_both_fragments() {
        const BG: &str = "background: rgb(0, 200, 200)";
        fn cyan_in_band(doc: &mut RinchDocument, band: usize) -> usize {
            let px = pixels(doc);
            let y0 = band * LINE as usize;
            (y0..y0 + LINE as usize)
                .flat_map(|y| {
                    let row = y * VW as usize;
                    px[row..row + VW as usize].iter()
                })
                .filter(|p| p[3] > 0 && p[0] < 80 && p[1] > 180 && p[2] > 180)
                .count()
        }
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let link = el(&mut doc, c, "a", BG);
        txt(&mut doc, link, "text");
        let blk = el(&mut doc, link, "div", BLK);
        txt(&mut doc, blk, "block");
        txt(&mut doc, link, "tail");
        doc.resolve_layout(VW, VH);

        assert!(
            cyan_in_band(&mut doc, 0) > 0,
            "the leading fragment must carry the <a>'s background"
        );
        assert!(
            cyan_in_band(&mut doc, 2) > 0,
            "and so must the trailing fragment — 0 here is the regression the \
             bridge exists to prevent"
        );
        assert_eq!(
            cyan_in_band(&mut doc, 1),
            0,
            "but not the block's own band: the fragments are two rectangles, not \
             one spanning the whole thing"
        );
    }

    /// **One rectangle per fragment, not one per member.**
    ///
    /// The bridge emits a background span per split-inline ancestor per
    /// *contiguous stretch* of the run. Emitting one per **member** instead puts
    /// two abutting spans where one fragment belongs — and the mutant that
    /// collapses the stretch tracking survived the whole suite, because the only
    /// background fixture sat on the fixed point where the two spellings agree:
    /// with zero padding they paint identical pixels, and two members are needed
    /// before "per member" and "per stretch" can differ at all.
    ///
    /// **Asserted as a count rather than in pixels, deliberately.** Two abutting
    /// spans differ from one only inside the padding at the seam, which is a
    /// handful of pixels sharing an edge with anti-aliased glyphs — measured, the
    /// pixel form picks up a 26-shade gradient of text AA and cannot separate the
    /// two spellings without a font-dependent threshold. The count is exact, and
    /// "how many rectangles does this fragment paint" is the property itself, not
    /// a proxy for it. Same idiom as `ifc_classifier_tests::flowed_by`, which
    /// reads `text_ranges` off the same struct for the same reason.
    ///
    /// Padding is declared anyway: with it, a wrong count is also a visibly
    /// double-composited seam, so the fixture stays honest if someone later
    /// replaces the count with an oracle that can see it.
    #[test]
    fn a_fragment_is_one_background_rectangle_not_one_per_member() {
        const BG: &str = "background: rgba(0, 0, 255, 0.5); padding-left: 6px; \
                          padding-right: 6px";
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let link = el(&mut doc, c, "a", BG);
        // TWO members in the leading fragment — the arity that discriminates.
        txt(&mut doc, link, "one ");
        let em = el(&mut doc, link, "i", "");
        txt(&mut doc, em, "two");
        let blk = el(&mut doc, link, "div", BLK);
        txt(&mut doc, blk, "block");
        txt(&mut doc, link, "tail");
        doc.resolve_layout(VW, VH);

        let mut counts: Vec<usize> = doc
            .tree
            .nodes
            .iter()
            .filter(|(_, n)| n.is_anonymous_block_box)
            .filter_map(|(_, n)| n.text_layout.as_ref().map(|l| l.background_spans.len()))
            .collect();
        counts.sort_unstable();
        assert_eq!(
            counts,
            vec![1, 1],
            "two fragments, one background rectangle each. `[2, 1]` means the \
             leading fragment's two members each got their own span and the \
             padding at the seam is painted twice."
        );
    }

    /// **An inherited text style crosses the split.** `color` on the `<a>` must
    /// reach both fragments, which it can only do through the style bridge — the
    /// element itself is never walked, so nothing pushes its span.
    #[test]
    fn an_inherited_color_reaches_both_fragments() {
        fn green_in_band(doc: &mut RinchDocument, band: usize) -> usize {
            let px = pixels(doc);
            let y0 = band * LINE as usize;
            (y0..y0 + LINE as usize)
                .flat_map(|y| {
                    let row = y * VW as usize;
                    px[row..row + VW as usize].iter()
                })
                .filter(|p| p[3] > 0 && p[0] < 90 && p[1] > 100 && p[1] < 210 && p[2] < 90)
                .count()
        }
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let link = el(&mut doc, c, "a", "color: rgb(0, 128, 0)");
        txt(&mut doc, link, "text");
        let blk = el(&mut doc, link, "div", "width: 40px; height: 30px");
        txt(&mut doc, blk, "block");
        txt(&mut doc, link, "tail");
        doc.resolve_layout(VW, VH);

        assert!(
            green_in_band(&mut doc, 0) > 0 && green_in_band(&mut doc, 2) > 0,
            "both fragments must render in the <a>'s colour: band 0 = {}, \
             band 2 = {}",
            green_in_band(&mut doc, 0),
            green_in_band(&mut doc, 2),
        );
    }

    /// **The twin property holds at a scale other than 1.0.**
    ///
    /// Every other pixel assertion in this file, and in the workspace, runs at
    /// 1.0 — where an offset hoisted out of a `* scale` is invisible. The profile
    /// is compared at 2.0 against the wrapper-free twin at 2.0, so a scale bug
    /// that affects the split and not the twin fails here.
    #[test]
    fn the_wrapper_changes_no_pixel_at_scale_two() {
        const S: f32 = 2.0;
        fn profile_at(doc: &mut RinchDocument, scale: f32) -> Vec<usize> {
            let px = pixels_at(doc, scale);
            let w = (VW * scale) as usize;
            let band = (LINE * scale) as usize;
            (0..6)
                .map(|b| {
                    let y0 = b * band;
                    (y0..y0 + band)
                        .flat_map(|y| px[y * w..y * w + w].iter())
                        .filter(|p| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
                        .count()
                })
                .collect()
        }
        let mut w = build("middle", true, "span");
        let mut p = build("middle", false, "span");
        let (wb, pb) = (profile_at(&mut w.doc, S), profile_at(&mut p.doc, S));
        assert!(
            pb.iter().sum::<usize>() > 0,
            "control drew nothing at scale {S} — the fixture is broken"
        );
        assert_eq!(
            wb, pb,
            "at scale {S} a bare inline wrapper must still change no pixel. \
             wrapped={wb:?} unwrapped={pb:?}",
        );
    }

    // There is deliberately no live test here of the form "an inline with an
    // out-of-flow child draws nothing on the second line". It passes today —
    // and it passes **for the wrong reason**, because the absolute is drawn
    // nowhere at all. That is this project's recurring test failure (a fixture
    // sitting where correct and broken agree), and the profile comparison above
    // is the version that discriminates.
}
