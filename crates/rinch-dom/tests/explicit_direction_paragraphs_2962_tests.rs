//! #2962: a block with an explicit direction (`dir=rtl`/`ltr`, any
//! `unicode-bidi` but `plaintext`) is split into bidi paragraphs at each
//! forced break (a `<br>`, a preserved newline), every paragraph taking the
//! block's direction (UAX #9 P1, X8), as Chrome does. Before, the block's
//! text was one paragraph, so a neutral (here an inline-block) that starts a
//! paragraph took its direction from the strong text before the break.
//! The bidi controls of an element open across the break (its
//! `unicode-bidi`, a block's override) carry into the next paragraph, as
//! Blink reopens them after a forced break.
//!
//! Each case is measured in Chrome 158 (headless, `--dump-dom`) on this exact
//! markup: the bundled Inter at a declared `font: 16px/20px`, a `400px` host
//! at the origin, `body { margin: 0 }`, `pre { margin: 0; font: inherit }`
//! and `span.t { display: inline-block }`. A case's boxes are the host's
//! `.t` elements, `[x, y, w, h]` relative to the host. No expected number
//! depends on a Hebrew glyph's width (Inter has none).

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::{get_node_summary, query_selector};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

type Case = (&'static str, String, &'static [[f32; 4]]);

fn cases() -> Vec<Case> {
    let a = "\u{5d0}";
    let t = r#"<span class=t>abc</span>"#;
    vec![
        // The box starts an rtl paragraph after a Latin one: at the right.
        (
            "div_rtl_br",
            format!("<div dir=rtl>abc<br>{t}abc</div>"),
            &[[372.08, 20.0, 27.92, 20.0]],
        ),
        (
            "pre_rtl_newline",
            format!("<pre dir=rtl>abc\n{t}abc</pre>"),
            &[[372.08, 20.0, 27.92, 20.0]],
        ),
        (
            "div_rtl_br_hebrew_after",
            format!("<div dir=rtl>abc<br>{t} {a}</div>"),
            &[[372.08, 20.0, 27.92, 20.0]],
        ),
        // The mirror: an ltr paragraph after a Hebrew one, its box at the left.
        (
            "div_ltr_br",
            format!("<div dir=ltr>{a}<br>{t}{a}</div>"),
            &[[0.0, 20.0, 27.92, 20.0]],
        ),
        (
            "pre_ltr_newline",
            format!("<pre dir=ltr>{a}\n{t}{a}</pre>"),
            &[[0.0, 20.0, 27.92, 20.0]],
        ),
        // A block's override carries across the break.
        (
            "override_across_br",
            format!(r#"<div dir=rtl style="unicode-bidi:bidi-override">ab<br>{t}xy</div>"#),
            &[[372.08, 20.0, 27.92, 20.0]],
        ),
        // So do an inline element's embedding and isolate: the second line's
        // `abc d` runs left to right inside the rtl line, the box first.
        (
            "embed_across_br",
            format!(
                r#"<div dir=rtl><span style="unicode-bidi:embed;direction:ltr">abc<br>{t} d</span></div>"#
            ),
            &[[357.78, 20.0, 27.92, 20.0]],
        ),
        (
            "isolate_across_br",
            format!("<div dir=rtl><span dir=ltr>abc<br>{t} d</span></div>"),
            &[[357.78, 20.0, 27.92, 20.0]],
        ),
    ]
}
fn document(markup: &str) -> RinchDocument {
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
    doc.load_css(
        "body { margin: 0; font: 16px/20px ProbeFace; } span.t { display: inline-block } \
         pre { margin: 0; font: inherit } #host > div, #host > pre { width: 400px }",
    );
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "id", "host");
    doc.set_attribute(host, "style", "width: 400px");
    doc.append_child(body, host);
    doc.set_inner_html(host, markup);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn boxes(doc: &RinchDocument) -> Vec<[f32; 4]> {
    query_selector(&doc.tree, ".t")
        .iter()
        .map(|&n| {
            let abs = &get_node_summary(&doc.tree, n).unwrap()["absolute"];
            let at = |k: &str| abs[k].as_f64().unwrap() as f32;
            [at("x"), at("y"), at("width"), at("height")]
        })
        .collect()
}

/// Every case against Chrome 158, each edge to within half a pixel.
#[test]
fn explicit_direction_paragraphs_split_at_forced_breaks_as_in_chrome_158() {
    let mut wrong = Vec::new();
    let all = cases();
    for (name, markup, chrome) in &all {
        let doc = document(markup);
        let got = boxes(&doc);
        let ok = got.len() == chrome.len()
            && got
                .iter()
                .zip(chrome.iter())
                .all(|(g, c)| (0..4).all(|i| (g[i] - c[i]).abs() <= 0.5));
        if !ok {
            wrong.push(format!("{name}: rinch {got:?}, Chrome {chrome:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ from Chrome 158:\n{}",
        wrong.len(),
        all.len(),
        wrong.join("\n")
    );
}
