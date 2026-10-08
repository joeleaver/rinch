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
        "c17_abs_left",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute;left:20px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=380.0x40.0 t=380.0x40.0",
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
