//! Issue #627 — `<h1>`–`<h6>` and `<th>` must carry the browser's UA typography.
//!
//! rinch's UA stylesheet named `h1`–`h6` only in its `display: block` rule and
//! did not mention `th` at all, so a bare heading computed `font-size: 16`,
//! `font-weight: 400` and zero margins — body text. `rinch-web` runs on the
//! browser's own UA sheet, so the same markup rendered as a heading there: a
//! desktop/web divergence, which this project treats as a defect.
//!
//! **Every expected number here is measured in Chrome 150**, not derived from
//! what rinch happens to produce (`data:text/html` page, standards mode, root
//! `font-size: 16px`):
//!
//! | tag | `font-size` | `font-weight` | `margin-top`/`-bottom` |
//! |-----|-------------|---------------|------------------------|
//! | h1  | 32px (2em)      | 700 | 21.44px  (0.67em) |
//! | h2  | 24px (1.5em)    | 700 | 19.92px  (0.83em) |
//! | h3  | 18.72px (1.17em)| 700 | 18.72px  (1em)    |
//! | h4  | 16px (1em)      | 700 | 21.28px  (1.33em) |
//! | h5  | 13.28px (0.83em)| 700 | 22.1776px(1.67em) |
//! | h6  | 10.72px (0.67em)| 700 | 24.9776px(2.33em) |
//! | th  | 16px            | 700 | 0                 |
//!
//! The margins are `em`, and an `em` in `margin` resolves against the element's
//! **own** computed `font-size` — which is why h4's 1.33em is 21.28px and not
//! 21.28px-of-something-else, and why `<h1 style="font-size: 12px">` measures
//! 8.04px of margin in Chrome rather than keeping 21.44px. That coupling is the
//! fixed point this file is most careful about: a fixture that only ever looks
//! at h1 at the default root size cannot tell "0.67em of the element" from
//! "0.67em of the root", because both are 32px there.
//!
//! `th` gets no `display` rule. rinch has no table formatting context at all —
//! `DisplayValue` has no `TableCell` variant, and the UA sheet leaves `tr`,
//! `td`, `thead`, `tbody` at Stylo's default `inline` while giving `table`
//! `display: block`. Declaring `display: table-cell` would be a lie the layout
//! engine cannot honour, so the rule carries only `font-weight` and
//! `text-align`, both of which are real.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::values::{LengthPercentageAutoValue, TextAlignValue};

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let id = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(id, "style", style);
    }
    doc.append_child(parent, id);
    id
}

fn font_size(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.font_size
}

fn weight(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.font_weight
}

fn align(doc: &RinchDocument, id: NodeId) -> TextAlignValue {
    doc.tree.get(id.0).unwrap().computed_style.text_align
}

/// The resolved top/bottom margin in px. `LengthPercentageAutoValue::Length`
/// is what a resolved `em` must arrive as — a `Percent` or `Auto` here would
/// mean the declaration never reached layout, which is a distinct failure from
/// the wrong number and is reported as such.
fn margins(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let s = &doc.tree.get(id.0).unwrap().computed_style;
    let px = |m: &LengthPercentageAutoValue, which: &str| match m {
        LengthPercentageAutoValue::Length(v) => *v,
        other => panic!("{which} margin did not resolve to a length: {other:?}"),
    };
    (px(&s.margin_top, "top"), px(&s.margin_bottom, "bottom"))
}

/// Chrome 150's UA values, as measured. `(tag, font-size, margin)`; every
/// heading's weight is 700.
const CHROME: [(&str, f32, f32); 6] = [
    ("h1", 32.0, 21.44),
    ("h2", 24.0, 19.92),
    ("h3", 18.72, 18.72),
    ("h4", 16.0, 21.28),
    ("h5", 13.28, 22.1776),
    ("h6", 10.72, 24.9776),
];

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

// ===== The headline behaviour =====

