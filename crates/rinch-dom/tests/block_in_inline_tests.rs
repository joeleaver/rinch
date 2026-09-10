//! #513 — block-level content inside an inline element, and everything after it.
//!
//! ```html
//! <div><a>text<div>block</div>tail</a></div>
//! ```
//!
//! rinch draws `text` and nothing else. A browser draws all three, by splitting
//! the inline box around the block-level child (CSS 2.1 §9.2.1.1, "block-in-inline").
//!
//! This file is the **falsifier**, not the fix: it pins what the defect is, what
//! a browser actually does, and — just as important — the neighbouring shapes a
//! candidate fix must *not* change. It was written before any fix existed, on
//! purpose, so it cannot be shaped by one.
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
//! An **out-of-flow** child does not split an inline in CSS, and rinch already
//! agrees (#406 classifies it `OutOfFlow`, which neither joins a run nor ends
//! one). Chrome: `<a>text<abs>tail</a>` is one 20px line with `tail` beside
//! `text`. Those fixtures are **not** `#[ignore]`d — they pass today, and a fix
//! that starts minting anonymous boxes inside inline containers has to keep them
//! passing.

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
    // The host every child is appended to: the inline wrapper, or the container.
    let h = if wrapped {
        el(&mut doc, c, wrapper_tag, "")
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
            // The block is two inlines deep when wrapped. Unwrapped, the same
            // content with both wrappers deleted — which is what Chrome's twin
            // shows is geometrically identical.
            txt(&mut doc, h, "out");
            let inner = if wrapped { el(&mut doc, h, "b", "") } else { h };
            txt(&mut doc, inner, "in");
            blocks.push(block(&mut doc, inner, "block", BLK));
            txt(&mut doc, inner, "after");
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
// acceptance run for a fix. Each message carries the browser's number.

/// The canonical markup from #513. Chrome: H=70 — one line, the 30px block, one
/// line. rinch: H=20, and the block and `tail` are drawn nowhere.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn a_block_inside_an_inline_lays_out_as_if_the_inline_were_not_there() {
    assert_wrapper_changes_nothing("middle", "a", LINE + BLK_H + LINE);
}

/// The same shape in a `<span>`, so a fix cannot be an `<a>`-specific accident.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn the_wrapper_being_a_span_rather_than_a_link_changes_nothing() {
    assert_wrapper_changes_nothing("middle", "span", LINE + BLK_H + LINE);
}

/// Block **first**: Chrome H=50, and there is no empty line box before it.
/// Off the fixed point of "middle" — a fix that always emits a leading run
/// would give 70.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn a_block_first_inside_an_inline_costs_no_leading_line() {
    assert_wrapper_changes_nothing("first", "a", BLK_H + LINE);
}

/// Block **last**: Chrome H=50, no trailing empty line.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn a_block_last_inside_an_inline_costs_no_trailing_line() {
    assert_wrapper_changes_nothing("last", "a", LINE + BLK_H);
}

/// A block as the inline's **only** child: Chrome H=30. Arity 0 for the run
/// grouping — no run exists at all, and the empty inline contributes no line.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn an_inline_holding_only_a_block_is_exactly_that_block_tall() {
    assert_wrapper_changes_nothing("only", "a", BLK_H);
}

/// The block is two inlines deep. Chrome splits **both** — `<a>` reports five
/// fragments and `<b>` three — and the result is still identical to the
/// wrapper-free twin.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn a_block_two_inlines_deep_splits_every_inline_above_it() {
    assert_wrapper_changes_nothing("nested", "a", LINE + BLK_H + LINE);
}

/// "and so does everything after it" — the sibling here is a child of the
/// **container**, after the inline, and Chrome puts it on the last line beside
/// `tail`.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
fn content_after_the_inline_survives_the_block_inside_it() {
    assert_wrapper_changes_nothing("sibling_after", "a", LINE + BLK_H + LINE);
}

/// Two blocks, three runs: Chrome H=120. One block is where "split once" and
/// "split at every block" agree.
#[test]
#[ignore = "#513: block-in-inline is not implemented; rinch stops at the block"]
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
#[ignore = "#513: an inline-block does not generate anonymous block boxes either"]
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
    assert_consistent(&w.doc, "absolute inside an inline");
}

/// rinch implements **no floats** (there is no `float` handling anywhere in
/// `rinch-dom`), so a floated child is laid out as an ordinary in-flow block.
/// That happens to give the browser's container height here — Chrome also says
/// 20px, because the float is out of flow — but by a different mechanism.
///
/// It also passes for the wrong reason in a second way: today the floated
/// child, and `tail` after it, are drawn nowhere at all, so 20px is what is
/// left rather than what was computed.
///
/// It is pinned live anyway, and the reason is falsification, not correctness:
/// a fix that starts treating an inline as a block container will classify this
/// float `InFlowBlock`, mint runs around it, and take the container to 70px —
/// three times the browser's answer. If that happens this test says so at the
/// moment it happens.
#[test]
fn a_floated_child_of_an_inline_does_not_add_lines_to_its_container() {
    let w = build("float", true, "a");
    assert_eq!(
        height_of(&w.doc, w.container),
        LINE,
        "Chrome renders this as one 20px line (the float is out of flow). rinch \
         reaches the same number without implementing floats; a fix that splits \
         the inline around it would give {}",
        LINE + BLK_H + LINE,
    );
    assert_consistent(&w.doc, "float inside an inline");
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
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
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
    #[ignore = "#513: block-in-inline is not implemented; the wrapper eats pixels"]
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
    /// still exists; nothing lays it out. So the block case and the out-of-flow
    /// case are one defect — **a box-generating descendant of a detached inline
    /// is orphaned from Taffy** — and the block case is the half #513 names.
    #[test]
    #[ignore = "#513: an out-of-flow child of an inline is orphaned from Taffy and drawn nowhere"]
    fn an_inline_wrapper_around_an_out_of_flow_child_changes_no_pixel() {
        assert_wrapper_changes_no_pixel("abs", "span");
    }

    /// The headline of #513, in pixels: a 30px magenta block inside an `<a>`
    /// paints **nowhere**. The unwrapped twin paints it, so the number is not a
    /// property of the painter or of the colour filter.
    #[test]
    #[ignore = "#513: the block inside an inline is drawn nowhere"]
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
    #[ignore = "#513: everything after the block inside an inline is drawn nowhere"]
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

    // There is deliberately no live test here of the form "an inline with an
    // out-of-flow child draws nothing on the second line". It passes today —
    // and it passes **for the wrong reason**, because the absolute is drawn
    // nowhere at all. That is this project's recurring test failure (a fixture
    // sitting where correct and broken agree), and the profile comparison above
    // is the version that discriminates.
}
