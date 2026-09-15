//! Issue #674 — the rest of the browser's UA defaults: `<hr>`, block margins,
//! `<pre>`'s preserved whitespace, `smaller` on `<small>`/`<sub>`/`<sup>`, and
//! monospace on `<code>`/`<kbd>`/`<samp>`/`<pre>`.
//!
//! #627 (PR #673) added the heading and `<th>` rules and stopped there. This is
//! the stylesheet half of what it left: rinch's UA sheet gave `<hr>` no border
//! at all (the sheet's own `* { border-width: 0 }` reset, there to undo Stylo's
//! `medium` initial, applied to `<hr>` too and nothing put it back), gave no
//! block element any margin, left `<pre>` at `white-space: normal`, left
//! `<small>` at its parent's size, and named no monospace family. `rinch-web`
//! runs on the browser's own UA sheet and did all five, so this is the
//! desktop/web divergence class this project treats as a defect.
//!
//! **Every expected number is measured in Chrome 150**, `getComputedStyle` on a
//! standards-mode page with a 16px root:
//!
//! | element | `margin-block` | `margin-inline` | other |
//! |---------|----------------|-----------------|-------|
//! | `p`, `ul`, `ol` | 1em | 0 | |
//! | `blockquote`, `figure` | 1em | 40px | |
//! | `pre` | 1em | 0 | `white-space: pre`, monospace |
//! | `dd` | 0 | 40px start only | |
//! | `ul`/`ol` inside `ul`/`ol` | **0** | | descendant, not child |
//! | `hr` | 0.5em | auto | 1px inset border, `color: gray`, `height: 0`, `overflow: hidden` |
//! | `small`, `sub`, `sup` | | | `font-size: smaller` → 13.3333px from 16, 16.6667px from 20 |
//! | `code`, `kbd`, `samp` | | | monospace |
//!
//! ## The fixed points this file deliberately samples off
//!
//! - **1em at a 16px root is 16px, and so is a literal `16px`.** Every margin
//!   fixture here builds its tree inside a `font-size: 20px` container and
//!   requires 20, so a rule spelled in px cannot pass.
//! - **`hr { border-color: gray }` and `hr { color: gray }` agree on every
//!   `<hr>` that does not declare a `color`.** Chrome's rule is the second one —
//!   the border is `currentcolor` — and
//!   `an_authored_color_repaints_the_hr_border` is the only fixture that can
//!   tell them apart.
//! - **A single block's margins are indistinguishable from an uncollapsed
//!   pair.** `adjacent_block_margins_collapse_to_one_em` needs two siblings and
//!   requires the *max*, not the sum.
//! - **`0.83em` and `smaller` differ by 0.05px at a 16px root**, which is under
//!   most tolerances. `small_sub_and_sup_take_the_smaller_keyword` requires
//!   Chrome's 13.3333/16.6667 to 0.005px and checks the nested compounding,
//!   where the two spellings diverge by 0.09px.
//!
//! `<hr>`'s paint — that the border row is real ink and not merely a computed
//! value — lives in `ua_hr_paint_tests.rs`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::values::{
    BorderStyleValue, DimensionValue, LengthPercentageAutoValue, LengthPercentageValue,
    OverflowValue, WhiteSpaceValue,
};

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// Chrome's own root size, so "1em" and "20px" are different numbers.
const ROOT: &str = "width: 800px; font-size: 20px";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let id = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(id, "style", style);
    }
    doc.append_child(parent, id);
    id
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// The four resolved margins in px, in `(top, bottom, left, right)` order.
///
/// A `Percent` or `Auto` where a length is expected means the declaration never
/// reached layout at all, which is a different failure from the wrong number
/// and is reported as such. `Auto` is legitimate for `<hr>`'s inline margins,
/// so `hr_inline_margins_are_auto` reads those through [`raw_inline_margins`].
fn margins(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let s = &doc.tree.get(id.0).unwrap().computed_style;
    let px = |m: &LengthPercentageAutoValue, which: &str| match m {
        LengthPercentageAutoValue::Length(v) => *v,
        other => panic!("{which} margin did not resolve to a length: {other:?}"),
    };
    (
        px(&s.margin_top, "top"),
        px(&s.margin_bottom, "bottom"),
        px(&s.margin_left, "left"),
        px(&s.margin_right, "right"),
    )
}

