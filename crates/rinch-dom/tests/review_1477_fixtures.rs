//! Review fixtures for PR #1477 (#658 #662): every number in `chrome` is
//! Chrome 153 (headless, bundled Inter as ProbeFace, 16px/20px,
//! getBoundingClientRect). `GAPS` pin rinch's current answer where it is not
//! Chrome's, with the reason.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const L: &str = "Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters";
fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc
}
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

/// (name, html, Chrome 153)
const CHROME: &[(&str, &str, &str)] = &[
    (
        "p01_nowrap_piece",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block">Wavy milliliters <span style="white-space:nowrap">WWW mmm Wavy milliliters WWW</span> mmm Wavy</span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p02_nowrap_wider",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block">ab <span style="white-space:nowrap">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span> cd</span></div>"##,
        "t=544.2x60.0",
    ),
    (
        "p03_pct_padding_long",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;padding:0 10%">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p05_minw_450",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;min-width:450px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=450.0x40.0",
    ),
    (
        "p07_minw_pct_binding_long",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;min-width:120%">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=480.0x40.0",
    ),
    (
        "p11_in_table_cell",
        r##"<div style="width:400px;display:table-cell"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p13_in_abs_left_right",
        r##"<div style="position:relative;width:400px;height:100px"><div data-m="f" style="position:absolute;left:0;right:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
    (
        "p14_rtl",
        r##"<div style="width:400px;direction:rtl"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p15_inline_flex_col",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-flex;flex-direction:column"><span>Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span><span>x</span></span></div>"##,
        "t=400.0x60.0",
    ),
    (
        "p16_inline_grid_pct_tracks",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-grid;grid-template-columns:50% 50%"><span data-m="a">aaa</span><span>bbbbbbbbbb</span></span></div>"##,
        "t=124.9x20.0 a=62.5x20.0",
    ),
    (
        "p17_inline_flex_pct_gap",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-flex;column-gap:10%"><span>aaa</span><span>bbb</span></span></div>"##,
        "t=56.3x20.0",
    ),
    (
        "p18_inline_flex_long_two_items",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-flex"><span data-m="a">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span><span>tail</span></span></div>"##,
        "t=400.0x40.0 a=378.0x40.0",
    ),
    (
        "p20_ib_holds_block_with_long",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block"><div>Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</div></span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p21_ib_pct_text_indent",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;text-indent:10%">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p22_two_ibs_line",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span><span data-m="u" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x40.0 u=400.0x40.0",
    ),
    (
        "p23_border_box_pad",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;box-sizing:border-box;padding:0 20px;border:5px solid">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x50.0",
    ),
    (
        "p24_negative_margin",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;margin:0 -50px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=500.0x40.0",
    ),
    (
        "p25_pct_margin",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;margin:0 10%">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=320.0x40.0",
    ),
    (
        "p27_ib_in_li_padded",
        r##"<div style="width:400px;padding-left:40px;box-sizing:border-box"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=360.0x40.0",
    ),
    (
        "p28_overflow_hidden_ib",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;overflow:hidden">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=400.0x40.0",
    ),
    (
        "p29_child_pct_padding",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block">more text here longer<div data-m="i" style="padding-left:50%">Wavy milliliters</div></span></div>"##,
        "t=162.5x60.0 i=162.5x40.0",
    ),
    (
        "p30_child_pct_margin",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block">more text here longer<div data-m="i" style="margin-left:50%">Wavy milliliters</div></span></div>"##,
        "t=162.5x60.0 i=81.3x40.0",
    ),
    (
        "p31_flex_child_pct_basis",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-flex;flex-wrap:wrap"><span data-m="i" style="flex-basis:50%">Wavy milliliters</span><span>more text here</span></span></div>"##,
        "t=222.2x40.0 i=111.1x40.0",
    ),
    (
        "p32_margin_fits_only_without",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;margin:0 30px">Wavy milliliters WWW mmm Wavy milliliters WWW</span></div>"##,
        "t=340.0x40.0",
    ),
    (
        "p33_margin_fits_only_without_b",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;margin:0 20px">Wavy milliliters WWW mmm Wavy milliliters WWW mmm</span></div>"##,
        "t=360.0x40.0",
    ),
    (
        "p34_flex_item_minw0",
        r##"<div style="width:400px;display:flex"><div data-m="f" style="min-width:0"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div></div>"##,
        "f=400.0x40.0 t=400.0x40.0",
    ),
];

/// (name, html, rinch today, Chrome 153, why)
/// (The five #1476 rows that were here — an `auto` box in a flex item, a grid
/// item, a shrink-to-fit absolute box — are Chrome's now, and live in
/// `atomic_inline_content_sized_cb_1476_tests.rs`.)
const GAPS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "p04_pct_padding_short",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;padding:0 10%">short text</span></div>"##,
        "t=72.0x20.0",
        "t=151.8x20.0",
        "pre-existing: own percentage padding on a fitting auto box resolves against nothing (issue draft 2)",
    ),
    (
        "p06_maxw_50pct",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block;max-width:50%">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"##,
        "t=100.0x160.0",
        "t=200.0x80.0",
        "pre-existing: pass C re-resolves a percentage max-width against the clamped width (issue draft 1)",
    ),
    (
        "p26_img_child_maxw",
        r##"<div style="width:400px"><span data-m="t" style="display:inline-block">short <span data-m="a" style="display:inline-block;width:100%;height:10px;background:red"></span></span></div>"##,
        "t=44.0x30.0 a=44.0x10.0",
        "t=43.5x40.0 a=43.5x10.0",
        "#624/#1258: a line holding only an atomic inline has no strut",
    ),
];

