//! Review of PR #1587 (#1122): rinch's tab stops against Chrome 155 on
//! adversarial shapes, and the rebuild loop's cap.
//!
//! Not pinned here: a long soft-wrapped `pre-wrap` paragraph can stop at
//! `MAX_TAB_BUILDS` with some tabs off their stops, 3.8–5px right of
//! Chrome's (`pairs(30, 1)` at 300px: 7 of 30 tabs). Refs #1593.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::dom_cursor_to_ifc_offset;

#[path = "review_1587/chrome_cases.rs"]
mod chrome;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d
}

fn text_nodes(d: &RinchDocument, id: u64, out: &mut Vec<u64>) {
    let n = d.tree.get(id).unwrap();
    if matches!(n.kind, rinch_dom::node::NodeKind::Text(_)) {
        out.push(id);
    }
    for &c in &n.children {
        text_nodes(d, c, out);
    }
}

fn block(style: &str, html: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0");
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("font: 16px/24px ProbeFace; width: 600px; {style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, html);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

fn carets_of(d: &RinchDocument, c: NodeId) -> Vec<(f32, f32)> {
    let mut texts = Vec::new();
    text_nodes(d, c.0, &mut texts);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let mut out = Vec::new();
    for t in texts {
        let len = match &d.tree.get(t).unwrap().kind {
            rinch_dom::node::NodeKind::Text(td) => td.content.len(),
            _ => unreachable!(),
        };
        let mut o = 0;
        let content = match &d.tree.get(t).unwrap().kind {
            rinch_dom::node::NodeKind::Text(td) => td.content.clone(),
            _ => unreachable!(),
        };
        // one caret per UTF-16 unit (Chrome's offsets); all probe chars are BMP
        let mut offs = vec![];
        for (i, _) in content.char_indices() {
            offs.push(i);
        }
        offs.push(len);
        for b in offs {
            let Some(flat) = dom_cursor_to_ifc_offset(&il.text_ranges, t, b) else {
                continue;
            };
            out.push(d.query_caret_position(c.0, flat).unwrap());
            o += 1;
        }
        let _ = o;
    }
    out
}

/// `yyy\tzz` pairs separated by spaces, word lengths from a fixed LCG.
fn pairs(n: u64, seed: u64) -> String {
    let mut s = seed;
    let mut next = |k: u64| {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        1 + ((s >> 33) % k) as usize
    };
    (0..n)
        .map(|_| format!("{}\t{}", "y".repeat(next(5)), "z".repeat(next(5))))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The carets just after each tab, in an LTR, start-aligned block with no
/// spacing: each must sit on a multiple of 36 (8 Inter spaces at 16px).
/// Returns the tabs, how many are off their stops, and `ifc_tab_rebuilds`
/// for the layout.
fn off_stop_tabs(text: &str, style: &str) -> (usize, usize, u64) {
    let (d, c) = block(style, text);
    let mut texts = Vec::new();
    text_nodes(&d, c.0, &mut texts);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let t = texts[0];
    let (mut tabs, mut off) = (0, 0);
    for (i, ch) in text.char_indices() {
        if ch != '\t' {
            continue;
        }
        tabs += 1;
        let flat = dom_cursor_to_ifc_offset(&il.text_ranges, t, i + 1).unwrap();
        let (x, _) = d.query_caret_position(c.0, flat).unwrap();
        let r = x.rem_euclid(36.0);
        // A tab whose end is the start of the next line (hung) reads 0.
        if r > 0.05 && 36.0 - r > 0.05 {
            off += 1;
        }
    }
    (
        tabs,
        off,
        d.tree.perf.get(rinch_dom::perf::Counter::IfcTabRebuilds),
    )
}

/// 15 `word\tword` pairs in a 300px `pre-wrap` block wrap, and each build moves one more break that the tab widths depend on: the loop
/// settles in exactly `MAX_TAB_BUILDS` (4) builds, three rebuilds per call
/// (the root compute's measure and the paint layout), and every tab is at
/// its stop. With a cap of 3 tabs stay off their stops. Longer paragraphs
/// need more builds than the cap allows (Refs #1593).
#[test]
fn a_paragraph_that_settles_in_the_last_build_has_every_tab_at_its_stop() {
    let (tabs, off, rebuilds) = off_stop_tabs(&pairs(15, 1), "white-space: pre-wrap; width: 300px");
    assert_eq!(tabs, 15);
    assert_eq!(off, 0, "tabs off their stops");
    assert_eq!(rebuilds, 6, "ifc_tab_rebuilds");
}

fn pixels(style: &str, html: &str) -> Vec<[u8; 4]> {
    let (mut d, _c) = block(style, html);
    let mut painter = rinch_dom::paint::skia_painter::TinySkiaPainter::new(800, 600);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &d.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut d.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Inked pixels (`pred`) in the rectangle x0..x1, y0..y1.
fn count(
    px: &[[u8; 4]],
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    pred: impl Fn(&[u8; 4]) -> bool,
) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if pred(&px[y * 800 + x]) {
                n += 1;
            }
        }
    }
    n
}

fn dark(p: &[u8; 4]) -> bool {
    p[3] > 0 && p[0] < 128 && p[1] < 128 && p[2] < 128
}
fn red(p: &[u8; 4]) -> bool {
    p[3] > 0 && p[0] > 200 && p[1] < 60 && p[2] < 60
}

/// Chrome draws an underline and a span's background across a tab's whole
/// width (8.98..36 here). x 14..32 is inside the tab only.
#[test]
fn decorations_and_backgrounds_span_the_tab() {
    let u = pixels(
        "white-space: pre; text-decoration: underline; color: black",
        "a\tb",
    );
    let under = count(&u, 14, 32, 0, 24, dark);
    let bg = pixels(
        "white-space: pre; color: black",
        "a<span style=\"background: rgb(255,0,0)\">\t</span>b",
    );
    let bgn = count(&bg, 14, 32, 0, 24, red);
    let bg2 = pixels(
        "white-space: pre; color: black",
        "<span style=\"background: rgb(255,0,0)\">a\tb</span>",
    );
    let bgn2 = count(&bg2, 14, 32, 0, 24, red);
    // Positive control: the `a` glyph is inked.
    let a = count(&u, 0, 9, 0, 20, dark);
    println!(
        "underline px in tab: {under}; tab-only span bg px: {bgn}; span bg px: {bgn2}; a glyph px: {a}"
    );
    assert!(a > 0);
    assert!(under >= 18, "underline across the tab: {under}");
    assert!(
        bgn >= 18 * 10,
        "background of a span holding only a tab: {bgn}"
    );
    assert!(bgn2 >= 18 * 10, "background of a span across a tab: {bgn2}");
}

/// The probe cases rinch matches Chrome 155 on at the PR head, as a pin:
/// every caret on Chrome's line within 0.05px. Excluded, with the reason in
/// the review: `bidi` (mixed-direction line), `justify` (carets after a letter-spaced cluster in
/// a justified line), `cascade2_300` (MAX_TAB_BUILDS cap, Refs #1593).
#[test]
fn matches_chrome_on_the_sampled_cases() {
    for (name, style, html, want) in chrome::CASES {
        if ["bidi", "justify", "cascade2_300"].contains(name) {
            continue;
        }
        let (d, c) = block(style, html);
        let got = carets_of(&d, c);
        assert_eq!(got.len(), want.len(), "{name}");
        for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            // #1426 part 1: `|x` (inside the span) and `|\tz` (after it) each
            // share a flat byte with the position across the padding (`a|`,
            // `y|`), and a byte's caret is drawn before the edge box at it:
            // `|x` outside the start padding, `|\tz` inside the end padding.
            // Chrome draws each DOM position on its own side. The caret rule
            // is part 2.
            if *name == "bg_span" && (i == 2 || i == 6) {
                continue;
            }
            let gl = (g.1 / 24.0).floor();
            if (gl - w.1).abs() > 0.5 {
                // Chrome draws the caret at a soft wrap at the upper line's end.
                assert!(g.0 == 0.0 && gl == w.1 + 1.0, "{name} #{i}: {g:?} vs {w:?}");
                continue;
            }
            assert!(
                (g.0 - w.0).abs() <= 0.05,
                "{name} #{i}: {g:?} vs Chrome {w:?}"
            );
        }
    }
}