/// The block margins only, for an element whose inline margins are legitimately
/// `auto` (which is every `<hr>`).
fn block_margins(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let (top, bottom, _, _) = {
        let s = &doc.tree.get(id.0).unwrap().computed_style;
        let px = |m: &LengthPercentageAutoValue, which: &str| match m {
            LengthPercentageAutoValue::Length(v) => *v,
            other => panic!("{which} margin did not resolve to a length: {other:?}"),
        };
        (
            px(&s.margin_top, "top"),
            px(&s.margin_bottom, "bottom"),
            0.0,
            0.0,
        )
    };
    (top, bottom)
}

fn raw_inline_margins(
    doc: &RinchDocument,
    id: NodeId,
) -> (LengthPercentageAutoValue, LengthPercentageAutoValue) {
    let s = &doc.tree.get(id.0).unwrap().computed_style;
    (s.margin_left, s.margin_right)
}

fn border_width(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let s = &doc.tree.get(id.0).unwrap().computed_style;
    let px = |w: &LengthPercentageValue| match w {
        LengthPercentageValue::Length(v) => *v,
        other => panic!("border width did not resolve to a length: {other:?}"),
    };
    (
        px(&s.border_top_width),
        px(&s.border_bottom_width),
        px(&s.border_left_width),
        px(&s.border_right_width),
    )
}

fn font_size(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.font_size
}

fn font_family(doc: &RinchDocument, id: NodeId) -> String {
    doc.tree
        .get(id.0)
        .unwrap()
        .computed_style
        .font_family
        .clone()
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

fn top_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.y
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

// ===== §2 — block margins =====

/// Chrome's `margin-block: 1em` on every block element that has one, read at a
/// 20px font size so `1em` cannot be confused with a literal 16px.
///
/// This is the headline fixture and it fails at HEAD: every one of these read 0.
#[test]
fn block_elements_carry_the_browser_ua_block_margins() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let tags = ["p", "blockquote", "figure", "ul", "ol", "pre"];
    let ids: Vec<NodeId> = tags.iter().map(|t| el(&mut doc, c, t, "")).collect();
    doc.resolve_layout(VW, VH);

    for (id, tag) in ids.iter().zip(tags.iter()) {
        let (top, bottom, _, _) = margins(&doc, *id);
        assert!(
            close(top, 20.0) && close(bottom, 20.0),
            "<{tag}> block margins: expected Chrome's 1em = 20px, got ({top}, {bottom})"
        );
    }
}

/// `blockquote` and `figure` are also indented 40px on both sides; `dd` only on
/// its start side; `p`/`ul`/`ol`/`pre` not at all.
///
/// The 40px is a fixed length in Chrome, not an `em` — it does **not** scale
/// with the 20px container, which is why the expectation is 40 and not 50.
#[test]
fn the_inline_indents_are_forty_fixed_pixels_on_the_right_elements() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let bq = el(&mut doc, c, "blockquote", "");
    let fig = el(&mut doc, c, "figure", "");
    let dl = el(&mut doc, c, "dl", "");
    let dt = el(&mut doc, c, "dt", "");
    doc.append_child(dl, dt);
    let dd = el(&mut doc, c, "dd", "");
    doc.append_child(dl, dd);
    let p = el(&mut doc, c, "p", "");
    doc.resolve_layout(VW, VH);

    for (id, tag) in [(bq, "blockquote"), (fig, "figure")] {
        let (_, _, left, right) = margins(&doc, id);
        assert!(
            close(left, 40.0) && close(right, 40.0),
            "<{tag}> inline margins: expected Chrome's 40px both sides, got ({left}, {right})"
        );
    }

    // `dd` is start-side only, and carries no block margin at all.
    let (top, bottom, left, right) = margins(&doc, dd);
    assert!(
        close(left, 40.0) && close(right, 0.0),
        "<dd>: expected Chrome's 40px start / 0 end, got ({left}, {right})"
    );
    assert!(
        close(top, 0.0) && close(bottom, 0.0),
        "<dd> takes no block margin in Chrome, got ({top}, {bottom})"
    );

    // `<dt>` takes nothing at all — the discriminator against a rule that named
    // the whole `dl` family.
    assert_eq!(
        margins(&doc, dt),
        (0.0, 0.0, 0.0, 0.0),
        "<dt> has no UA margin in Chrome"
    );

    // And a `<p>` is not indented, which is what separates the two rules.
    let (_, _, left, right) = margins(&doc, p);
    assert!(
        close(left, 0.0) && close(right, 0.0),
        "<p> must not take the blockquote indent, got ({left}, {right})"
    );
}

