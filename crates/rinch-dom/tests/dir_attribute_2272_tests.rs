//! #2272: the HTML `dir` attribute sets the element's direction, as Chrome's
//! UA sheet and its `dir=auto` resolution do.
//!
//! `dir=ltr`/`dir=rtl` (any case) set `direction`; `dir=auto` takes the
//! direction of the first strong character of the element's text, skipping
//! descendants that set their own `dir` and `<bdi>`, `<script>`, `<style>`
//! and `<textarea>` (HTML's "auto directionality"), and is `ltr` when there
//! is none. The direction then reaches everything that reads it: a list's
//! logical padding, block placement under auto margins, `text-align:
//! start`/`end`, and the order of an RTL paragraph's inline boxes.
//!
//! Each case is measured in Chrome 158 (headless, `--dump-dom`) on this exact
//! markup: the bundled Inter at a declared `font: 16px/20px`, a `400px` host
//! at the origin, `body { margin: 0 }`, and `span.t, bdi.t { display:
//! inline-block }` so that a measured word has a box of its own. A case's
//! boxes are the host's `li` and `.t` elements in document order, `[x, y, w,
//! h]` relative to the host. No expected number depends on a Hebrew glyph's
//! width (Inter has none, so that would pin the host's fallback font).
//!
//! rinch also gives each `div` and `p` child of the host `width: 400px`, the
//! width it has anyway: an auto-width block aligns a line against its width
//! plus a pixel of break slack (#1869), a known difference of its own that
//! `direction: rtl` shows as readily as `dir`.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::{get_node_summary, query_selector};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const HEB: &str = "\u{5d0}\u{5d1}\u{5d2}";

type Case = (&'static str, String, &'static [[f32; 4]]);