#[test]
fn matches_chrome_153() {
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

fn fresh_at(html: &str, w: f32) -> String {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "style", "font: 16px/20px ProbeFace");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(w, 600.0);
    dump(&doc)
}
fn id(doc: &RinchDocument, m: &str) -> rinch_core::dom::NodeId {
    rinch_core::dom::NodeId(query_selector(&doc.tree, &format!("[data-m={m}]"))[0])
}

#[test]
fn probe_resize() {
    let html = format!(
        r#"<div style="width:50%"><span data-m="t" style="display:inline-block">{L}</span> <span data-m="u" style="display:inline-block">short text here</span></div>"#
    );
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "style", "font: 16px/20px ProbeFace");
    doc.set_inner_html(wrap, &html);
    doc.append_child(body, wrap);
    let mut bad = 0;
    for w in [
        800.0, 1400.0, 600.0, 200.0, 240.0, 800.0, 1200.0, 1000.0, 260.0,
    ] {
        doc.resolve_layout(w, 600.0);
        let (a, b) = (dump(&doc), fresh_at(&html, w));
        println!("RESIZE w={w} inc[{a}] fresh[{b}]");
        if a != b {
            bad += 1;
        }
    }
    assert_eq!(bad, 0);
}

#[test]
fn probe_text_edits() {
    // grow: short -> long; shrink: long -> short; cb width style change.
    let mut bad = 0;
    for (start, end) in [("short", L), (L, "short"), ("Wavy milliliters WWW", L)] {
        let mk = |t: &str| {
            format!(
                r#"<div data-m="f" style="width:400px"><span data-m="t" style="display:inline-block"><b data-m="a">{t}</b></span></div>"#
            )
        };
        let mut doc = lay_out(&mk(start));
        let a = id(&doc, "a");
        doc.set_text_content(a, end);
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk(end))));
        println!("EDIT {start:.5}->{end:.5} inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    for (w0, w1) in [(400, 300), (300, 700), (700, 300), (400, 600), (600, 560)] {
        let mk = |w| {
            format!(
                r#"<div data-m="f" style="width:{w}px"><span data-m="t" style="display:inline-block">{L}</span><span data-m="u" style="display:inline-block">Wavy milliliters WWW mmm Wavy</span></div>"#
            )
        };
        let mut doc = lay_out(&mk(w0));
        let f = id(&doc, "f");
        doc.set_style(f, "width", &format!("{w1}px"));
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk(w1))));
        println!("CBW {w0}->{w1} inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    // a percentage child appears later (restyle only)
    {
        let mk = |s: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="t" style="display:inline-block"><div data-m="i" style="{s}">Wavy milliliters</div>more text here</span></div>"#
            )
        };
        let mut doc = lay_out(&mk(""));
        let i = id(&doc, "i");
        doc.set_style(i, "width", "50%");
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk("width: 50%"))));
        println!("PCTLATE inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    // box's own display flips block -> inline-block
    {
        let mk = |d: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="t" style="display:{d}">{L}</span></div>"#
            )
        };
        let mut doc = lay_out(&mk("inline"));
        let t = id(&doc, "t");
        doc.set_style(t, "display", "inline-block");
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk("inline-block"))));
        println!("DISPFLIP inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    assert_eq!(bad, 0);
}