/// A bare `<h1>`–`<h6>` carries Chrome's size, weight and margins.
///
/// This is the fixture that fails at HEAD: every heading read 16px/400/0.
#[test]
fn a_bare_heading_carries_the_browser_ua_typography() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // A declared root font-size, so the 2em..0.67em scale is anchored to a
    // declaration rather than to whatever `medium` happens to be.
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let ids: Vec<NodeId> = CHROME
        .iter()
        .map(|(tag, _, _)| el(&mut doc, c, tag, ""))
        .collect();
    doc.resolve_layout(800.0, 600.0);

    for (id, (tag, size, margin)) in ids.iter().zip(CHROME.iter()) {
        let got = font_size(&doc, *id);
        assert!(
            close(got, *size),
            "<{tag}> font-size: expected Chrome's {size}, got {got}"
        );
        assert_eq!(weight(&doc, *id), 700.0, "<{tag}> must be bold");
        let (top, bottom) = margins(&doc, *id);
        assert!(
            close(top, *margin) && close(bottom, *margin),
            "<{tag}> block margins: expected Chrome's {margin}, got ({top}, {bottom})"
        );
    }
}

/// `<th>` is bold and centred; `<td>` is neither.
///
/// The `<td>` half is the discriminator: a rule written `th, td { … }`, or a
/// selector typo landing on both, passes the `<th>` assertions alone.
#[test]
fn th_is_bold_and_centred_and_td_is_not() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let table = el(&mut doc, c, "table", "");
    let row = el(&mut doc, table, "tr", "");
    let th = el(&mut doc, row, "th", "");
    let td = el(&mut doc, row, "td", "");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(weight(&doc, th), 700.0, "<th> is bold in every browser");
    assert_eq!(align(&doc, th), TextAlignValue::Center, "<th> centres");
    assert_eq!(weight(&doc, td), 400.0, "<td> is NOT bold");
    assert_eq!(
        align(&doc, td),
        TextAlignValue::Start,
        "<td> does not centre"
    );
}

/// rinch has no table formatting context, and this change deliberately does not
/// invent one: `<th>` and `<td>` keep whatever `display` they had before.
///
/// Pinned because "browser UA parity" reads as an invitation to add
/// `display: table-cell`, which `DisplayValue` cannot represent — the resulting
/// value would fall back to something arbitrary rather than laying a table out.
/// If rinch ever grows tables, this test is the one to delete on purpose.
#[test]
fn the_th_rule_does_not_claim_a_table_display() {
    use rinch_dom::computed_style::values::DisplayValue;
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let table = el(&mut doc, body, "table", "");
    let row = el(&mut doc, table, "tr", "");
    let th = el(&mut doc, row, "th", "");
    let td = el(&mut doc, row, "td", "");
    doc.resolve_layout(800.0, 600.0);

    let disp = |id: NodeId| doc.tree.get(id.0).unwrap().computed_style.display;
    assert_eq!(disp(th), disp(td), "<th> and <td> still lay out alike");
    assert_eq!(
        disp(th),
        DisplayValue::Inline,
        "rinch leaves table cells at Stylo's default `inline`"
    );
}

// ===== #616's handshake: an author declaration must win =====

/// `<h1 style="font-weight: normal">` computes 400, and `<h1 style="font-size: 12px">`
/// computes 12 — the new rules are cascade rules, not a post-cascade patch
/// (#616/#618 deleted the patch machinery; re-adding one here would resurrect it).
#[test]
fn an_author_declaration_beats_the_new_ua_heading_rules() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let bare = el(&mut doc, c, "h1", "");
    let normal = el(&mut doc, c, "h1", "font-weight: normal");
    let twelve = el(&mut doc, c, "h1", "font-size: 12px");
    let th_normal = el(&mut doc, c, "th", "font-weight: normal");
    let th_left = el(&mut doc, c, "th", "text-align: left");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(weight(&doc, bare), 700.0, "control: the UA rule applies");
    assert_eq!(
        weight(&doc, normal),
        400.0,
        "`font-weight: normal` on <h1> must compute to 400"
    );
    assert!(
        close(font_size(&doc, twelve), 12.0),
        "`font-size: 12px` on <h1> must compute to 12, got {}",
        font_size(&doc, twelve)
    );
    assert_eq!(weight(&doc, th_normal), 400.0, "and on <th> too");
    assert_eq!(align(&doc, th_left), TextAlignValue::Start, "`left` wins");
}