fn cases() -> Vec<Case> {
    let a = "\u{5d0}";
    let bar100 = r#"<div class=t style="width:100px;height:10px"></div>"#;
    vec![
        // The issue's list: Chrome draws it as `direction: rtl`.
        (
            "ul_dir_rtl",
            "<ul dir=rtl style=margin:0><li><div>block child</div></li></ul>".into(),
            &[[0.0, 0.0, 360.0, 20.0]],
        ),
        (
            "ul_dir_auto_hebrew",
            format!("<ul dir=auto style=margin:0><li>{HEB}</li></ul>"),
            &[[0.0, 0.0, 360.0, 20.0]],
        ),
        (
            "ul_dir_auto_latin_first",
            format!("<ul dir=auto style=margin:0><li>abc {a}</li></ul>"),
            &[[40.0, 0.0, 360.0, 20.0]],
        ),
        // A fixed-width block under an RTL parent sits at its right.
        (
            "block_dir_auto",
            format!("<div dir=auto><p style=margin:0>{a}\u{5d1}</p>{bar100}</div>"),
            &[[300.0, 20.0, 100.0, 10.0]],
        ),
        (
            "block_dir_rtl",
            format!("<div dir=rtl>{bar100}</div>"),
            &[[300.0, 0.0, 100.0, 10.0]],
        ),
        // `text-align: start` (the initial value) and `end`.
        (
            "start_dir_rtl",
            "<div dir=rtl><span class=t>abc</span></div>".into(),
            &[[372.08, 0.0, 27.92, 20.0]],
        ),
        (
            "start_dir_upper_case_rtl",
            "<div dir=RTL><span class=t>abc</span></div>".into(),
            &[[372.08, 0.0, 27.92, 20.0]],
        ),
        (
            "end_dir_rtl",
            r#"<p dir=rtl style="margin:0;text-align:end"><span class=t>abc</span></p>"#.into(),
            &[[0.0, 0.0, 27.92, 20.0]],
        ),
        (
            "dir_ltr_inside_rtl",
            "<div dir=rtl><p dir=ltr style=margin:0><span class=t>abc</span></p></div>".into(),
            &[[0.0, 0.0, 27.92, 20.0]],
        ),
        (
            "end_dir_auto_hebrew_first",
            format!(
                r#"<p dir=auto style="margin:0;text-align:end">{a} <span class=t>abc</span></p>"#
            ),
            &[[0.0, 0.0, 27.92, 20.0]],
        ),
        (
            "left_dir_auto_hebrew_first",
            format!(
                r#"<p dir=auto style="margin:0;text-align:left">{a} <span class=t>abc</span></p>"#
            ),
            &[[0.0, 0.0, 27.92, 20.0]],
        ),
        // Inline order: two neutral boxes in an RTL paragraph run right to left.
        (
            "inline_order_dir_rtl",
            concat!(
                "<div dir=rtl>",
                r#"<span class=t style="display:inline-block;width:20px;height:10px"></span>"#,
                r#"<span class=t style="display:inline-block;width:30px;height:10px"></span>"#,
                "</div>"
            )
            .into(),
            &[[380.0, 6.0, 20.0, 10.0], [350.0, 6.0, 30.0, 10.0]],
        ),
        (
            "inline_order_dir_auto",
            format!(
                r#"<div dir=auto><span class=t style="display:inline-block;width:20px;height:10px"></span>{a}</div>"#
            ),
            &[[380.0, 6.0, 20.0, 10.0]],
        ),
        // Not a direction: no effect.
        (
            "dir_invalid",
            "<div dir=foo><span class=t>abc</span></div>".into(),
            &[[0.0, 0.0, 27.92, 20.0]],
        ),
        (
            "bdi_latin",
            "<bdi class=t>abc</bdi>".into(),
            &[[0.0, 0.0, 27.92, 20.0]],
        ),
        // No strong character: `ltr`, even inside an RTL parent.
        (
            "dir_auto_digits_only",
            "<div dir=auto><span class=t>123</span></div>".into(),
            &[[0.0, 0.0, 26.16, 20.0]],
        ),
        (
            "dir_auto_digits_in_rtl",
            "<div dir=rtl><div dir=auto><span class=t>123</span></div></div>".into(),
            &[[0.0, 0.0, 26.16, 20.0]],
        ),
        // The first strong character is found through an inline element...
        (
            "dir_auto_through_b",
            format!("<div dir=auto><b>{a}</b>{bar100}</div>"),
            &[[300.0, 20.0, 100.0, 10.0]],
        ),
        // ...but not in a `<bdi>`, nor in a descendant with its own `dir`.
        (
            "dir_auto_skips_bdi",
            format!("<div dir=auto><bdi>{a}</bdi>x{bar100}</div>"),
            &[[0.0, 20.0, 100.0, 10.0]],
        ),
        (
            "dir_auto_skips_dir_descendant",
            format!("<div dir=auto><span dir=ltr>a</span>{a}{bar100}</div>"),
            &[[300.0, 20.0, 100.0, 10.0]],
        ),
        // One direction for the whole element, every line of it: `isolate`,
        // not `plaintext`, whose second paragraph would be `ltr`. (Chrome makes a
        // `<pre>` or `<textarea>` with `dir=auto` `plaintext`, each paragraph
        // its own; rinch's `plaintext` block takes one direction for all of
        // them, a known difference of `plaintext` itself.)
        (
            "dir_auto_after_a_br",
            format!("<div dir=auto>{a}<br><span class=t>abc</span></div>"),
            &[[372.08, 20.0, 27.92, 20.0]],
        ),
        // A hint: an author `direction` beats it.
        (
            "author_direction_beats_dir_auto",
            format!(r#"<div dir=auto style="direction:ltr">{a}<br><span class=t>abc</span></div>"#),
            &[[0.0, 20.0, 27.92, 20.0]],
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
        "body { margin: 0; font: 16px/20px ProbeFace; } span.t, bdi.t { display: inline-block } \
         #host > div, #host > p { width: 400px }",
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
    // No case has both an `li` and a `.t`, so each list is in document order.
    let mut ids: Vec<_> = query_selector(&doc.tree, "li");
    ids.extend(query_selector(&doc.tree, ".t"));
    ids.iter()
        .map(|&n| {
            let abs = &get_node_summary(&doc.tree, n).unwrap()["absolute"];
            let at = |k: &str| abs[k].as_f64().unwrap() as f32;
            [at("x"), at("y"), at("width"), at("height")]
        })
        .collect()
}

/// Every case against Chrome 158, each edge to within half a pixel (Inter's
/// advances shape a few hundredths apart in the two engines).
#[test]
fn the_dir_attribute_lays_out_as_chrome_158_does() {
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

fn li_x(doc: &RinchDocument) -> f32 {
    let li = query_selector(&doc.tree, "li")[0];
    get_node_summary(&doc.tree, li).unwrap()["absolute"]["x"]
        .as_f64()
        .unwrap() as f32
}

fn by_id(doc: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    rinch_core::dom::NodeId(query_selector(&doc.tree, &format!("#{id}"))[0])
}

fn rtl(doc: &RinchDocument, id: &str) -> bool {
    doc.tree
        .get(by_id(doc, id).0)
        .unwrap()
        .computed_style
        .direction_rtl
}

const LIST: &str = "<ul style=margin:0><li>x</li></ul>";

/// The direction follows the text as it changes, at the same viewport: a
/// write of the text, a child inserted, a `dir` set on a descendant, which
/// hides its text, and a child removed. Chrome 158, the first: the list's item moves from
/// x 40 to 0 when `abc` becomes `א`.
#[test]
fn dir_auto_follows_its_text_as_it_changes() {
    let mut doc = document(&format!(
        "<div id=d dir=auto><span id=s>abc</span>{LIST}</div>"
    ));
    assert_eq!(li_x(&doc), 40.0, "ltr from `abc`");
    let s = by_id(&doc, "s");
    doc.set_text_content(s, "\u{5d0}");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(li_x(&doc), 0.0, "rtl once the text is Hebrew");
    assert!(rtl(&doc, "d"));

    // A child inserted where there was no strong letter at all.
    let mut doc = document(&format!(
        "<div id=d dir=auto><span id=s></span>{LIST}</div>"
    ));
    assert_eq!(li_x(&doc), 40.0, "no strong letter: ltr");
    let s = by_id(&doc, "s");
    let b = doc.create_element("b");
    let t = doc.create_text("\u{5d0}");
    doc.append_child(b, t);
    doc.append_child(s, b);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(li_x(&doc), 0.0, "rtl from the inserted Hebrew");

    // A `dir` on the descendant that holds the Hebrew takes it out.
    let mut doc = document(&format!(
        "<div id=d dir=auto><span id=s>\u{5d0}</span>a{LIST}</div>"
    ));
    assert_eq!(li_x(&doc), 0.0, "rtl from the span's Hebrew");
    let s = by_id(&doc, "s");
    doc.set_attribute(s, "dir", "ltr");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        li_x(&doc),
        40.0,
        "the span's text no longer counts: `a`, ltr"
    );
    doc.set_attribute(s, "dir", "nonsense");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(li_x(&doc), 0.0, "a `dir` in no state hides nothing");

    // Removing the child that held the Hebrew: the first strong letter is
    // the `a` after it.
    let d = by_id(&doc, "d");
    doc.remove_child(d, s);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(li_x(&doc), 40.0, "ltr once the Hebrew is removed");
}

/// A text field's `dir=auto` reads its value (Chrome 158: `<input dir=auto
/// value=א>` and `<textarea dir=auto>א</textarea>` are `rtl`, an input of
/// `abc` is `ltr`), and follows it as the value is written.
#[test]
fn a_fields_dir_auto_follows_its_value() {
    let mut doc = document(
        "<input id=i dir=auto value=\"\u{5d0}\"><textarea id=ta dir=auto>\u{5d0}</textarea>\
         <input id=j dir=auto value=abc>",
    );
    assert!(rtl(&doc, "i"));
    assert!(rtl(&doc, "ta"));
    assert!(!rtl(&doc, "j"));
    let i = by_id(&doc, "i");
    doc.set_attribute(i, "value", "abc");
    doc.resolve_layout(800.0, 600.0);
    assert!(!rtl(&doc, "i"), "ltr once the value is Latin");
    let j = by_id(&doc, "j");
    doc.set_attribute(j, "value", "\u{5d0}");
    doc.resolve_layout(800.0, 600.0);
    assert!(rtl(&doc, "j"), "rtl once the value is Hebrew");
}