#[test]
fn probe_more_incremental() {
    let mut bad = 0;
    // vw child inside an auto ib, resize
    let html = r#"<div style="width:400px"><span data-m="t" style="display:inline-block">ab <div data-m="i" style="width:60vw;height:5px"></div> Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span></div>"#;
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "style", "font: 16px/20px ProbeFace");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    for w in [800.0, 600.0, 400.0, 900.0, 500.0] {
        doc.resolve_layout(w, 600.0);
        let (a, b) = (dump(&doc), fresh_at(html, w));
        println!("VW w={w} inc[{a}] fresh[{b}]");
        if a != b {
            bad += 1;
        }
    }
    // font-size change on the containing block (inherited)
    {
        let mk = |fs: &str| {
            format!(
                r#"<div data-m="f" style="width:400px;{fs}"><span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy</span></div>"#
            )
        };
        let mut doc = lay_out(&mk(""));
        let f = id(&doc, "f");
        doc.set_style(f, "font-size", "24px");
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk("font-size:24px"))));
        println!("FS inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
        doc.set_style(f, "font-size", "16px");
        doc.resolve_layout(800.0, 602.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk("font-size:16px"))));
        println!("FS2 inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    // chain: ib(auto) > div(width:80%) > ib(auto) long text; then edit outer text
    {
        let mk = |t: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="o" style="display:inline-block">{t}<div data-m="f" style="width:80%"><span data-m="t" style="display:inline-block">{L}</span></div></span></div>"#
            )
        };
        let mut doc = lay_out(&mk("head "));
        println!("CHAIN fresh0[{}]", dump(&doc));
        let o = id(&doc, "o");
        let tx = doc.create_text("more more more more more more more more more more more");
        doc.append_child(o, tx);
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (
            dump(&doc),
            dump(&lay_out(&mk("head ").replace(
                "</div></span></div>",
                "</div>more more more more more more more more more more more</span></div>",
            ))),
        );
        println!("CHAIN inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    // capped ib, then its max-width restyled to 200px and back to none
    {
        let mk = |s: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="t" style="display:inline-block;{s}">{L}</span></div>"#
            )
        };
        let mut doc = lay_out(&mk(""));
        let t = id(&doc, "t");
        doc.set_style(t, "max-width", "200px");
        doc.resolve_layout(800.0, 601.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk("max-width:200px"))));
        println!("MAXW inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
        doc.set_style(t, "max-width", "none");
        doc.resolve_layout(800.0, 602.0);
        let (x, y) = (dump(&doc), dump(&lay_out(&mk("max-width:none"))));
        println!("MAXW2 inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
        doc.set_style(t, "margin-left", "150px");
        doc.resolve_layout(800.0, 603.0);
        let (x, y) = (
            dump(&doc),
            dump(&lay_out(&mk("max-width:none;margin-left:150px"))),
        );
        println!("MARGIN inc[{x}] fresh[{y}]");
        if x != y {
            bad += 1;
        }
    }
    assert_eq!(bad, 0);
}

