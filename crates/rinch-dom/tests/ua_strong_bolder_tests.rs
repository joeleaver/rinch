//! `<strong>` and `<b>` are `font-weight: bolder`, as in the HTML Standard and
//! Chrome: one step heavier than the parent, by CSS Fonts' relative-weight
//! table. rinch's UA sheet said `bold`, which is the same answer only under a
//! parent of 350–549 (a normal one). Under a bold heading a `<strong>` is 900
//! in Chrome and was 700 here, so wherever the family has a face heavier than
//! its bold the heading drew narrower and wrapped later than Chrome's.
//!
//! Every expected number is Chrome 156's for the same markup (Playwright,
//! `getComputedStyle(e).fontWeight` and `getBoundingClientRect`), with the
//! bundled Inter loaded by `@font-face` for the widths.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../rinch/assets/fonts/Inter-Bold.ttf");

fn weight(doc: &RinchDocument, id: &str) -> f32 {
    let n = doc.query_selector(&format!("#{id}")).expect(id);
    doc.tree.get(n.0).unwrap().computed_style.font_weight
}

#[test]
fn strong_and_b_are_one_step_heavier_than_their_parent() {
    // (parent weight, Chrome 156's computed weight of the <strong> and <b>)
    const TABLE: &[(u32, f32)] = &[
        (100, 400.0),
        (200, 400.0),
        (300, 400.0),
        (350, 700.0),
        (400, 700.0),
        (500, 700.0),
        (549, 700.0),
        (550, 900.0),
        (600, 900.0),
        (700, 900.0),
        (800, 900.0),
        (900, 900.0),
    ];
    let mut html = String::new();
    for (w, _) in TABLE {
        html += &format!(
            "<div style='font-weight:{w}'><strong id=s{w}>x</strong><b id=b{w}>x</b></div>"
        );
    }
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(800.0, 600.0);
    for &(w, want) in TABLE {
        assert_eq!(weight(&doc, &format!("s{w}")), want, "<strong> under {w}");
        assert_eq!(weight(&doc, &format!("b{w}")), want, "<b> under {w}");
    }
}

#[test]
fn nested_bold_elements_and_a_strong_in_a_heading_compound() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_inner_html(
        body,
        "<div><b><b id=bb>x</b></b></div>\
         <h1><strong id=h1s>x</strong></h1>\
         <table><tr><th><b id=thb>x</b></th></tr></table>\
         <div style='font-weight:700'><b style='font-weight:normal'><strong id=reset>x</strong></b></div>",
    );
    doc.resolve_layout(800.0, 600.0);
    // Chrome 156: 900, 900, 900, 700.
    assert_eq!(weight(&doc, "bb"), 900.0, "<b><b>: 700, then 900");
    assert_eq!(weight(&doc, "h1s"), 900.0, "<strong> in a bold <h1>");
    assert_eq!(weight(&doc, "thb"), 900.0, "<b> in a bold <th>");
    assert_eq!(
        weight(&doc, "reset"),
        700.0,
        "an author `normal` between resets the step"
    );
}

/// The weight reaches the text: under a 300 parent a `<strong>` is set at
/// 400, so it takes the family's regular face, as the parent's own text
/// does, and not its bold. The family is the bundled Inter, its Regular (400)
/// and Bold (700) files. Chrome 156 widths at 16px: 242.41 for
/// the 300 span, the `<strong>` and the `<b>`; 249.19 for the 700 span (rinch
/// rounds box edges to whole pixels: 242 and 249).
#[test]
fn a_strong_under_a_light_parent_takes_the_regular_face() {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    for face in [INTER_REGULAR, INTER_BOLD] {
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(face)),
            Some(FontInfoOverride {
                family_name: Some("G"),
                ..Default::default()
            }),
        );
    }
    doc.load_css(
        "body { margin: 0; font: 16px/20px G; } .r { display: flex; } .r > * { flex: none; }",
    );
    let body = doc.body();
    const T: &str = "The quick brown fox jumps over";
    doc.set_inner_html(
        body,
        &format!(
            "<div class=r style='font-weight:300'><span id=g300>{T}</span></div>\
             <div class=r style='font-weight:300'><strong id=s300>{T}</strong></div>\
             <div class=r style='font-weight:300'><b id=b300>{T}</b></div>\
             <div class=r style='font-weight:700'><span id=g700>{T}</span></div>"
        ),
    );
    doc.resolve_layout(800.0, 600.0);
    let w = |id: &str| {
        let n: NodeId = doc.query_selector(&format!("#{id}")).expect(id);
        doc.tree.get(n.0).unwrap().layout.width
    };
    let (light, strong, b, bold) = (w("g300"), w("s300"), w("b300"), w("g700"));
    assert_eq!(
        (light, bold),
        (242.0, 249.0),
        "the two faces, as Chrome sets them"
    );
    assert_eq!(
        strong, light,
        "<strong> under 300 is set in the regular face"
    );
    assert_eq!(b, light, "<b> under 300 is set in the regular face");
}
