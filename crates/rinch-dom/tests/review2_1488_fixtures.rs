//! Review 2 of PR #1488 (#1476) — fixtures for the round-2 findings.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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

fn one(doc: &RinchDocument, m: &str) -> NodeId {
    NodeId(query_selector(&doc.tree, &format!("[data-m={m}]"))[0])
}

const LONG: &str = "Wavymillilitersunbroken WWW mmm Wavy";
const SHORT: &str = "a b c d e f g h i j k l m n o p q r s t u v w";

/// Finding 1 — a text edit that changes a box's **min-content width** and
/// leaves its **size** where it was. The box is re-measured, comes out the
/// size it had, and nothing tells the inline formatting context around it
/// that what the box contributes to a narrower line has changed: the flex
/// item keeps the automatic minimum size of the old text.
///
/// Here the box is `max-width: 170px` and both texts are two lines at 170:
/// the first has a 180px word (the item cannot shrink below it), the second
/// has none (the item shrinks to share the row).
#[test]
fn r2f1_an_edit_that_moves_only_the_min_content_width_reaches_the_container() {
    let html = |text: &str, box_style: &str| {
        format!(
            r#"<div data-m="c" style="width:400px;display:flex"><div data-m="f"><span data-m="t" style="display:inline-block;{box_style}">{text}</span></div><div data-m="g" style="min-width:0"><span data-m="u" style="display:inline-block;white-space:nowrap">{SHORT}</span></div></div>"#
        )
    };
    let mut bad = vec![];
    for (name, box_style, from, to) in [
        ("long_to_short", "max-width:170px", LONG, SHORT),
        ("short_to_long", "max-width:170px", SHORT, LONG),
        ("fixed_height", "max-width:170px;height:50px", LONG, SHORT),
    ] {
        let mut doc = mount(&html(from, box_style));
        doc.resolve_layout(800.0, 600.0);
        let t = one(&doc, "t");
        let text = NodeId(doc.tree.get(t.0).unwrap().children[0]);
        doc.set_text_content(text, to);
        doc.resolve_layout(800.0, 601.0);
        let inc = dump(&doc);
        doc.resolve_layout(800.0, 602.0);
        let again = dump(&doc);
        let mut fresh = mount(&html(to, box_style));
        fresh.resolve_layout(800.0, 600.0);
        let fresh = dump(&fresh);
        if inc != fresh || again != fresh {
            bad.push(format!(
                "{name}: incremental[{inc}] again[{again}] fresh[{fresh}]"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Finding 1 as the random history found it (no-percentage seed 218).
#[test]
fn r2f1b_seed_218() {
    let html = |t0: &str, t1: &str, extra: bool| {
        format!(
            r#"<div data-m="c" style="width:400px;display:flex"><div data-m="f" style="padding:0 5px">ab <span data-m="t" style="display:inline-block;max-width:170px">{t0}</span></div><div data-m="f" style="min-width:0">ab <span data-m="t" style="display:inline-block;margin:0 13px">{t1}{}</span></div><div data-m="f" style="min-width:0"><span data-m="t" style="display:inline-block;white-space:nowrap">{SHORT}</span></div></div>"#,
            if extra {
                r#"<b data-x="1">Wavymillilitersunbrokenlonger zz</b>"#
            } else {
                ""
            }
        )
    };
    let boxes = |doc: &RinchDocument| -> Vec<usize> { query_selector(&doc.tree, "[data-m=t]") };
    let mut bad = vec![];
    for variant in ["full", "no_extra_step", "no_row1_edit"] {
        let mut doc = mount(&html(LONG, "Wavy", variant == "no_extra_step"));
        doc.resolve_layout(800.0, 600.0);
        let b = boxes(&doc);
        if variant != "no_extra_step" {
            let x = doc.create_element("b");
            doc.set_attribute(x, "data-x", "1");
            let tx = doc.create_text("Wavymillilitersunbrokenlonger zz");
            doc.append_child(x, tx);
            doc.append_child(NodeId(b[1]), x);
            doc.resolve_layout(800.0, 601.0);
        }
        if variant != "no_row1_edit" {
            let text1 = NodeId(doc.tree.get(b[1]).unwrap().children[0]);
            doc.set_text_content(text1, "Wavy");
            doc.resolve_layout(800.0, 603.0);
        }
        let text0 = NodeId(doc.tree.get(b[0]).unwrap().children[0]);
        doc.set_text_content(text0, SHORT);
        doc.resolve_layout(800.0, 604.0);
        let inc = dump(&doc);
        doc.resolve_layout(800.0, 605.0);
        let again = dump(&doc);
        let mut fresh = mount(&html(SHORT, "Wavy", true));
        fresh.resolve_layout(800.0, 600.0);
        let fresh = dump(&fresh);
        if inc != fresh || again != fresh {
            bad.push(format!(
                "{variant}: incremental[{inc}] again[{again}] fresh[{fresh}]"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
