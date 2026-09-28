//! A closed `<select>`'s sizing and paint (#1098), from the review of PR #1124:
//! each fixture pins a mutant `select_intrinsic_size_tests` let survive — an
//! author `min-height` dropped, the label clip ignoring the arrow box, the
//! label centred in the border box rather than the content box, and an arrow
//! box that is not scaled with the device scale.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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

/// Chrome 153: `min-height: 3em` on a 16px select gives 50 x 48, `30px` gives
/// 50 x 30 — an author min-height larger than the intrinsic one wins.
/// Kills: `intrinsic.max(author_min)` -> `intrinsic` on the height axis.
#[test]
fn an_author_min_height_above_the_intrinsic_one_wins() {
    for (css, chrome_h) in [("min-height: 3em", 48.0), ("min-height: 30px", 30.0)] {
        let mut doc = document();
        let body = doc.body();
        let sel = doc.create_element("select");
        doc.set_attribute(sel, "style", &format!("font: 16px/20px ProbeFace; {css}"));
        doc.append_child(body, sel);
        let o = doc.create_element("option");
        doc.append_child(sel, o);
        let t = doc.create_text("abc");
        doc.append_child(o, t);
        doc.resolve_layout(800.0, 600.0);
        let l = doc.tree.get(sel.0).unwrap().layout;
        assert_eq!((l.width, l.height), (50.0, chrome_h), "{css}");
    }
}

/// Paint `select(style) > option > "label"` at `scale`, return (pixels, w, h,
/// select layout).
fn paint(
    style: &str,
    label: &str,
    scale: f32,
) -> (Vec<[u8; 4]>, u32, u32, rinch_dom::node::LayoutResult) {
    const VW: u32 = 300;
    const VH: u32 = 160;
    let mut doc = document();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.set_attribute(sel, "style", style);
    doc.append_child(body, sel);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text(label);
    doc.append_child(o, t);
    doc.resolve_layout(VW as f32 / scale, VH as f32 / scale);
    let l = doc.tree.get(sel.0).unwrap().layout;
    let mut painter = TinySkiaPainter::new(VW, VH);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
        (VW as f32, VH as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    (painter.pixels().as_chunks::<4>().0.to_vec(), VW, VH, l)
}

fn black(px: &[[u8; 4]], vw: u32, x: u32, y: u32) -> bool {
    let p = px[(y * vw + x) as usize];
    p[3] > 200 && p[0] < 60 && p[1] < 60 && p[2] < 60
}

/// An explicit width narrower than the label: the label is clipped at the
/// arrow box, so in the arrow box's columns the only black ink is the chevron,
/// which spans 7 px vertically around the content box's middle.
/// Kills: the clip's right edge at the content box's edge (arrow box ignored),
/// and (at scale 2) an arrow box that is not scaled.
#[test]
fn a_clipped_label_stops_at_the_arrow_box() {
    for scale in [1.0f32, 2.0] {
        let (px, vw, vh, l) = paint(
            "position: absolute; left: 10px; top: 10px; width: 80px; \
             font: 40px ProbeFace; color: rgb(0, 0, 0)",
            "WWWWWWWWWW",
            scale,
        );
        assert_eq!(l.width, 80.0);
        let s = |v: f32| (v * scale).round() as u32;
        // Content box: border 1.
        let (cl, cr) = (s(10.0 + 1.0), s(10.0 + 80.0 - 1.0));
        let (ct, cb) = (s(10.0 + 1.0), s(10.0 + l.height - 1.0));
        let cy = (ct + cb) / 2;
        let arrow_l = cr - s(16.0);
        // Positive control: the label is painted left of the arrow box.
        assert!(
            (cl..arrow_l).any(|x| (ct..cb).any(|y| black(&px, vw, x, y))),
            "scale {scale}: no label ink at all"
        );
        let band = s(6.0);
        let stray: Vec<(u32, u32)> = (arrow_l + 1..cr)
            .flat_map(|x| (ct..cb.min(vh)).map(move |y| (x, y)))
            .filter(|&(x, y)| (y + band < cy || y > cy + band) && black(&px, vw, x, y))
            .collect();
        assert!(
            stray.is_empty(),
            "scale {scale}: label ink inside the arrow box, outside the chevron's band: {:?}",
            &stray[..stray.len().min(8)]
        );
        // The chevron itself: centred in the arrow box, so its ink stays in
        // the arrow box's middle 8 px (+1 for anti-aliasing) either side.
        let mid = cr - s(8.0);
        let chevron: Vec<u32> = (arrow_l..cr)
            .filter(|&x| (cy - band..=cy + band).any(|y| black(&px, vw, x, y)))
            .collect();
        assert!(!chevron.is_empty(), "scale {scale}: no chevron");
        assert!(
            chevron
                .iter()
                .all(|&x| x + s(5.0) >= mid && x <= mid + s(5.0)),
            "scale {scale}: chevron columns {chevron:?} not centred on {mid}"
        );
    }
}

/// Asymmetric author padding: the label is centred in the **content** box, so
/// with `padding-top: 30px` its ink starts below the border + padding.
/// Kills: `text_y` measured from the border box top instead of the content box.
#[test]
fn the_label_is_centred_in_the_content_box_under_asymmetric_padding() {
    let (px, vw, _, l) = paint(
        "position: absolute; left: 10px; top: 10px; padding-top: 30px; \
         font: 40px ProbeFace; color: rgb(0, 0, 0)",
        "IIII",
        1.0,
    );
    let content_top = 10 + 1 + 30;
    let first_ink_row = (10..(10 + l.height as u32))
        .find(|&y| (12..60).any(|x| black(&px, vw, x, y)))
        .expect("no label ink: no positive control");
    assert!(
        first_ink_row >= content_top,
        "label ink from row {first_ink_row}, above the content box top {content_top}"
    );
}
