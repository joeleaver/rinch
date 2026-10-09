//! #1476 — an **auto-width** atomic inline (`inline-block`, `inline-flex`,
//! `inline-grid`) is shrink-to-fit where its containing block's width is
//! itself content-derived: inside another auto atomic inline, a flex item, a
//! grid item in an `auto` track, a shrink-to-fit absolute box, a
//! `fit-content` / `min-content` block.
//!
//! CSS sizes such a containing block from the box's **min-content
//! contribution** (its longest word) and **max-content contribution**, and
//! then lays the box out in the width that gives. rinch lined an atomic
//! inline up at the one size it had (`Node::layout`, its max-content width
//! until something capped it), so the containing block came out as wide as
//! the box and the cap could not bind.
//!
//! Every number in `CHROME` is **Chrome 153**'s (headless, standards mode,
//! `file://`, the bundled Inter registered as `ProbeFace` through
//! `@font-face`, `16px/20px`), read with `getBoundingClientRect`. The long
//! line's max-content width is 544.2 and its longest word 66.1, and the
//! containers are 400, 350, 380, 300 and 60 wide, so "max-content", "the
//! container" and "min-content" are three different answers in every row.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const L: &str = "Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters";

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

/// A document whose `<body>` holds one `16px/20px ProbeFace` wrapper around
/// `html`, not laid out yet.
fn mount(html: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "style", "font: 16px/20px ProbeFace");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc
}

