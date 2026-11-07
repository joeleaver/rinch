//! Follow-up to #2309/#2313: three placeholder bugs Postrinch found in a real
//! held-back mail (every `<img>` without `src`). Each case is Chrome 156's
//! (headless, standards mode, the bundled Inter as `@font-face`, the same
//! markup at `line-height: 1.4`); rows are measured from each box's top.
//!
//! - An inline alt placeholder is at least 16px tall: Chrome's alt container
//!   holds its 16x16 broken-image icon (rinch draws no icon but keeps the
//!   box), so a cell's `line-height: 0` or `10px`, or `font-size: 0`, does
//!   not collapse it. Its text is laid out from the box's top and clipped to
//!   the box, as Chrome's `overflow: hidden` container clips it: at
//!   `line-height: 0` only the glyphs' lower half shows (Chrome rows 0..8).
//! - Every box's painted extent is inside its box, and the next block starts
//!   at its bottom.
//! - An empty or invalid `height` attribute is ignored (HTML's rules for
//!   dimension values), on a loaded picture and a placeholder alike: Chrome
//!   lays a 3x2 picture with `width=30 height=""` out 30x20.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const W: u32 = 400;
const H: u32 = 380;
const RED_3X2: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";

/// Chrome's page (`.scratch/chrome/p4.html` of the PR): each case in a
/// 300px column at `left: 10px`, a 10px red block after it.
fn page() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("I"),
            ..Default::default()
        }),
    );
    doc.load_css(
        "html, body { margin: 0; background: #fff; font-family: I; font-size: 16px; \
         line-height: 1.4; color: #000 } \
         .r { position: absolute; left: 10px; width: 300px } \
         .n { height: 10px; background: #f00 }",
    );
    let body = doc.body();
    doc.set_inner_html(
        body,
        &format!(
            r#"<div class=r style="top:10px"><img id=a alt="Logo Brand" width=270 style="display:block;width:100%;line-height:0"><div class=n id=an></div></div>
<div class=r style="top:70px"><img id=b alt="Logo Brand" width=270 style="display:block;width:100%;line-height:10px"><div class=n id=bn></div></div>
<div class=r style="top:130px"><img id=c alt="Logo Brand" width=270 style="display:block;width:100%;font-size:30px;line-height:20px"><div class=n id=cn></div></div>
<div class=r style="top:190px;font-size:0;line-height:0"><img id=d alt="Instagram" width=40 height="" style="width:40px;height:auto;display:block"><div class=n id=dn></div></div>
<div class=r style="top:250px"><img id=e alt="Logo Brand" width=270 style="display:block;width:100%"><div class=n id=en></div></div>
<div class=r style="top:310px"><img id=f src="{RED_3X2}" width=30 height="" style="display:block"><img id=g src="{RED_3X2}" width=30 height="abc" style="display:block"><img id=h alt="x" width=30 height="" style="display:block"></div>"#
        ),
    );
    doc.resolve_layout(W as f32, H as f32);
    doc
}

fn node(doc: &RinchDocument, id: &str) -> u64 {
    rinch_dom::query::query_all(&doc.tree, doc.tree.root_id, &format!("#{id}"))[0]
}

/// `(x, y, width, height)` on screen.
fn rect(doc: &RinchDocument, id: &str) -> (f64, f64, f32, f32) {
    let n = node(doc, id);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n, 1.0);
    let l = &doc.tree.nodes[n].layout;
    (x, y, l.width, l.height)
}

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    let mut p = TinySkiaPainter::new(W, H);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut lcx,
    );
    p
}

/// The rows (from `top`) holding dark text ink between `top - 8` and
/// `top + 40`, in the column.
fn ink_rows(p: &TinySkiaPainter, top: u32) -> Option<(u32, u32)> {
    let d = p.pixels();
    let mut out: Option<(u32, u32)> = None;
    for y in top - 8..top + 40 {
        for x in 10..300 {
            let i = ((y * W + x) * 4) as usize;
            if d[i] < 140 && d[i + 1] < 140 && d[i + 2] < 140 {
                let r = y as i64 - top as i64;
                assert!(r >= 0, "ink above its box at row {r}");
                let r = r as u32;
                out = Some(out.map_or((r, r), |(a, b)| (a.min(r), b.max(r))));
            }
        }
    }
    out
}

#[test]
fn inline_alt_boxes_are_chromes_size_and_hold_their_ink() {
    let mut doc = page();
    let p = paint(&mut doc);
    // (id, top, Chrome's box height, Chrome's ink rows; None: none)
    for (id, top, h, ink) in [
        ("a", 10u32, 16.0, Some((0, 8))),
        ("b", 70, 16.0, Some((0, 13))),
        ("c", 130, 20.0, Some((0, 19))),
        ("d", 190, 16.0, None),
        ("e", 250, 22.0, Some((5, 19))),
    ] {
        let (_, y, _, bh) = rect(&doc, id);
        assert_eq!((y, bh), (f64::from(top), h), "#{id}: Chrome's box");
        let got = ink_rows(&p, top);
        match (got, ink) {
            (None, None) => {}
            (Some(g), Some(c)) => assert!(
                g.0.abs_diff(c.0) <= 1 && g.1.abs_diff(c.1) <= 1,
                "#{id}: ink rows {g:?}, Chrome {c:?}"
            ),
            _ => panic!("#{id}: ink rows {got:?}, Chrome {ink:?}"),
        }
        if let Some(g) = got {
            assert!(g.1 < h as u32, "#{id}: ink {g:?} inside its {h}px box");
        }
        // The next block starts at its bottom.
        let (_, ny, _, _) = rect(&doc, &format!("{id}n"));
        assert_eq!(ny, y + f64::from(bh), "#{id}: the next block");
    }
    assert_eq!(rect(&doc, "d").2, 40.0, "#d keeps its declared width");
}

#[test]
fn an_empty_or_invalid_height_attribute_is_ignored() {
    let doc = page();
    // Chrome 156: 30x20 for both (the picture's own 3:2).
    assert_eq!(rect(&doc, "f").2..=rect(&doc, "f").3, 30.0..=20.0);
    assert_eq!(rect(&doc, "g").2..=rect(&doc, "g").3, 30.0..=20.0);
    // A placeholder with `height=""` is not 0 tall: its alt text, one line.
    let (_, _, w, h) = rect(&doc, "h");
    assert_eq!(w, 30.0);
    assert!(h >= 16.0, "{h}");
    let n = node(&doc, "h");
    let _ = NodeId(n);
}
