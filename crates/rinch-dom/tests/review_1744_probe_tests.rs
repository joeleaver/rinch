//! Review probes for #1744 — every expectation is Chrome 155's offsetWidth
//! (google-chrome 155.0.8059.39 headless, bundled Inter as ProbeFace,
//! `font: 16px/20px ProbeFace; display: block`).
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

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

fn build(css: &str, text: &str, label: Option<&str>) -> (RinchDocument, u64) {
    let mut doc = document();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.set_attribute(
        sel,
        "style",
        &format!("font: 16px/20px ProbeFace; display: block; {css}"),
    );
    doc.append_child(body, sel);
    let opt = doc.create_element("option");
    if let Some(l) = label {
        doc.set_attribute(opt, "label", l);
    }
    doc.append_child(sel, opt);
    let t = doc.create_text(text);
    doc.append_child(opt, t);
    (doc, sel.0)
}

fn w(css: &str, text: &str, label: Option<&str>) -> f32 {
    let (mut doc, sel) = build(css, text, label);
    doc.resolve_layout(800.0, 600.0);
    doc.tree.get(sel).unwrap().layout.width
}

#[test]
fn probes() {
    let cases: &[(&str, &str, &str, Option<&str>, f32)] = &[
        ("ss-none", "", "straße", None, 70.0),
        ("ss-up", "text-transform: uppercase", "straße", None, 94.0),
        ("ss-explicit", "", "STRASSE", None, 94.0),
        ("ideo", "", "a\u{3000}\u{3000}b", None, 73.0),
        (
            "cap-hyph",
            "text-transform: capitalize",
            "foo-bar o'neil",
            None,
            131.0,
        ),
        ("nbsp-ws", "word-spacing: 10px", "a\u{a0}b", None, 56.0),
        ("nbsp", "", "a\u{a0}b", None, 46.0),
        ("ideo-ws", "word-spacing: 10px", "a\u{3000}b", None, 57.0),
        ("sigma", "text-transform: lowercase", "ΣΑΣ", None, 52.0),
        ("lig", "text-transform: uppercase", "\u{fb01}x", None, 47.0),
        ("trail-nbsp", "", "ab\u{a0}", None, 46.0),
        ("lab-nbsp", "", "q", Some("x\u{a0}y"), 45.0),
        ("lab-ideo", "", "q", Some("\u{3000}x"), 47.0),
        ("lab-tab-ls", "letter-spacing: 5px", "q", Some("x\ty"), 60.0),
        (
            "lab-up",
            "text-transform: uppercase",
            "q",
            Some("  x   y "),
            71.0,
        ),
    ];
    let mut bad = vec![];
    for (name, css, text, label, chrome) in cases {
        let got = w(css, text, *label);
        eprintln!("{name}: rinch {got} chrome {chrome}");
        if got != *chrome {
            bad.push(format!("{name}: rinch {got} chrome {chrome}"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn value_is_option_text() {
    let (mut doc, sel) = build("", "  a \t  b\u{a0} ", None);
    doc.resolve_layout(800.0, 600.0);
    let m = rinch_dom::select::resolve_select_model(&doc.tree, sel);
    assert_eq!(m.options[0].value, "a b\u{a0}");
    assert_eq!(m.options[0].label, "a b\u{a0}");
}

/// Paint at fractional scales: the label's ink stays inside the label box the
/// measure gave (left of the arrow box), whatever the spacing.
#[test]
fn ink_fits_the_measured_box_at_fractional_scales() {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    const VW: u32 = 1200;
    const VH: u32 = 200;
    let mut bad = vec![];
    for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0, 3.0] {
        for ls in [
            "letter-spacing: 0.7px",
            "letter-spacing: 3.3px",
            "letter-spacing: -1.3px",
            "word-spacing: 7.7px",
            "letter-spacing: 0.37em; text-transform: uppercase",
        ] {
            let (mut doc, sel) = build(
                &format!("position: absolute; left: 10px; top: 10px; color: rgb(0,0,0); {ls}"),
                "mwmwm iliil mmm",
                None,
            );
            doc.resolve_layout(VW as f32 / scale, VH as f32 / scale);
            let l = doc.tree.get(sel).unwrap().layout;
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
            let px = painter.pixels().as_chunks::<4>().0.to_vec();
            let s = |v: f32| (v * scale).round() as u32;
            let (top, bottom) = (s(10.0 + 1.0), s(10.0 + l.height - 1.0));
            let arrow_l = s(10.0 + l.width - 1.0 - 16.0);
            let right = (0..arrow_l)
                .rev()
                .find(|&x| {
                    (top..bottom).any(|y| {
                        let p = px[(y * VW + x) as usize];
                        p[3] > 128 && p[0] < 128
                    })
                })
                .unwrap_or(0);
            // label box right edge: border 1 + inset 4 + label width = width - 1 - 16 - 1? ; compare vs (width - 1 - 16) i.e. arrow box start, minus the 4px inset as slack of ±
            let label_end = 10.0 + l.width - 1.0 - 16.0; // label box ends where the arrow box starts
            eprintln!(
                "scale {scale} {ls}: width {} ink right {} (css {:.2}) label_end {label_end}",
                l.width,
                right,
                right as f32 / scale
            );
            if right as f32 / scale > label_end {
                bad.push(format!(
                    "scale {scale} {ls}: ink {} past {}",
                    right as f32 / scale,
                    label_end
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Restyles that reach the label re-size the select: the select's own
/// word-spacing, and an inherited letter-spacing / text-transform from an
/// ancestor.
#[test]
fn restyles_resize() {
    let (mut doc, sel) = build("", "a b c", None);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(sel).unwrap().layout.width, 59.0);
    doc.set_style(rinch_core::dom::NodeId(sel), "word-spacing", "10px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(sel).unwrap().layout.width,
        79.0,
        "own word-spacing"
    );

    let (mut doc, sel) = build("", "abc", None);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(sel).unwrap().layout.width, 50.0);
    let body = doc.body();
    doc.set_style(body, "letter-spacing", "5px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(sel).unwrap().layout.width,
        65.0,
        "inherited letter-spacing"
    );
    doc.set_style(body, "text-transform", "uppercase");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(sel).unwrap().layout.width,
        71.0,
        "inherited transform"
    );
}