/// An author `font-size` moves the margins with it, because the UA margin is
/// `em` and an `em` in `margin` resolves against the element's own font-size.
/// Chrome: `<h1 style="font-size: 12px">` has `margin-top: 8.04px` (0.67 × 12).
///
/// This is the fixed-point guard. At the default root size h1 is 32px and
/// 0.67em is 21.44px whether the `em` is read against the element or against
/// its parent — the two candidate rules agree exactly there. Declaring 12px
/// separates them: element-relative gives 8.04, parent-relative would give
/// 10.72.
#[test]
fn a_heading_margin_follows_the_elements_own_font_size() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let twelve = el(&mut doc, c, "h1", "font-size: 12px");
    let fifty = el(&mut doc, c, "h1", "font-size: 50px");
    doc.resolve_layout(800.0, 600.0);

    let (top, bottom) = margins(&doc, twelve);
    assert!(
        close(top, 8.04) && close(bottom, 8.04),
        "Chrome gives a 12px <h1> an 8.04px block margin, got ({top}, {bottom})"
    );
    let (top, _) = margins(&doc, fifty);
    assert!(
        close(top, 33.5),
        "a 50px <h1> must get 0.67 × 50 = 33.5, got {top}"
    );
}

/// The heading scale is relative to the *inherited* font-size, not to a fixed
/// 16px: an `<h1>` inside a 32px container is 64px in Chrome.
///
/// The second fixed point. `2em` and the literal `32px` are indistinguishable
/// at the default root size, so a mutant spelling the scale in px survives
/// every fixture that only ever uses the default.
#[test]
fn the_heading_scale_is_em_relative_not_a_fixed_pixel_size() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let big = el(&mut doc, body, "div", "width: 800px; font-size: 32px");
    let h1 = el(&mut doc, big, "h1", "");
    let h3 = el(&mut doc, big, "h3", "");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        close(font_size(&doc, h1), 64.0),
        "an <h1> under a 32px parent is 2em = 64px, got {}",
        font_size(&doc, h1)
    );
    assert!(
        close(font_size(&doc, h3), 37.44),
        "an <h3> under a 32px parent is 1.17em = 37.44px, got {}",
        font_size(&doc, h3)
    );
    // And the margin follows the heading's own new size, not the parent's.
    let (top, _) = margins(&doc, h1);
    assert!(
        close(top, 42.88),
        "0.67 × 64 = 42.88, got {top} (42.88 vs 21.44 separates \
         element-relative from root-relative)"
    );
}

/// A nested heading is sized from its ancestor heading, which is what `em`
/// means and what Chrome does: `<h1><h1>` is 64px inside 32px.
///
/// Chrome, measured: the inner `<h1>` reads `font-size: 64px`.
#[test]
fn a_nested_heading_compounds_like_every_other_em() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let outer = el(&mut doc, c, "h1", "");
    let inner = el(&mut doc, outer, "h1", "");
    doc.resolve_layout(800.0, 600.0);

    assert!(close(font_size(&doc, outer), 32.0));
    assert!(
        close(font_size(&doc, inner), 64.0),
        "a nested <h1> compounds to 64px, got {}",
        font_size(&doc, inner)
    );
}

