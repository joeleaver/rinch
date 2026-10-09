//! Review of PR #1488 (#1476) — fixtures for the findings.
//!
//! Every `chrome` string is Chrome 153 (headless, standards mode, the bundled
//! Inter as `ProbeFace`, `16px/20px`, `* { box-sizing: border-box }`,
//! `getBoundingClientRect`). Each test lays the document out, and then lays it
//! out again with nothing changed but the viewport height: a layout that is
//! not the same the second time is one frame of wrong geometry followed by a
//! jump on the next unrelated relayout.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const L: &str = "Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters";

fn mount(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "style", "font: 16px/20px ProbeFace");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc
}

/// `m=WxH` for every `[data-m]`, in tree order.
fn dump(doc: &RinchDocument) -> String {
    query_selector(&doc.tree, "[data-m]")
        .into_iter()
        .map(|id| {
            let n = doc.tree.get(id).unwrap();
            format!(
                "{}={:.1}x{:.1}",
                n.attributes.get("data-m").unwrap(),
                n.layout.width,
                n.layout.height
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
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

/// First layout, and the same document laid out again.
fn twice(html: &str) -> (String, String) {
    let mut doc = mount(html);
    doc.resolve_layout(800.0, 600.0);
    let first = dump(&doc);
    doc.resolve_layout(800.0, 601.0);
    (first, dump(&doc))
}

fn check(cases: &[(&str, String, &str)]) {
    let mut bad = vec![];
    for (name, html, chrome) in cases {
        let (first, second) = twice(html);
        if first != second {
            bad.push(format!(
                "{name}: NOT STABLE first[{first}] second[{second}] chrome[{chrome}]"
            ));
        } else if !close(chrome, &first) {
            bad.push(format!("{name}: rinch[{first}] chrome[{chrome}]"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Finding 1 — the four-pass cap. An `auto` atomic inline inside a flex item
/// inside an `auto` atomic inline …, four boxes deep: the first layout leaves
/// **every** box at its max-content width (544 x 20, main's answer) and the
/// next layout — any layout, with nothing changed — caps them all.
#[test]
fn f1_boxes_nested_in_flex_items_four_deep_are_capped_by_the_first_layout() {
    let t = format!(r#"<span data-m="t" style="display:inline-block">{L}</span>"#);
    let ib =
        |inner: &str| format!(r#"<span data-m="b" style="display:inline-block">{inner}</span>"#);
    let fx =
        |inner: &str| format!(r#"<div style="display:flex"><div data-m="f">{inner}</div></div>"#);
    // c > flex > item > ib > flex > item > ib > flex > item > ib > t
    let deep6 = fx(&ib(&fx(&ib(&fx(&ib(&t))))));
    // c > ib > flex > item > ib > flex > item > ib > flex > item > t
    let deep6b = ib(&fx(&ib(&fx(&ib(&fx(&t))))));
    let _ = &deep6b;
    check(&[(
        "ib_in_flex_item_x3",
        format!(r#"<div data-m="c" style="width:400px">{deep6}</div>"#),
        "c=400.0x40.0 f=400.0x40.0 b=400.0x40.0 f=400.0x40.0 b=400.0x40.0 f=400.0x40.0 b=400.0x40.0 t=400.0x40.0",
    )]);
}

/// Finding 1, second half: the same nesting starting with the box
/// (`ib > flex > item > ib > flex > item > ib > flex > item > ib`). No layout
/// caps these — every one is 544 x 20 however often the document is laid out
/// (main's answer), and raising the pass cap does not change it. Three boxes
/// deep (one level less) is capped.
#[test]
fn f1b_four_boxes_each_holding_a_flex_item_are_capped() {
    let t = format!(r#"<span data-m="t" style="display:inline-block">{L}</span>"#);
    let ib =
        |inner: &str| format!(r#"<span data-m="b" style="display:inline-block">{inner}</span>"#);
    let fx =
        |inner: &str| format!(r#"<div style="display:flex"><div data-m="f">{inner}</div></div>"#);
    let deep6b = ib(&fx(&ib(&fx(&ib(&fx(&t))))));
    check(&[(
        "flex_item_in_ib_x3",
        format!(r#"<div data-m="c" style="width:400px">{deep6b}</div>"#),
        "c=400.0x40.0 b=400.0x40.0 f=400.0x40.0 b=400.0x40.0 f=400.0x40.0 b=400.0x40.0 f=400.0x40.0 t=400.0x40.0",
    )]);
}

/// Finding 2 — a regression from main. An `auto` atomic inline that holds a
/// percentage-width block child (#662), inside a flex item: the first layout
/// resolves the child's percentage against nothing (it comes out as wide as
/// the box) and the next layout puts it right. main gives 9 both times.
#[test]
fn f2_a_percentage_child_of_a_box_in_a_flex_item_is_resolved_by_the_first_layout() {
    check(&[(
        "pct_child_in_flex_item",
        r#"<div data-m="c" style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block">ab<div data-m="i" style="width:50%;height:5px"></div></span></div></div>"#.to_string(),
        "c=400.0x25.0 f=18.8x25.0 t=18.8x25.0 i=9.4x5.0",
    )]);
}

/// Finding 3 — the height recorded with a min-content width is the height of
/// the box's interior laid out under min-content *available space*, which is
/// not the box's height at its min-content **width** when that width comes
/// from something other than the text (a `min-width`, a fixed-width child):
/// the text is broken at every word there and is one or two lines in the box.
/// A container that measures the line at a width other than the one the box
/// is then resolved against (a shrink-to-fit absolute box) keeps that height.
#[test]
fn f3_a_box_at_its_min_width_is_not_as_tall_as_its_text_broken_at_every_word() {
    check(&[(
        "abs_min_width",
        format!(
            r#"<div data-m="c" style="width:400px;position:relative;height:90px"><div data-m="f" style="position:absolute"><span data-m="t" style="display:inline-block;min-width:450px">{L}</span></div></div>"#
        ),
        "c=400.0x90.0 f=450.0x40.0 t=450.0x40.0",
    )]);
}

fn one(doc: &RinchDocument, m: &str) -> rinch_core::dom::NodeId {
    rinch_core::dom::NodeId(query_selector(&doc.tree, &format!("[data-m={m}]"))[0])
}

/// Probe — a change inside a box that moves its min-content width without
/// touching its text: incremental equals fresh.
#[test]
fn p4_a_non_text_change_inside_a_box_drops_its_min_content_size() {
    let html = |iw: u32, box_style: &str, child_style: &str| {
        format!(
            r#"<div data-m="c" style="width:100px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;{box_style}">Wavy milliliters WWW<div data-m="i" style="width:{iw}px;height:5px;{child_style}"></div></span></div><div data-m="g">side text</div></div>"#
        )
    };
    let mut bad = vec![];
    // (what, mutate, fresh html)
    type Step = (
        &'static str,
        fn(&mut RinchDocument),
        (u32, &'static str, &'static str),
    );
    let steps: &[Step] = &[
        (
            "child width 120->200",
            |d| {
                let i = one(d, "i");
                d.set_style(i, "width", "200px")
            },
            (200, "", ""),
        ),
        (
            "child width 120->20",
            |d| {
                let i = one(d, "i");
                d.set_style(i, "width", "20px")
            },
            (20, "", ""),
        ),
        (
            "box padding",
            |d| {
                let t = one(d, "t");
                d.set_style(t, "padding", "0 25px")
            },
            (120, "padding:0 25px", ""),
        ),
        (
            "box min-width",
            |d| {
                let t = one(d, "t");
                d.set_style(t, "min-width", "180px")
            },
            (120, "min-width:180px", ""),
        ),
        (
            "box nowrap",
            |d| {
                let t = one(d, "t");
                d.set_style(t, "white-space", "nowrap")
            },
            (120, "white-space:nowrap", ""),
        ),
        (
            "box border",
            |d| {
                let t = one(d, "t");
                d.set_style(t, "border", "9px solid")
            },
            (120, "border:9px solid", ""),
        ),
        (
            "box font-size",
            |d| {
                let t = one(d, "t");
                d.set_style(t, "font-size", "24px")
            },
            (120, "font-size:24px", ""),
        ),
        (
            "box letter-spacing",
            |d| {
                let t = one(d, "t");
                d.set_style(t, "letter-spacing", "3px")
            },
            (120, "letter-spacing:3px", ""),
        ),
        (
            "child margin",
            |d| {
                let i = one(d, "i");
                d.set_style(i, "margin-left", "90px")
            },
            (120, "", "margin-left:90px"),
        ),
        (
            "child padding",
            |d| {
                let i = one(d, "i");
                d.set_style(i, "padding-left", "190px")
            },
            (120, "", "padding-left:190px"),
        ),
        (
            "child display none",
            |d| {
                let i = one(d, "i");
                d.set_style(i, "display", "none")
            },
            (120, "", "display:none"),
        ),
    ];
    for (what, mutate, (iw, bs, cs)) in steps {
        let mut doc = mount(&html(120, "", ""));
        doc.resolve_layout(800.0, 600.0);
        mutate(&mut doc);
        doc.resolve_layout(800.0, 601.0);
        let inc = dump(&doc);
        let mut fresh = mount(&html(*iw, bs, cs));
        fresh.resolve_layout(800.0, 600.0);
        let fresh = dump(&fresh);
        if inc != fresh {
            bad.push(format!("{what}: incremental[{inc}] fresh[{fresh}]"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Finding 3, in a plain block: a box whose `min-width` (or a fixed-width
/// child) makes it wider than its block container. A fresh layout is right;
/// after the container is resized, the block is as tall as the box's text
/// broken at every word (200) around a 40px box.
#[test]
fn f3b_resizing_a_block_around_a_box_at_its_min_width_keeps_the_blocks_height() {
    let cases: &[(&str, String)] = &[
        (
            "min_width",
            format!(r#"<div data-m="c" style="width:400px"><div data-m="f"><span data-m="t" style="display:inline-block;min-width:450px">{L}</span></div></div>"#),
        ),
        (
            "fixed_child",
            r#"<div data-m="c" style="width:100px"><div data-m="f"><span data-m="t" style="display:inline-flex;flex-direction:column"><div data-m="i" style="width:150px;height:10px"></div><span>Wavy milliliters WWW mmm Wavy</span></span></div></div>"#.to_string(),
        ),
        (
            "min_width_flex_item_min0",
            r#"<div data-m="c" style="width:100px;display:flex"><div data-m="f" style="min-width:0"><span data-m="t" style="display:inline-block;min-width:150px">Wavy milliliters WWW mmm</span></div></div>"#.to_string(),
        ),
    ];
    let mut bad = vec![];
    for (name, html) in cases {
        let mut doc = mount(html);
        doc.resolve_layout(800.0, 600.0);
        let c = one(&doc, "c");
        for (n, w) in [90, 263, 95].into_iter().enumerate() {
            doc.set_style(c, "width", &format!("{w}px"));
            doc.resolve_layout(800.0, 601.0 + n as f32);
            let inc = dump(&doc);
            let mut fresh = mount(html);
            let fc = one(&fresh, "c");
            fresh.set_style(fc, "width", &format!("{w}px"));
            fresh.resolve_layout(800.0, 600.0);
            let fresh = dump(&fresh);
            if inc != fresh {
                bad.push(format!("{name} at {w}: incremental[{inc}] fresh[{fresh}]"));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Finding 2, the shape an app would write: a `nowrap` chip with a
/// percentage-width bar under its label, in a flex item and in a grid item.
/// (A box whose min-content width is its max-content width — one word, or
/// `nowrap` — is the case: nothing else changes, so no later pass resolves
/// the percentage the min-content measure dropped.)
#[test]
fn f2b_a_nowrap_chip_with_a_percentage_bar_in_a_flex_or_grid_item() {
    let chip = r#"<span data-m="t" style="display:inline-flex;flex-direction:column;white-space:nowrap;padding:0 10px">Needs review<span data-m="i" style="width:50%;height:2px"></span></span>"#;
    check(&[
        (
            "flex_item",
            format!(
                r#"<div data-m="c" style="width:400px;display:flex"><div data-m="f">{chip}</div></div>"#
            ),
            "c=400.0x22.0 f=123.2x22.0 t=123.2x22.0 i=51.6x2.0",
        ),
        (
            "grid_item",
            format!(
                r#"<div data-m="c" style="width:400px;display:grid;grid-template-columns:auto 50px"><div data-m="f">{chip}</div><div>x</div></div>"#
            ),
            "c=400.0x22.0 f=350.0x22.0 t=123.2x22.0 i=51.6x2.0",
        ),
    ]);
}

/// Finding 3 in its commonest form. A box's min-content *height* is recorded
/// as the height of its text broken at **every** opportunity (how a
/// min-content measure lays it out), and a line narrower than the box lines it
/// up at that height. At its min-content *width* — its longest word — the
/// short words share lines: "Go to the documentation" is 2 lines at 113px, and
/// 4 lines broken at every word.
///
/// First layout: a shrink-to-fit absolute box and a `flex-start` column item
/// are 80 tall around the 40px box (Chrome 40), and two flex items around
/// "a b c d e f milliliters" are 140 tall around 60px boxes (Chrome 60).
#[test]
fn f3c_a_container_narrower_than_a_box_is_as_tall_as_the_box() {
    let goto = r#"<span data-m="t" style="display:inline-block">Go to the documentation</span>"#;
    let abc = |m: &str| {
        format!(r#"<span data-m="{m}" style="display:inline-block">a b c d e f milliliters</span>"#)
    };
    check(&[
        (
            "abs",
            format!(
                r#"<div data-m="c" style="width:100px;position:relative;height:90px"><div data-m="f" style="position:absolute">{goto}</div></div>"#
            ),
            "c=100.0x90.0 f=113.2x40.0 t=113.2x40.0",
        ),
        (
            "column_flex_start",
            format!(
                r#"<div data-m="c" style="width:100px;display:flex;flex-direction:column;align-items:flex-start"><div data-m="f">{goto}</div></div>"#
            ),
            "c=100.0x40.0 f=113.2x40.0 t=113.2x40.0",
        ),
        (
            "two_flex_items",
            format!(
                r#"<div data-m="c" style="width:100px;display:flex"><div data-m="f">{}</div><div data-m="g" style="min-width:0">{}</div></div>"#,
                abc("t"),
                abc("u")
            ),
            "c=100.0x60.0 f=66.1x60.0 t=66.1x60.0 g=33.9x60.0 u=66.1x60.0",
        ),
    ]);
}

/// The same after a resize, **in a plain block** (and a `min-width: 0` flex
/// item, and a grid item): a fresh layout is right (Chrome: the block is
/// 40 tall at 100px and at 99px) and main is right; here the block is 80 tall
/// around the 40px box once its width has changed.
#[test]
fn f3d_resizing_a_block_narrower_than_a_box_keeps_the_blocks_height() {
    let goto = r#"<span data-m="t" style="display:inline-block">Go to the documentation</span>"#;
    let cases = [
        (
            "block",
            format!(r#"<div data-m="c" style="width:100px"><div data-m="f">{goto}</div></div>"#),
        ),
        (
            "flex_item_min_width_0",
            format!(
                r#"<div data-m="c" style="width:100px;display:flex"><div data-m="f" style="min-width:0">{goto}</div></div>"#
            ),
        ),
        (
            "grid_item_min_width_0",
            format!(
                r#"<div data-m="c" style="width:100px;display:grid;grid-template-columns:auto auto"><div data-m="f" style="min-width:0">{goto}</div><div>x</div></div>"#
            ),
        ),
    ];
    let mut bad = vec![];
    for (name, html) in &cases {
        let mut doc = mount(html);
        doc.resolve_layout(800.0, 600.0);
        let c = one(&doc, "c");
        for (n, w) in [99, 60, 100].into_iter().enumerate() {
            doc.set_style(c, "width", &format!("{w}px"));
            doc.resolve_layout(800.0, 601.0 + n as f32);
            let inc = dump(&doc);
            let mut fresh = mount(html);
            let fc = one(&fresh, "c");
            fresh.set_style(fc, "width", &format!("{w}px"));
            fresh.resolve_layout(800.0, 600.0);
            let fresh = dump(&fresh);
            if inc != fresh {
                bad.push(format!("{name} at {w}: incremental[{inc}] fresh[{fresh}]"));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
