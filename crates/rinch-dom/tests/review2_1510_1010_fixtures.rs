//! Review 2 of #1510: an auto-width absolute box must not come out narrower
//! than the auto atomic inline it holds. Chrome 155: both the same width
//! (`a28`: 280.92 / 280.92). The capped-contribution change in `ifc.rs`
//! (09dce7f8) makes a fresh layout answer `a=280` around `t=281`; the
//! 1px table tolerance hides it.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = r#"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;">"#;
const T: &str = r#"<span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span>"#;

fn width(html: &str, m: &str) -> f32 {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    let id = query_selector(&doc.tree, &format!("[data-m={m}]"))[0];
    doc.tree.get(id).unwrap().layout.width
}

#[test]
fn an_auto_width_absolute_box_is_not_narrower_than_its_content() {
    let shapes = [
        format!(
            r#"{C}<div style="margin-left:42px"><div data-m="a" style="position:absolute;margin-left:5%;margin-right:10%">{T}</div></div></div>"#
        ),
        format!(
            r#"{C}<div data-m="a" style="position:absolute;left:7.3%;margin-right:17.7%">{T}</div></div>"#
        ),
        format!(
            r#"{C}<div data-m="a" style="position:absolute;left:5.3%;margin-right:3.7%">{T}</div></div>"#
        ),
    ];
    let bad: Vec<_> = shapes
        .iter()
        .map(|h| (width(h, "a"), width(h, "t")))
        .filter(|(a, t)| t - a > 0.5)
        .collect();
    assert!(
        bad.is_empty(),
        "box narrower than its content (box, content): {bad:?}"
    );
}