fn lay_out(html: &str) -> RinchDocument {
    let mut doc = mount(html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `m=WxH` for every `[data-m]`, in the order the Chrome page reads them.
fn dump(doc: &RinchDocument) -> String {
    let mut out = vec![];
    for m in ["o", "f", "t", "u", "a", "i"] {
        for id in query_selector(&doc.tree, &format!("[data-m={m}]")) {
            let l = doc.tree.get(id).unwrap().layout;
            out.push(format!("{m}={:.1}x{:.1}", l.width, l.height));
        }
    }
    out.join(" ")
}

fn close(want: &str, got: &str) -> bool {
    let p = |s: &str| -> (f32, f32) {
        let (a, b) = s.split_once('=').unwrap().1.split_once('x').unwrap();
        (a.parse().unwrap(), b.parse().unwrap())
    };
    want.split(' ').count() == got.split(' ').count()
        && want.split(' ').zip(got.split(' ')).all(|(c, r)| {
            let (c, r) = (p(c), p(r));
            (c.0 - r.0).abs() < 1.0 && (c.1 - r.1).abs() < 0.5
        })
}

fn id(doc: &RinchDocument, m: &str) -> rinch_core::dom::NodeId {
    let ids = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(ids.len(), 1, "one [data-m={m}]");
    rinch_core::dom::NodeId(ids[0])
}

/// (name, html, Chrome 153)
const CHROME: &[(&str, &str, &str)] = &[
    (
        "c01_nested",
        r##"<div style="width:400px"><span data-m="o" style="display:inline-block"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div>"##,
        "o=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c02_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c03_grid_item",
        r##"<div style="width:400px;display:grid"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c04_grid_autocol",
        r##"<div style="width:400px;display:grid;grid-template-columns:auto 50px"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div><div>x</div></div>"##,
        "f=350.0x40.0 t=350.0x40.0",
    ),
    (
        "c05_abs",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c06_ib_in_flex_ib",
        r##"<div style="width:400px;display:flex"><span data-m="o" style="display:inline-block"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div>"##,
        "o=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c07_column_align_start",
        r##"<div style="width:400px;display:flex;flex-direction:column;align-items:flex-start"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c08_flex_item_text_before",
        r##"<div style="width:400px;display:flex"><div data-m="f">ab <span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x60.0 t=400.0x40.0",
    ),
    (
        "c09_triple",
        r##"<div style="width:400px"><span data-m="o" style="display:inline-block"><span data-m="u" style="display:inline-block"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></span></div>"##,
        "o=400.0x40.0 t=400.0x40.0 u=400.0x40.0",
    ),
    (
        "c10_inline_flex_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-flex">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c11_two_items",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div><div data-m="u"><span data-m="a" style="display:inline-block">short text here</span></div></div>"##,
        "f=332.6x40.0 t=332.6x40.0 u=67.4x40.0 a=67.4x40.0",
    ),
    (
        "c12_flex_grow_wide",
        r##"<div style="width:700px;display:flex"><div data-m="f" style="flex:1"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=700.0x20.0 t=544.2x20.0",
    ),
    (
        "c13_min_content_cb",
        r##"<div data-m="f" style="width:min-content"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "f=66.1x200.0 t=66.1x200.0",
    ),
    (
        "c15_nested_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="o" style="display:inline-block"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div></div>"##,
        "o=400.0x40.0 f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c16_margin_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;margin:0 30px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=340.0x40.0",
    ),
    (
        "c18_padding_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;padding:0 10px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c20_text_then_inner",
        r##"<div style="width:400px"><span data-m="o" style="display:inline-block">pre <span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div>"##,
        "o=400.0x60.0 t=400.0x40.0",
    ),
    (
        "c21_two_ibs_in_flex_item",
        r##"<div style="width:300px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW</span> <span data-m="u" style="display:inline-block">mmm Wavy milliliters</span></div></div>"##,
        "f=300.0x40.0 t=164.9x20.0 u=158.9x20.0",
    ),
    (
        "c22_grid_two_auto",
        r##"<div style="width:400px;display:grid;grid-template-columns:auto auto"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div><div data-m="u"><span data-m="a" style="display:inline-block">short text here</span></div></div>"##,
        "f=289.8x40.0 t=289.8x40.0 u=110.2x40.0 a=110.2x20.0",
    ),
    (
        "c23_fit_content_cb",
        r##"<div style="width:400px"><div data-m="f" style="width:fit-content"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c24_inline_grid_nested",
        r##"<div style="width:400px"><span data-m="o" style="display:inline-grid"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div>"##,
        "o=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c25_flex_item_fits",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">short text here</span></div></div>"##,
        "f=110.2x20.0 t=110.2x20.0",
    ),
    (
        "c26_wrap_narrow_word",
        r##"<div style="width:60px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW</span></div></div>"##,
        "f=66.1x60.0 t=66.1x60.0",
    ),
    (
        "c27_nested_narrow",
        r##"<div style="width:60px"><span data-m="o" style="display:inline-block"><span data-m="t" style="display:inline-block">Wavy milliliters WWW</span></span></div>"##,
        "o=66.1x60.0 t=66.1x60.0",
    ),
    (
        "c28_flex_nowrap_ib",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;white-space:nowrap">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=544.2x20.0 t=544.2x20.0",
    ),
    (
        "c29_column_stretch",
        r##"<div style="width:400px;display:flex;flex-direction:column"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "c30_abs_in_abs",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute;right:0"><span data-m="o" style="display:inline-block"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div></div>"##,
        "o=400.0x40.0 f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "d01_capped_within_a_pixel",
        r##"<div data-m="f" style="width:543px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "f=543.0x40.0 t=543.0x40.0",
    ),
    (
        "d02_pct_box_inside_in_flex_item",
        r##"<div style="width:300px;display:flex"><div data-m="f"><span data-m="o" style="display:inline-block">Wavy milliliters WWW mmm <span data-m="t" style="display:inline-block;width:50%">Wavy milliliters</span> Wavy milliliters</span></div></div>"##,
        "o=300.0x40.0 f=300.0x40.0 t=150.0x20.0",
    ),
    (
        "d06_button_in_narrow_item",
        r##"<div style="width:100px;display:flex"><div data-m="f">Label <span data-m="t" style="display:inline-flex;padding:0 8px">Add item now</span></div></div>"##,
        "f=100.0x60.0 t=100.0x40.0",
    ),
    (
        "d07_in_span_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f">ab <span>cd <span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span> ef</span> gh</div></div>"##,
        "f=400.0x80.0 t=400.0x40.0",
    ),
    (
        "d08_two_long_in_one_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span> <span data-m="u" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x80.0 t=400.0x40.0 u=400.0x40.0",
    ),
    (
        "d09_scroll_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f" style="overflow:hidden"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "d10_padded_outer",
        r##"<div style="width:400px"><span data-m="o" style="display:inline-block;padding:0 10px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></span></div>"##,
        "o=400.0x40.0 t=380.0x40.0",
    ),
    (
        "d11_inline_flex_row_of_boxes",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="o" style="display:inline-flex"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy</span><span data-m="u" style="display:inline-block">milliliters WWW mmm Wavy milliliters</span></span></div></div>"##,
        "o=400.0x40.0 f=400.0x40.0 t=190.9x40.0 u=209.1x40.0",
    ),
    (
        "d14_column_start_margins",
        r##"<div style="width:400px;display:flex;flex-direction:column;align-items:flex-start"><div data-m="f"><span data-m="t" style="display:inline-block;margin:0 30px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=340.0x40.0",
    ),
    (
        "d15_abs_margins",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute"><span data-m="t" style="display:inline-block;margin:0 30px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=340.0x40.0",
    ),
    (
        "d16_pct_inside_eligible",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters<span data-m="i" style="display:inline-block;width:50%">Wavy milliliters</span> Wavy</span></div></div>"##,
        "f=400.0x60.0 t=400.0x60.0 i=200.0x20.0",
    ),
    (
        "d13_column_center",
        r##"<div style="width:400px;display:flex;flex-direction:column;align-items:center"><div data-m="f">ab <span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x60.0 t=400.0x40.0",
    ),
];

/// (name, html, rinch today, Chrome 153, why) — not Chrome's yet, for a
/// reason that is not this issue's. Each pins rinch's current answer so a
/// fix has to come here and move the row up.
const GAPS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "c17_abs_left",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
        "f=380.0x40.0 t=380.0x40.0",
        "#1404: Taffy shrinks an auto-width absolute box to fit its parent's width, not that width less its insets",
    ),
    (
        "d03_pct_max_width_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;max-width:80%">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=278.0x60.0 t=278.0x60.0",
        "f=400.0x40.0 t=320.0x40.0",
        "a percentage `max-width` is not plain shrink-to-fit: the box is lined up at the size it has, which its own cyclic percentage shrinks (the #1293 class)",
    ),
    (
        "d04_fit_content_in_flex_item",
        r##"<div style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;width:fit-content">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=544.0x20.0 t=544.0x20.0",
        "f=400.0x40.0 t=400.0x40.0",
        "a `fit-content` width is not `auto`: the box is lined up at the size it has (max-content)",
    ),
    (
        "d12_abs_right_only",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute;right:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
        "f=380.0x40.0 t=380.0x40.0",
        "#1404, as c17",
    ),
];