// ---------------------------------------------------------------------------
// Round 2: histories aimed at the fast paths' preconditions (maps empty / no
// percentage-sized atomic inline at the start, then one appears or goes).
fn find(doc: &RinchDocument, m: &str) -> rinch_core::dom::NodeId {
    id(doc, m)
}
fn check(tag: &str, doc: &RinchDocument, end: &str, bad: &mut u32) {
    let (x, y) = (dump(doc), dump(&lay_out(end)));
    println!("R2 {tag} inc[{x}] fresh[{y}]");
    if x != y {
        *bad += 1;
    }
}
const S381: &str = "Wavy milliliters WWW mmm Wavy milliliters WWW";

#[test]
fn round2_fast_path_histories() {
    let mut bad = 0;
    // H1/H2: an inner auto ib restyled to a percentage width inside a fitting
    // inline-flex, then back to auto.
    {
        let mk = |s: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="o" style="display:inline-flex">head <span data-m="i" style="display:inline-block;{s}">Wavy milliliters WWW</span> tail</span></div>"#
            )
        };
        let mut doc = lay_out(&mk(""));
        let i = find(&doc, "i");
        doc.set_style(i, "width", "50%");
        doc.resolve_layout(800.0, 601.0);
        check("H1 auto->50%", &doc, &mk("width:50%"), &mut bad);
        doc.set_style(i, "width", "auto");
        doc.resolve_layout(800.0, 602.0);
        check("H2 50%->auto", &doc, &mk("width:auto"), &mut bad);
        doc.set_style(i, "min-width", "120%");
        doc.resolve_layout(800.0, 603.0);
        check(
            "H2b min-width 120%",
            &doc,
            &mk("width:auto;min-width:120%"),
            &mut bad,
        );
    }
    // H3/H4: a percentage ib inserted later into a fitting auto ib, then removed.
    {
        let base = r#"<div style="width:400px"><span data-m="o" style="display:inline-block">head words here</span></div>"#;
        let with = r#"<div style="width:400px"><span data-m="o" style="display:inline-block">head words here<span data-m="i" style="display:inline-block;width:40%">x</span></span></div>"#;
        let mut doc = lay_out(base);
        let o = find(&doc, "o");
        let i = doc.create_element("span");
        doc.set_attribute(i, "data-m", "i");
        doc.set_attribute(i, "style", "display:inline-block;width:40%");
        let x = doc.create_text("x");
        doc.append_child(i, x);
        doc.append_child(o, i);
        doc.resolve_layout(800.0, 601.0);
        check("H3 insert %", &doc, with, &mut bad);
        doc.remove_node(i);
        doc.resolve_layout(800.0, 602.0);
        check("H4 remove %", &doc, base, &mut bad);
    }
    // H4b: a capped (keyword-entry) box inside a fitting outer, removed.
    {
        let base = r#"<div style="width:400px"><span data-m="o" style="display:inline-flex;flex-direction:column">head</span></div>"#;
        let with = format!(
            r#"<div style="width:400px"><span data-m="o" style="display:inline-flex;flex-direction:column">head<span data-m="i" style="display:inline-block">{L}</span></span></div>"#
        );
        let mut doc = lay_out(&with);
        let i = find(&doc, "i");
        doc.remove_node(i);
        doc.resolve_layout(800.0, 601.0);
        check("H4b remove capped", &doc, base, &mut bad);
    }
    // H5: margins in the fit test, by restyle: 0 -> 30px -> -20px -> 1em -> 10%.
    {
        let mk = |m: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="t" style="display:inline-block;margin:0 {m}">{S381}</span></div>"#
            )
        };
        let mut doc = lay_out(&mk("0px"));
        let t = find(&doc, "t");
        let mut h = 601.0;
        for m in [
            "30px",
            "0px",
            "-20px",
            "1em",
            "0.5em",
            "10%",
            "auto",
            "12px",
            "calc(5px + 2%)",
        ] {
            doc.set_style(t, "margin", &format!("0 {m}"));
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(&format!("H5 margin {m}"), &doc, &mk(m), &mut bad);
        }
    }
    // H6: em margins, font-size changed on the box (margin moves, text moves).
    {
        let mk = |fs: &str| {
            format!(
                r#"<div style="width:400px"><span data-m="t" style="display:inline-block;margin:0 2em;font-size:{fs}">{S381}</span></div>"#
            )
        };
        let mut doc = lay_out(&mk("12px"));
        let t = find(&doc, "t");
        let mut h = 601.0;
        for fs in ["16px", "13px", "17px", "12px"] {
            doc.set_style(t, "font-size", fs);
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(&format!("H6 fs {fs}"), &doc, &mk(fs), &mut bad);
        }
    }
    // H7: containing block of a holding box at width 0 and back.
    {
        let mk = |w: &str| {
            format!(
                r#"<div data-m="f" style="width:{w}"><span data-m="o" style="display:inline-flex">head <span data-m="i" style="display:inline-block;width:50%">Wavy milliliters</span></span></div>"#
            )
        };
        let mut doc = lay_out(&mk("400px"));
        let f = find(&doc, "f");
        let mut h = 601.0;
        for w in ["300px", "400px", "250px", "500px"] {
            doc.set_style(f, "width", w);
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(&format!("H7 cb {w}"), &doc, &mk(w), &mut bad);
        }
    }
    // H8: a fitting auto box whose block narrows below it, then widens, with
    // no percentage or keyword box anywhere at the start (maps empty).
    {
        let mk = |w: u32| {
            format!(
                r#"<div data-m="f" style="width:{w}px"><span data-m="t" style="display:inline-block">{S381}</span><span data-m="u" style="display:inline-block">short</span></div>"#
            )
        };
        let mut doc = lay_out(&mk(500));
        let f = find(&doc, "f");
        let mut h = 601.0;
        for w in [300u32, 500, 380, 382, 200, 600] {
            doc.set_style(f, "width", &format!("{w}px"));
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(&format!("H8 cb {w}"), &doc, &mk(w), &mut bad);
        }
    }
    assert_eq!(bad, 0);
}