/// An out-of-flow box before a tab takes no room on the line (Chrome: the
/// tab ends at 36 after `a`), whatever the box's width.
#[test]
fn an_out_of_flow_box_before_a_tab_takes_no_room() {
    for pos in ["absolute", "fixed"] {
        let html = format!(
            "a<span style=\"position: {pos}; display: inline-block; width: 50px; height: 5px\"></span>\tb"
        );
        let (d, c) = block("white-space: pre", &html);
        let xs: Vec<f32> = carets_of(&d, c).into_iter().map(|c| c.0).collect();
        assert!((xs[3] - 36.0).abs() < 0.05, "{pos}: {xs:?}");
    }
}

/// Incremental layout after edits equals a fresh layout of the final state,
/// for a wrapping paragraph with tabs (the cap included: `pairs(60, 3)`
/// does not settle in four builds).
#[test]
fn an_incremental_layout_with_tabs_matches_a_fresh_one() {
    let final_text = pairs(60, 3);
    let style = "white-space: pre-wrap; width: 300px";
    // Incremental: start from another text and width, then edit.
    let mut d = doc();
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0");
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("font: 16px/24px ProbeFace; width: 600px; {style}"),
    );
    d.append_child(body, c);
    let lead = d.create_element("span");
    let lead_t = d.create_text("q\tq");
    d.append_child(lead, lead_t);
    d.append_child(c, lead);
    let t = d.create_text(&pairs(60, 1));
    d.append_child(c, t);
    d.resolve_layout(800.0, 600.0);
    d.set_text_content(t, &final_text);
    d.resolve_layout(810.0, 600.0);
    d.set_text_content(lead_t, "");
    d.resolve_layout(800.0, 600.0);
    let inc = carets_of(&d, c);
    // Fresh.
    let (f, fc) = block(style, &format!("<span></span>{final_text}"));
    let fresh = carets_of(&f, fc);
    assert_eq!(inc.len(), fresh.len());
    for (i, (a, b)) in inc.iter().zip(&fresh).enumerate() {
        assert!(
            (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01,
            "#{i}: {a:?} vs fresh {b:?}"
        );
    }
}