/// A list nested inside a list carries **no** block margin, and the rule reaches
/// a *grandchild* — the `<ul>` hangs off an `<li>`, not off the outer `<ul>`.
///
/// Kills a mutant that writes the rule with a child combinator (`ul > ul`),
/// which matches nothing in real list markup and leaves 2em of dead space at
/// every nesting level.
#[test]
fn a_list_nested_in_a_list_carries_no_block_margin() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let outer = el(&mut doc, c, "ul", "");
    let li = el(&mut doc, outer, "li", "");
    let inner = el(&mut doc, li, "ul", "");
    let inner_ol = el(&mut doc, li, "ol", "");
    doc.resolve_layout(VW, VH);

    let (top, bottom, _, _) = margins(&doc, outer);
    assert!(
        close(top, 20.0) && close(bottom, 20.0),
        "the outer <ul> keeps its 1em, got ({top}, {bottom})"
    );
    for (id, what) in [(inner, "ul in ul"), (inner_ol, "ol in ul")] {
        let (top, bottom) = {
            let m = margins(&doc, id);
            (m.0, m.1)
        };
        assert!(
            close(top, 0.0) && close(bottom, 0.0),
            "a nested list ({what}) takes no block margin in Chrome, got ({top}, {bottom})"
        );
    }
}

/// An author `margin: 0` beats the new UA rule — the #616/#618 handshake, which
/// is why these are cascade rules and not a post-cascade patch.
#[test]
fn an_author_margin_beats_the_ua_block_margin() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let zeroed = el(&mut doc, c, "p", "margin: 0");
    let bq = el(&mut doc, c, "blockquote", "margin: 0");
    let raised = el(&mut doc, c, "p", "margin-top: 3px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        margins(&doc, zeroed),
        (0.0, 0.0, 0.0, 0.0),
        "an author `margin: 0` must beat the UA `margin-block: 1em`"
    );
    assert_eq!(
        margins(&doc, bq),
        (0.0, 0.0, 0.0, 0.0),
        "an author `margin: 0` must beat the UA inline indent too"
    );
    let (top, bottom, _, _) = margins(&doc, raised);
    assert!(
        close(top, 3.0),
        "an author `margin-top` must win, got {top}"
    );
    assert!(
        close(bottom, 20.0),
        "…while leaving the UA `margin-bottom` in place, got {bottom}"
    );
}