/// Round 2, finding 1: a containing block that goes to width 0 leaves the
/// box at the size its last positive width gave it (an incremental pass),
/// where a fresh layout measures it as `auto` (max-content).
#[test]
fn round2_zero_width_block_history() {
    let mut bad = 0;
    {
        let mk = |w: &str| {
            format!(
                r#"<div data-m="f" style="width:{w}"><span data-m="o" style="display:inline-flex">head <span data-m="i" style="display:inline-block;width:50%">Wavy milliliters</span></span></div>"#
            )
        };
        let mut doc = lay_out(&mk("400px"));
        let f = find(&doc, "f");
        let mut h = 601.0;
        for w in ["0px", "400px", "0px", "300px"] {
            doc.set_style(f, "width", w);
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(&format!("H7 cb {w}"), &doc, &mk(w), &mut bad);
        }
    }
    let mk = |w: &str| {
        format!(
            r#"<div data-m="f" style="width:{w}"><span data-m="t" style="display:inline-block">{S381}</span></div>"#
        )
    };
    let mut doc = lay_out(&mk("300px"));
    let f = find(&doc, "f");
    let mut h = 601.0;
    for w in ["0px", "300px", "0px", "500px", "0px"] {
        doc.set_style(f, "width", w);
        doc.resolve_layout(800.0, h);
        h += 1.0;
        check(&format!("Z cb {w}"), &doc, &mk(w), &mut bad);
    }
    assert_eq!(bad, 0);
}