#[test]
fn content_sized_containing_blocks_match_chrome_153() {
    let bad: Vec<_> = CHROME
        .iter()
        .filter_map(|(n, h, want)| {
            let got = dump(&lay_out(h));
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn known_gaps_pin_rinchs_current_answer() {
    let bad: Vec<_> = GAPS
        .iter()
        .filter_map(|(n, h, now, chrome, why)| {
            let got = dump(&lay_out(h));
            (!close(now, &got))
                .then(|| format!("{n} ({why}): pinned[{now}] got[{got}] chrome[{chrome}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The shapes of the histories below: a container `[data-m=c]` of width
/// `{W}` around an `auto` box `[data-m=t]` holding `{T}`.
const SHAPES: &[(&str, &str)] = &[
    (
        "nested",
        r#"<div data-m="c" style="width:{W}px"><span data-m="o" style="display:inline-block"><span data-m="t" style="display:inline-block">{T}</span></span></div>"#,
    ),
    (
        "flex_item",
        r#"<div data-m="c" style="width:{W}px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">{T}</span></div></div>"#,
    ),
    (
        "flex_item_two",
        r#"<div data-m="c" style="width:{W}px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">{T}</span></div><div data-m="u"><span data-m="a" style="display:inline-block">short text here</span></div></div>"#,
    ),
    (
        "grid_auto_track",
        r#"<div data-m="c" style="width:{W}px;display:grid;grid-template-columns:auto 50px"><div data-m="f"><span data-m="t" style="display:inline-block">{T}</span></div><div>x</div></div>"#,
    ),
    (
        "absolute",
        r#"<div data-m="c" style="position:relative;width:{W}px;height:100px"><div data-m="f" style="position:absolute"><span data-m="t" style="display:inline-block">{T}</span></div></div>"#,
    ),
    (
        "column_align_start",
        r#"<div data-m="c" style="width:{W}px;display:flex;flex-direction:column;align-items:flex-start"><div data-m="f"><span data-m="t" style="display:inline-block">{T}</span></div></div>"#,
    ),
    (
        "nested_in_flex_item",
        r#"<div data-m="c" style="width:{W}px;display:flex"><div data-m="f"><span data-m="o" style="display:inline-flex"><span data-m="t" style="display:inline-block">{T}</span></span></div></div>"#,
    ),
    (
        "fit_content_block",
        r#"<div data-m="c" style="width:{W}px"><div data-m="f" style="width:fit-content">ab <span data-m="t" style="display:inline-block">{T}</span></div></div>"#,
    ),
    (
        "column_start_margins",
        r#"<div data-m="c" style="width:{W}px;display:flex;flex-direction:column;align-items:flex-start"><div data-m="f"><span data-m="t" style="display:inline-block;margin:0 30px">{T}</span></div></div>"#,
    ),
    (
        "percentage_box_inside",
        r#"<div data-m="c" style="width:{W}px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">{T}<span data-m="i" style="display:inline-block;width:50%">Wavy milliliters</span> Wavy</span></div></div>"#,
    ),
    (
        "inside_an_abs_inside_a_box",
        r#"<div data-m="c" style="width:{W}px"><span data-m="o" style="display:inline-flex;position:relative;width:50px;height:20px"><div data-m="f" style="position:absolute;width:{W}px;display:grid;grid-template-columns:1fr 1fr"><div data-m="u">ab <span data-m="t" style="display:inline-block">{T}</span></div></div></span></div>"#,
    ),
];

fn shape(template: &str, width: u32, text: &str) -> String {
    template
        .replace("{W}", &width.to_string())
        .replace("{T}", text)
}

const TEXTS: &[&str] = &[
    L,
    "Wavy",
    "Wavy milliliters WWW",
    "Wavymillilitersunbroken WWW mmm Wavy milliliters WWW mmm Wavy",
    L,
];

/// Incremental layout equals a fresh layout of the same document, through
/// container resizes (wider, where a capped box has to come back to its
/// max-content width, and narrower than its longest word) and text edits
/// (which change the box's min-content width, so the one that was measured
/// must not be used again).
#[test]
fn a_history_of_resizes_and_edits_equals_a_fresh_layout() {
    let mut bad = vec![];
    for (name, template) in SHAPES {
        let mut doc = lay_out(&shape(template, 400, L));
        let (c, t) = (id(&doc, "c"), id(&doc, "t"));
        let text_node = rinch_core::dom::NodeId(doc.tree.get(t.0).unwrap().children[0]);
        let mut tick = 0.0;
        let mut text = L;
        let mut check = |doc: &mut RinchDocument, what: String, width: u32, text: &str| {
            tick += 1.0;
            doc.resolve_layout(800.0, 600.0 + tick);
            let (inc, fresh) = (dump(doc), dump(&lay_out(&shape(template, width, text))));
            if inc != fresh {
                bad.push(format!("{name} {what}: incremental[{inc}] fresh[{fresh}]"));
            }
        };
        for width in [600, 300, 700, 60, 400, 545, 543, 400] {
            doc.set_style(c, "width", &format!("{width}px"));
            if *name == "inside_an_abs_inside_a_box" {
                doc.set_style(id(&doc, "f"), "width", &format!("{width}px"));
            }
            check(&mut doc, format!("width {width}"), width, text);
        }
        for (n, new) in TEXTS.iter().enumerate() {
            text = new;
            doc.set_text_content(text_node, text);
            check(&mut doc, format!("text {n}"), 400, text);
            doc.set_style(c, "width", "200px");
            if *name == "inside_an_abs_inside_a_box" {
                doc.set_style(id(&doc, "f"), "width", "200px");
            }
            check(&mut doc, format!("text {n} at 200"), 200, text);
            doc.set_style(c, "width", "400px");
            if *name == "inside_an_abs_inside_a_box" {
                doc.set_style(id(&doc, "f"), "width", "400px");
            }
            check(&mut doc, format!("text {n} back at 400"), 400, text);
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// What the min-content size costs, in computes of the box
/// (`inline_block_computes`), and who pays it.
///
/// - A box on lines that are never narrower than it — a chip in a block —
///   is measured once and never at min-content.
/// - A box in a container sized from its content (a flex item here) is asked
///   for its min-content size by that container: the min-content compute and
///   the `auto` measure that puts its interior back, once.
/// - After that a layout that changes nothing inside the box measures
///   nothing, and capping it uses the size already measured.
#[test]
fn the_min_content_size_is_measured_once_and_only_when_a_line_asks() {
    use rinch_dom::perf::Counter;
    let computes = |doc: &RinchDocument| doc.tree.perf.total().get(Counter::InlineBlockComputes);
    let chip = r#"<div><span style="display:inline-block">short text</span></div>"#;

    // 20 chips in blocks.
    let doc = lay_out(&format!(
        r#"<div data-m="c" style="width:700px">{}</div>"#,
        chip.repeat(20)
    ));
    assert_eq!(computes(&doc), 20, "a chip in a block: one compute");

    // 20 chips in flex items.
    let mut doc = lay_out(&format!(
        r#"<div data-m="c" style="width:700px;display:flex;flex-wrap:wrap">{}</div>"#,
        chip.repeat(20)
    ));
    let first = computes(&doc);
    assert_eq!(first, 60, "a chip in a flex item: three computes, once");
    let c = id(&doc, "c");
    doc.set_style(c, "width", "690px");
    doc.resolve_layout(800.0, 601.0);
    assert_eq!(computes(&doc), first, "a relayout measures no chip");

    // One box too wide for its flex item: the three above, then the capped
    // layout (a max-content probe and the layout at the capped width).
    let mut doc = lay_out(&shape(SHAPES[1].1, 400, L));
    let first = computes(&doc);
    assert_eq!(first, 5);
    // Narrower: the min-content size is known. One probe, one layout.
    let c = id(&doc, "c");
    doc.set_style(c, "width", "300px");
    doc.resolve_layout(800.0, 601.0);
    assert_eq!(computes(&doc) - first, 2);
}

/// A flex item that becomes an atomic inline because its parent's `display`
/// changed has never been measured as one, so it has no recorded natural
/// size: it is lined up at the size it was laid out at. (Found by the scoped
/// IFC oracle, seed 1129: the box around it came out empty, 2 x 2.)
#[test]
fn a_box_that_becomes_an_atomic_inline_by_a_restyle_is_lined_up_at_its_size() {
    let html = |outer: &str| {
        format!(
            r#"<div style="width:110px"><section data-m="o" style="{outer}"><span data-m="t" style="display:inline-flex">Wavy</span></section></div>"#
        )
    };
    let mut doc = lay_out(&html("display:inline-flex"));
    let o = id(&doc, "o");
    doc.set_attribute(o, "style", "display:inline-block; padding:1px");
    doc.resolve_layout(800.0, 601.0);
    let fresh = dump(&lay_out(&html("display:inline-block; padding:1px")));
    assert_eq!(dump(&doc), fresh);
    assert!(close("o=43.7x22.0 t=41.7x20.0", &fresh), "{fresh}");
}