/// Adjacent block margins collapse to the **larger** of the two, as in Chrome:
/// `<p>a</p><p>b</p>` leaves a 1em gap, not 2em, and a `<p>`/`<hr>` pair leaves
/// the `<p>`'s 1em rather than the `<hr>`'s 0.5em or their sum.
///
/// Sampled off the single-element fixed point on purpose: one block's margins
/// look identical whether or not the engine collapses.
#[test]
fn adjacent_block_margins_collapse_to_the_larger() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let a = el(&mut doc, c, "p", "line-height: 20px");
    text(&mut doc, a, "a");
    let b = el(&mut doc, c, "p", "line-height: 20px");
    text(&mut doc, b, "b");
    let rule = el(&mut doc, c, "hr", "");
    let d = el(&mut doc, c, "p", "line-height: 20px");
    text(&mut doc, d, "d");
    doc.resolve_layout(VW, VH);

    let gap = top_of(&doc, b) - (top_of(&doc, a) + height_of(&doc, a));
    assert!(
        close(gap, 20.0),
        "two adjacent <p> collapse to one 1em gap in Chrome (20px here), got {gap}"
    );

    // `<p>`'s 1em against `<hr>`'s 0.5em: the max, not the sum and not the min.
    let to_rule = top_of(&doc, rule) - (top_of(&doc, b) + height_of(&doc, b));
    assert!(
        close(to_rule, 20.0),
        "<p> 1em against <hr> 0.5em collapses to 1em (20px), got {to_rule}"
    );
    let from_rule = top_of(&doc, d) - (top_of(&doc, rule) + height_of(&doc, rule));
    assert!(
        close(from_rule, 20.0),
        "<hr> 0.5em against <p> 1em collapses to 1em (20px), got {from_rule}"
    );
}

/// Flex items do **not** collapse their margins — measured in Chrome, two `<p>`
/// in a `flex-direction: column` sit 2em apart where the same pair in a block
/// container sits 1em apart.
///
/// This is the case a `Stack` of bare `<p>` hits, and it is the counter-fixture
/// to the one above: if rinch ever collapsed unconditionally, that one would
/// still pass and this one would not.
#[test]
fn flex_items_do_not_collapse_their_block_margins() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 800px; font-size: 20px; display: flex; flex-direction: column",
    );
    let a = el(&mut doc, c, "p", "line-height: 20px");
    text(&mut doc, a, "a");
    let b = el(&mut doc, c, "p", "line-height: 20px");
    text(&mut doc, b, "b");
    doc.resolve_layout(VW, VH);

    let gap = top_of(&doc, b) - (top_of(&doc, a) + height_of(&doc, a));
    assert!(
        close(gap, 40.0),
        "flex items keep both margins in Chrome (2 x 20px), got {gap}"
    );
}

// ===== §1 — `<hr>` =====

/// A bare `<hr>` carries Chrome's whole box: 0.5em block margins, a 1px border
/// on every side, `height: 0`, `overflow: hidden` — and therefore a 2px tall
/// box rather than the invisible zero-height one rinch produced.
#[test]
fn a_bare_hr_carries_the_browser_ua_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let rule = el(&mut doc, c, "hr", "");
    doc.resolve_layout(VW, VH);

    let (top, bottom) = block_margins(&doc, rule);
    assert!(
        close(top, 10.0) && close(bottom, 10.0),
        "<hr> block margins: expected Chrome's 0.5em = 10px at 20px, got ({top}, {bottom})"
    );

    let (bt, bb, bl, br) = border_width(&doc, rule);
    assert!(
        close(bt, 1.0) && close(bb, 1.0) && close(bl, 1.0) && close(br, 1.0),
        "<hr> border: expected 1px on all four sides, got ({bt}, {bb}, {bl}, {br})"
    );

    let s = &doc.tree.get(rule.0).unwrap().computed_style;
    assert_eq!(
        s.border_top_style,
        BorderStyleValue::Solid,
        "Chrome's `inset` has no rinch spelling — `border_style_from_stylo` maps \
         groove/ridge/inset/outset to Solid. If this ever becomes `None`, the \
         `border-style` declaration stopped reaching the cascade."
    );
    assert!(
        matches!(s.height, DimensionValue::Length(v) if v == 0.0),
        "<hr> is `height: 0` in Chrome; its 2px comes entirely from the borders, got {:?}",
        s.height
    );
    assert_eq!(s.overflow_x, OverflowValue::Hidden);
    assert_eq!(s.overflow_y, OverflowValue::Hidden);

    // Content 0 + 1px top + 1px bottom.
    assert!(
        close(height_of(&doc, rule), 2.0),
        "<hr> lays out 2px tall in Chrome, got {}",
        height_of(&doc, rule)
    );
}