#[test]
fn round2_tolerance_probe() {
    for w in [
        "2000px", "381px", "380.5px", "380px", "379.8px", "379.5px", "379px",
    ] {
        let doc = lay_out(&format!(
            r#"<div data-m="f" style="width:{w}"><span data-m="t" style="display:inline-block">{S381}</span></div>"#
        ));
        let t = query_selector(&doc.tree, "[data-m=t]")[0];
        let n = doc.tree.get(t).unwrap();
        let u = doc
            .tree
            .taffy
            .unrounded_layout(n.taffy_id.unwrap())
            .size
            .width;
        println!("TOL cb={w} t={} unrounded={u}", dump(&doc));
    }
}

/// Round 2: the skip for a box already resolved at its block's width is
/// pinned by cost, not geometry (dropping it re-measures every capped box on
/// every pass and lands on the same sizes).
#[test]
fn round2_a_resolved_box_is_not_measured_again_by_an_unrelated_relayout() {
    let mut doc = lay_out(&format!(
        r#"<div style="width:400px"><span data-m="t" style="display:inline-block">{L}</span><span data-m="u" style="display:inline-block;width:fit-content">{L}</span></div><div data-m="f" style="height:10px"></div>"#
    ));
    let f = find(&doc, "f");
    doc.tree.perf.end_frame();
    doc.set_style(f, "height", "30px");
    doc.resolve_layout(800.0, 600.0);
    let n = doc
        .tree
        .perf
        .get(rinch_dom::perf::Counter::InlineBlockComputes);
    println!("R2 COST inline_block_computes={n} [{}]", dump(&doc));
    assert_eq!(dump(&doc), "f=800.0x30.0 t=400.0x40.0 u=400.0x40.0");
    assert_eq!(n, 0);
}

/// Round 2: "holds a resolved box" when the only resolved boxes are capped
/// `auto` ones (keyword entries, no percentage-sized atomic inline).
#[test]
fn round2_holds_a_capped_box_histories() {
    let mut bad = 0;
    for (outer, mid) in [
        ("display:inline-block", "width:80%"),
        ("display:inline-flex;flex-direction:column", "width:80%"),
        ("display:inline-block", "width:300px"),
        ("display:inline-grid", "width:80%"),
        ("display:inline-block", "max-width:250px"),
    ] {
        let mk = |w: u32, inner: &str, extra: &str| {
            format!(
                r#"<div data-m="f" style="width:{w}px"><span data-m="o" style="{outer}">head{extra}<div data-m="a" style="{mid}"><span data-m="t" style="display:inline-block">{inner}</span></div></span></div>"#
            )
        };
        // cb restyles
        let mut doc = lay_out(&mk(400, L, ""));
        let f = find(&doc, "f");
        let mut h = 601.0;
        for w in [300u32, 700, 250, 400] {
            doc.set_style(f, "width", &format!("{w}px"));
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(
                &format!("HC {outer}|{mid} cb {w}"),
                &doc,
                &mk(w, L, ""),
                &mut bad,
            );
        }
        // inner text edit short -> long -> short
        let t = find(&doc, "t");
        for txt in ["short", L, "Wavy milliliters WWW"] {
            doc.set_text_content(t, txt);
            doc.resolve_layout(800.0, h);
            h += 1.0;
            check(
                &format!("HC {outer}|{mid} txt {txt:.6}"),
                &doc,
                &mk(400, txt, ""),
                &mut bad,
            );
        }
        // sibling text appended inside the outer box
        let o = find(&doc, "o");
        let tx = doc.create_text(" more");
        let a = find(&doc, "a");
        doc.insert_before(o, tx, a);
        doc.resolve_layout(800.0, h);
        let mut fresh = mount(&mk(400, "Wavy milliliters WWW", ""));
        let (fo, fa) = (find(&fresh, "o"), find(&fresh, "a"));
        let ft = fresh.create_text(" more");
        fresh.insert_before(fo, ft, fa);
        fresh.resolve_layout(800.0, 600.0);
        let (x, y) = (dump(&doc), dump(&fresh));
        println!("R2 HC {outer}|{mid} sib inc[{x}] fresh-same-dom[{y}]");
        if x != y {
            bad += 1;
        }
    }
    assert_eq!(bad, 0);
}