/// The declarations survive a restyle, and removing the author override
/// restores the UA value — the same shape `font_weight_normal_tests` pins for
/// `<b>`, checked here because these rules are new.
#[test]
fn the_ua_heading_rules_survive_a_restyle() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let h2 = el(&mut doc, c, "h2", "font-weight: normal");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(weight(&doc, h2), 400.0, "the declaration wins first");

    doc.set_attribute(h2, "style", "color: red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(weight(&doc, h2), 700.0, "removing it restores the UA bold");
    assert!(close(font_size(&doc, h2), 24.0), "and the UA size");
    let (top, _) = margins(&doc, h2);
    assert!(close(top, 19.92), "and the UA margin, got {top}");
}

/// A heading's margins reach Taffy and move the box. The computed style
/// agreeing is not the same claim as the layout moving, and at HEAD both the
/// gap and the offset below were 0.
///
/// Both shapes are measured in Chrome 150, and rinch is asserted against
/// **Chrome's numbers**, including the margin collapsing that makes the two
/// differ:
///
/// | container | first `<h1>`'s offset in it | gap between the two |
/// |---|---|---|
/// | no padding      | 0       | 21.4375 |
/// | `padding-top: 1px` | 22.4375 | 21.4375 |
///
/// The first row is the one that would read as a bug and is not: an in-flow
/// first child's top margin collapses **out through** its parent's top edge, so
/// it moves the parent rather than the child, and the child's offset inside the
/// parent stays 0. Padding on the parent blocks that collapse and the margin
/// reappears as an offset — which is why the padded row is here at all. It is
/// this file's other fixed point: asserting only the unpadded case cannot
/// distinguish "the margin collapsed correctly" from "the margin never arrived".
///
/// Chrome's 21.4375 is its own LayoutUnit quantisation of 21.44 (1/64 px) and
/// rinch's is Taffy's whole-pixel rounding of the same 21.44 — measured, the
/// two engines' boxes agree to within that one pixel everywhere in this
/// fixture. So the layout assertions carry a 1px tolerance, where the computed
/// -style ones above use 0.01; a layout number here is a rounded box edge, not
/// a computed value. `line-height` is declared throughout so no glyph metric of
/// this host's font set enters any number.
#[test]
fn the_heading_margins_reach_layout() {
    const M: f32 = 21.44; // 0.67em of 32px, Chrome's computed margin-top on h1

    fn stacked(container_style: &str) -> (f32, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", container_style);
        let first = el(&mut doc, c, "h1", "line-height: 20px");
        let second = el(&mut doc, c, "h1", "line-height: 20px");
        doc.resolve_layout(800.0, 600.0);

        let y = |id: NodeId| doc.tree.get(id.0).unwrap().layout.y;
        let h = |id: NodeId| doc.tree.get(id.0).unwrap().layout.height;
        (y(first), y(second) - (y(first) + h(first)))
    }

    const BASE: &str = "width: 800px; font-size: 16px; line-height: 20px";

    // A box edge, within Taffy's whole-pixel rounding.
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0;

    let (first_y, gap) = stacked(BASE);
    assert!(
        near(gap, M),
        "two stacked <h1>s must be one collapsed 21.44px margin apart \
         (Chrome: 21.4375), got {gap}"
    );
    assert_eq!(
        first_y, 0.0,
        "with no padding on the container the first <h1>'s top margin collapses \
         out through it, so its offset stays exactly 0 (Chrome: 0)"
    );

    let (first_y, gap) = stacked(&format!("{BASE}; padding-top: 1px"));
    assert!(
        near(first_y, 1.0 + M),
        "padding blocks the collapse, so the first <h1> sits at 1 + 21.44 \
         (Chrome: 22.4375), got {first_y}"
    );
    assert!(
        near(gap, M),
        "and the gap between the two is unchanged, got {gap}"
    );
}