/// The `<hr>` border is `currentcolor` over a UA `color: gray`, not a UA
/// `border-color`. Declaring a `color` on the element repaints the rule.
///
/// **The one fixture that can tell the two spellings apart** — every other
/// `<hr>` here sits on the fixed point where they agree.
#[test]
fn an_authored_color_repaints_the_hr_border() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let plain = el(&mut doc, c, "hr", "");
    let red = el(&mut doc, c, "hr", "color: rgb(255, 0, 0)");
    doc.resolve_layout(VW, VH);

    let grey = doc
        .tree
        .get(plain.0)
        .unwrap()
        .computed_style
        .border_top_color
        .expect("<hr> must have a resolved border colour");
    let [r, g, b, _] = grey.components;
    assert!(
        close(r, 128.0 / 255.0) && close(g, 128.0 / 255.0) && close(b, 128.0 / 255.0),
        "a bare <hr> is Chrome's gray (128,128,128), got ({r}, {g}, {b})"
    );

    let painted = doc
        .tree
        .get(red.0)
        .unwrap()
        .computed_style
        .border_top_color
        .expect("<hr> must have a resolved border colour");
    let [r, g, b, _] = painted.components;
    assert!(
        close(r, 1.0) && close(g, 0.0) && close(b, 0.0),
        "an author `color` must repaint the <hr> border (it is currentcolor), got ({r}, {g}, {b})"
    );
}

/// `<hr>`'s inline margins are `auto`, which is invisible at full width and
/// centres a narrowed rule — Chrome puts a `width: 100px` `<hr>` in the middle.
#[test]
fn hr_inline_margins_are_auto_and_centre_a_narrowed_rule() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let full = el(&mut doc, c, "hr", "");
    let narrow = el(&mut doc, c, "hr", "width: 100px");
    doc.resolve_layout(VW, VH);

    let (left, right) = raw_inline_margins(&doc, full);
    assert!(
        matches!(left, LengthPercentageAutoValue::Auto)
            && matches!(right, LengthPercentageAutoValue::Auto),
        "<hr> takes `margin-inline: auto` in Chrome, got ({left:?}, {right:?})"
    );

    // 800px container, 100px rule → (800-100)/2. The equivalent Chrome number
    // is 331px of margin over a 764px body, i.e. the same centring around a
    // **102px** box: Chrome adds the 1px side borders outside the declared
    // `width` (content-box), where rinch applies a global `box-sizing:
    // border-box` (Taffy's default, noted in `style_resolution/mod.rs`). That
    // 2px is a pre-existing, workspace-wide difference, not something the
    // `<hr>` rule introduces; the property under test is that the rule centres
    // at all, which it did not before `margin-inline: auto`.
    let n = &doc.tree.get(narrow.0).unwrap().layout;
    assert!(
        close(n.width, 100.0) && close(n.x, 350.0),
        "a narrowed <hr> centres itself: expected a 100px box at x = 350, got {n:?}"
    );
}

/// An author `border: none` beats the UA rule, and `margin: 0` beats its
/// margins — the `Divider` component depends on exactly this, since it is an
/// `<hr class="rinch-divider">` whose CSS declares both.
#[test]
fn an_author_border_and_margin_beat_the_ua_hr_rule() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let rule = el(&mut doc, c, "hr", "border: none; margin: 0");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        border_width(&doc, rule),
        (0.0, 0.0, 0.0, 0.0),
        "an author `border: none` must beat the UA `border-width: 1px`"
    );
    assert_eq!(
        margins(&doc, rule),
        (0.0, 0.0, 0.0, 0.0),
        "an author `margin: 0` must beat both UA margins, the `auto` one included"
    );
}

// ===== §3 — `<pre>` =====

/// A bare `<pre>` preserves its runs of spaces and its newlines: two lines of
/// text where HEAD collapsed them onto one.
///
/// Height, not glyph geometry, is the assertion — and `line-height` is declared
/// so the number is a declaration rather than a local-font measurement.
#[test]
fn a_bare_pre_preserves_its_whitespace() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 20px");
    let preserved = el(&mut doc, c, "pre", "line-height: 20px; margin: 0");
    text(&mut doc, preserved, "a  b\nc");
    let collapsed = el(
        &mut doc,
        c,
        "pre",
        "line-height: 20px; margin: 0; white-space: normal",
    );
    text(&mut doc, collapsed, "a  b\nc");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.tree
            .get(preserved.0)
            .unwrap()
            .computed_style
            .white_space,
        WhiteSpaceValue::Pre,
        "the UA sheet must give <pre> `white-space: pre`"
    );
    assert!(
        close(height_of(&doc, preserved), 40.0),
        "<pre> keeps its newline: two 20px lines, got {}",
        height_of(&doc, preserved)
    );

    // The counter-case, and the author-override fixture in one: declaring
    // `white-space: normal` collapses it back to a single line.
    assert_eq!(
        doc.tree
            .get(collapsed.0)
            .unwrap()
            .computed_style
            .white_space,
        WhiteSpaceValue::Normal,
        "an author `white-space` must beat the UA <pre> rule"
    );
    assert!(
        close(height_of(&doc, collapsed), 20.0),
        "…and collapse the newline back to one line, got {}",
        height_of(&doc, collapsed)
    );
}

// ===== §4 — `font-size: smaller` =====

/// `<small>`, `<sub>` and `<sup>` take Chrome's `smaller`: 13.3333px from a
/// 16px parent and 16.6667px from a 20px one, compounding to 11.1111px when
/// nested.
///
/// The tolerance is deliberately 0.005px. The audit suggested `0.83em`, which
/// gives 13.28 rather than 13.3333 and 11.0224 rather than 11.1111 — inside a
/// 0.01 tolerance at neither level, but inside a 0.1 one at the first.
#[test]
fn small_sub_and_sup_take_the_smaller_keyword() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let at16 = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let s16 = el(&mut doc, at16, "small", "");
    let sub16 = el(&mut doc, at16, "sub", "");
    let sup16 = el(&mut doc, at16, "sup", "");
    let nested = el(&mut doc, s16, "small", "");
    let at20 = el(&mut doc, body, "div", "width: 800px; font-size: 20px");
    let s20 = el(&mut doc, at20, "small", "");
    let authored = el(&mut doc, at20, "small", "font-size: 30px");
    doc.resolve_layout(VW, VH);

    let tight = |a: f32, b: f32| (a - b).abs() < 0.005;
    for (id, tag) in [(s16, "small"), (sub16, "sub"), (sup16, "sup")] {
        let got = font_size(&doc, id);
        assert!(
            tight(got, 13.3333),
            "<{tag}> from a 16px parent is Chrome's 13.3333px, got {got}"
        );
    }
    let got = font_size(&doc, nested);
    assert!(
        tight(got, 11.1111),
        "`smaller` compounds: <small> in <small> is 11.1111px in Chrome, got {got}"
    );
    let got = font_size(&doc, s20);
    assert!(
        tight(got, 16.6667),
        "<small> from a 20px parent is Chrome's 16.6667px, got {got}"
    );
    let got = font_size(&doc, authored);
    assert!(
        close(got, 30.0),
        "an author `font-size` must beat the UA `smaller`, got {got}"
    );
}

// ===== §7 — monospace =====