/// An author `margin-top: 0` beats the UA `margin-block`.
///
/// Worth its own fixture because it crosses the logical/physical boundary: the
/// UA rule is spelled `margin-block` (a *logical* shorthand, which Stylo maps to
/// the physical longhands during the cascade) and the author's is physical. If
/// the mapping happened after the cascade instead, the UA value would land on
/// top of the author's and this would read 21.44.
#[test]
fn a_physical_author_margin_beats_the_logical_ua_one() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
    let zeroed = el(&mut doc, c, "h1", "margin-top: 0; margin-bottom: 0");
    let top_only = el(&mut doc, c, "h1", "margin-top: 4px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        margins(&doc, zeroed),
        (0.0, 0.0),
        "an author `margin-top`/`margin-bottom` of 0 must beat the UA `margin-block`"
    );
    let (top, bottom) = margins(&doc, top_only);
    assert!(
        close(top, 4.0),
        "an author `margin-top` must win on its own axis, got {top}"
    );
    assert!(
        close(bottom, 21.44),
        "while the UA `margin-block` still supplies the bottom, got {bottom}"
    );
}

// ===== The component library must not move =====

/// `rinch-components`' `Title` renders a real `<h1>`–`<h6>`, so it is the one
/// consumer most exposed to a new UA heading rule. It is unaffected in **both**
/// of its configurations, and this fixture pins both because they are unaffected
/// for two *different* reasons:
///
/// - **Theme CSS loaded** (`App` does this whenever the `theme` feature is on):
///   `.rinch-title--1` declares `font-size: var(--rinch-h1-font-size)` and
///   `.rinch-title` declares `margin: 0`, and the theme defines those custom
///   properties (`--rinch-h1-font-size: 2.125rem` = 34px, weight 700). Author
///   declarations, so they simply beat the UA rule.
/// - **No theme CSS**: the custom properties are undefined, so each `var()`
///   declaration is *invalid at computed-value time* — and IACVT computes to
///   `unset`, which for the inherited `font-size`/`font-weight` means `inherit`.
///   The declaration still wins the cascade; it just resolves to the inherited
///   value. So the heading reads 16px/400, **not** the UA `2em`/bold.
///
/// That second case is the one worth a test: "an undefined `var()` falls back to
/// the UA rule" is the intuitive guess and it is wrong, and if it were right the
/// component library would silently change size in every theme-less build.
#[test]
fn the_title_component_is_unaffected_with_and_without_theme_css() {
    const TITLE_CSS: &str = ".rinch-title { margin: 0; } \
         .rinch-title--1 { font-size: var(--rinch-h1-font-size); \
                           font-weight: var(--rinch-h1-font-weight); }";

    fn title_h1(extra_css: &str) -> (f32, f32, (f32, f32)) {
        let mut doc = RinchDocument::new();
        doc.load_css(&format!("{extra_css}{TITLE_CSS}"));
        let body = doc.body();
        let c = el(&mut doc, body, "div", "width: 800px; font-size: 16px");
        let t = el(&mut doc, c, "h1", "");
        doc.set_attribute(t, "class", "rinch-title rinch-title--1");
        doc.resolve_layout(800.0, 600.0);
        (font_size(&doc, t), weight(&doc, t), margins(&doc, t))
    }

    let (size, w, m) = title_h1("");
    assert!(
        close(size, 16.0),
        "with no theme CSS the undefined var() computes to the inherited 16px, \
         not the UA 2em — got {size}"
    );
    assert_eq!(w, 400.0, "and to the inherited 400, not the UA bold");
    assert_eq!(
        m,
        (0.0, 0.0),
        "`.rinch-title {{ margin: 0 }}` beats the UA margin"
    );

    let themed = ":root { --rinch-h1-font-size: 2.125rem; --rinch-h1-font-weight: 700; } ";
    let (size, w, m) = title_h1(themed);
    assert!(
        close(size, 34.0),
        "with the theme loaded Title is the theme's 2.125rem = 34px, got {size}"
    );
    assert_eq!(w, 700.0, "and the theme's own weight");
    assert_eq!(m, (0.0, 0.0), "and still no margin");
}