/// `<code>`, `<kbd>`, `<samp>` and `<pre>` compute a monospace family from the
/// **UA sheet**, so it holds in a build with no `theme` feature. At HEAD a bare
/// `<code>` computed `serif`.
///
/// rinch deliberately does not reproduce Chrome's monospace font-*size* quirk
/// (13px for a `medium` monospace element), so the size is the inherited one.
#[test]
fn the_monospace_family_is_in_the_ua_sheet_not_only_the_theme() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let tags = ["code", "kbd", "samp", "pre"];
    let ids: Vec<NodeId> = tags.iter().map(|t| el(&mut doc, c, t, "")).collect();
    let other = el(&mut doc, c, "span", "");
    let authored = el(&mut doc, c, "code", "font-family: cursive");
    doc.resolve_layout(VW, VH);

    for (id, tag) in ids.iter().zip(tags.iter()) {
        assert_eq!(
            font_family(&doc, *id),
            "monospace",
            "<{tag}> must be monospace without the theme stylesheet"
        );
        assert!(
            close(font_size(&doc, *id), 20.0),
            "<{tag}> keeps the inherited size — rinch has no monospace-size quirk"
        );
    }
    assert_ne!(
        font_family(&doc, other),
        "monospace",
        "the rule must not leak onto every inline element"
    );
    assert_eq!(
        font_family(&doc, authored),
        "cursive",
        "an author `font-family` must beat the UA monospace rule"
    );
}

// ===== Guards =====

/// The new rules touch exactly the elements Chrome names and nothing else.
///
/// The discriminator against a rule list that over-reaches — `div`, `span`,
/// `li`, `dt`, `section` and `figcaption` all carry zero margins in Chrome, and
/// a `<div>` that grew 1em would move every layout in the workspace.
#[test]
fn the_block_margin_rule_does_not_reach_past_chromes_list() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let tags = ["div", "span", "li", "dt", "section", "figcaption", "header"];
    let ids: Vec<NodeId> = tags.iter().map(|t| el(&mut doc, c, t, "")).collect();
    doc.resolve_layout(VW, VH);

    for (id, tag) in ids.iter().zip(tags.iter()) {
        assert_eq!(
            margins(&doc, *id),
            (0.0, 0.0, 0.0, 0.0),
            "<{tag}> has no UA margin in Chrome and must not gain one here"
        );
    }

    // And `<body>`'s own `margin: 0` — rinch's deliberate deviation from
    // Chrome's 8px, for GUI apps — is untouched by any of this.
    assert_eq!(
        margins(&doc, body),
        (0.0, 0.0, 0.0, 0.0),
        "rinch zeroes the body margin on purpose; the new rules must not disturb it"
    );
}

/// `<hr>` is the only element that gained a border. The sheet's
/// `* { border-width: 0 }` reset — which exists to undo Stylo's `medium`
/// initial — still governs everything else.
///
/// Kills a mutant that raises the reset itself, or that spells the rule
/// `* { border-width: 1px }`.
#[test]
fn only_hr_gained_a_border() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let tags = ["div", "p", "pre", "blockquote", "table", "td", "img"];
    let ids: Vec<NodeId> = tags.iter().map(|t| el(&mut doc, c, t, "")).collect();
    doc.resolve_layout(VW, VH);

    for (id, tag) in ids.iter().zip(tags.iter()) {
        assert_eq!(
            border_width(&doc, *id),
            (0.0, 0.0, 0.0, 0.0),
            "<{tag}> must keep the UA sheet's zero border"
        );
    }
}

/// Nothing but `<pre>` preserves whitespace, and nothing but the four monospace
/// tags leaves the inherited family.
#[test]
fn the_pre_rules_do_not_leak_to_other_blocks() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let tags = ["div", "p", "blockquote", "figure", "ul"];
    let ids: Vec<NodeId> = tags.iter().map(|t| el(&mut doc, c, t, "")).collect();
    doc.resolve_layout(VW, VH);

    for (id, tag) in ids.iter().zip(tags.iter()) {
        assert_eq!(
            doc.tree.get(id.0).unwrap().computed_style.white_space,
            WhiteSpaceValue::Normal,
            "<{tag}> must keep `white-space: normal`"
        );
        assert_ne!(
            font_family(&doc, *id),
            "monospace",
            "<{tag}> must not become monospace"
        );
    }
}

/// The `Divider` component is an `<hr class="rinch-divider">` whose stylesheet
/// declares `border: 0; margin: 0`. Reproduced here as an author stylesheet so
/// the component's look is pinned against the new UA rule from inside
/// `rinch-dom`, which cannot depend on `rinch-components`.
///
/// The UA `overflow: hidden` is **not** overridden by that CSS and therefore
/// survives on a `Divider`, which is what the second half asserts: its labelled
/// variant puts a child inside the rule, and a clipped child would be the
/// regression this change could plausibly cause.
#[test]
fn the_divider_components_own_css_still_wins() {
    let mut doc = RinchDocument::new();
    doc.load_stylo_css(
        ".rinch-divider { border: 0; margin: 0; }\n\
         .rinch-divider--with-label { display: flex; align-items: center; height: auto; }\n\
         .rinch-divider__label { padding: 0 16px; }",
    );
    let body = doc.body();
    let c = el(&mut doc, body, "div", ROOT);
    let plain = el(&mut doc, c, "hr", "");
    doc.set_attribute(plain, "class", "rinch-divider");
    let labelled = el(&mut doc, c, "hr", "");
    doc.set_attribute(labelled, "class", "rinch-divider rinch-divider--with-label");
    let label = el(&mut doc, labelled, "span", "line-height: 20px");
    doc.set_attribute(label, "class", "rinch-divider__label");
    text(&mut doc, label, "or");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        border_width(&doc, plain),
        (0.0, 0.0, 0.0, 0.0),
        "the component's own `border: 0` must beat the UA <hr> border"
    );
    assert_eq!(
        margins(&doc, plain),
        (0.0, 0.0, 0.0, 0.0),
        "the component's own `margin: 0` must beat both UA <hr> margins"
    );

    // The labelled variant grows to hold its label; the UA `overflow: hidden`
    // it inherits from the new rule must not crop it.
    let h = height_of(&doc, labelled);
    assert!(
        h >= 20.0,
        "a labelled Divider must still be as tall as its label (>= 20px), got {h}"
    );
    assert!(
        height_of(&doc, label) > 0.0 && doc.tree.get(label.0).unwrap().layout.width > 0.0,
        "the Divider label must still lay out inside the rule"
    );
}

/// An author declaration whose `var()` is undefined does **not** fall through
/// to the UA rule: it is invalid at computed-value time, which computes to
/// `unset` — and `font-family` is inherited, so it takes the *parent's* family.
///
/// This is the #627 trap, and it is why `.rinch-code` and `.rinch-kbd` now
/// spell their family `var(--rinch-font-family-monospace, monospace)`. In a
/// build with `components` but no `theme` the custom property is undefined, so
/// without the fallback those two components would render in the body font even
/// with the new UA monospace rule in place — "the component declares it" and
/// "the component's declaration has a value" are different claims.
#[test]
fn an_undefined_var_inherits_rather_than_falling_through_to_the_ua_rule() {
    let mut doc = RinchDocument::new();
    doc.load_stylo_css(
        ".undefined { font-family: var(--rinch-font-family-monospace); }\n\
         .fallback { font-family: var(--rinch-font-family-monospace, monospace); }",
    );
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-family: cursive");
    let bare = el(&mut doc, c, "code", "");
    let undefined = el(&mut doc, c, "code", "");
    doc.set_attribute(undefined, "class", "undefined");
    let fallback = el(&mut doc, c, "code", "");
    doc.set_attribute(fallback, "class", "fallback");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        font_family(&doc, bare),
        "monospace",
        "positive control: with no author declaration the UA rule applies"
    );
    assert_eq!(
        font_family(&doc, undefined),
        "cursive",
        "an undefined var() computes to `unset` = `inherit`, NOT to the UA rule"
    );
    assert_eq!(
        font_family(&doc, fallback),
        "monospace",
        "…and a fallback in the var() is what keeps a theme-less component monospace"
    );
}
