//! Carets beside inline boxes, which hold no bytes of the text: at a line
//! boundary next to one (#1903, #1711), and on the logical side of one in
//! right-to-left text (#1835).

use rinch_core::dom::{CaretAffinity, DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc(style: &str, inner: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font:16px/25px ProbeFace;{style}"));
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

fn caret(d: &RinchDocument, c: NodeId, byte: usize, a: CaretAffinity) -> (f32, f32) {
    d.query_caret_position_with_affinity(c.0, byte, a).unwrap()
}

/// `aaaa ` / `bbbb ` + the span's closing edge box / `cc` at 60px: the
/// downstream caret at the first character of line 2 is at that line's
/// start, not at line 1's end after the box (#1903); the upstream one at the
/// same byte stays at line 1's end. The selection of line 2 starts there too.
#[test]
fn the_caret_after_a_line_ending_in_an_edge_box_starts_the_next_line_issue_1903() {
    let (d, c) = doc(
        "width:60px",
        r#"aaaa <span style="padding:0 2px">bbbb </span>cc"#,
    );
    let lines: Vec<_> = {
        let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
        il.layout.lines().map(|l| l.text_range()).collect()
    };
    assert_eq!(lines, vec![0..5, 5..10, 10..12], "the lines of the fixture");
    assert_eq!(caret(&d, c, 10, CaretAffinity::Downstream), (0.0, 50.0));
    let (ux, uy) = caret(&d, c, 10, CaretAffinity::Upstream);
    assert_eq!(uy, 25.0, "upstream: line 1");
    assert!(ux > 40.0, "upstream: line 1's end, {ux}");
    // The caret before `c` is left of the one after it, on one line.
    let (x11, y11) = caret(&d, c, 11, CaretAffinity::Downstream);
    assert_eq!(y11, 50.0);
    assert!(x11 > 8.0, "{x11}");
    let rects = d.query_selection_rects(c.0, 10, 12);
    assert_eq!(rects.len(), 1, "{rects:?}");
    let (x, y, w, _) = rects[0];
    assert_eq!((x, y), (0.0, 50.0), "{rects:?}");
    assert!((w - 18.28).abs() < 0.1, "{rects:?}");
}

/// The upstream caret at the end of the text before an image alone on its
/// line is at that text's end, not at the start of the text after the image
/// (#1711): `alpha ` / `bravo` / image / `charlie` at 80px.
#[test]
fn the_upstream_caret_before_an_image_alone_on_its_line_ends_the_line_above_issue_1711() {
    let (d, c) = doc(
        "width:80px",
        r#"alpha bravo<img src="x.png" style="width:40px;height:16px">charlie"#,
    );
    let (ex, ey) = caret(&d, c, 10, CaretAffinity::Downstream);
    let (ux, uy) = caret(&d, c, 11, CaretAffinity::Upstream);
    assert_eq!(uy, ey, "on `bravo`'s line");
    assert!(ux > ex, "after the `o`: {ux} vs {ex}");
    let (dx, dy) = caret(&d, c, 11, CaretAffinity::Downstream);
    assert_eq!(dx, 0.0, "downstream: `charlie`'s start");
    assert!(dy > uy + 25.0, "below the image's line: {dy}");
}

/// A padded link in Hebrew text: its edge boxes sit on the logical side of
/// its text, so the caret past its end edge (where a character typed after
/// the link goes) is at the box's left edge, which is where the byte caret
/// after the link is; the caret inside the link at its end is at the box's
/// right edge (#1835). The start edge is the mirror. Before the fix every
/// trailing side was the box's right edge, so the caret after the link was
/// 5px into it.
#[test]
fn edge_carets_of_a_padded_link_in_right_to_left_text_issue_1835() {
    let (mut d, c) = doc("width:400px", "");
    d.set_inner_html(
        c,
        r#"<p>שלום <a href="x" style="padding:0 5px">אבג</a> עולם</p>"#,
    );
    d.resolve_layout(800.0, 600.0);
    let p = d.get_children(c)[0];
    let a = d.query_selector_all("a")[0];
    let edge = |end: bool, after: bool| d.query_inline_edge_caret(p.0, a.0, end, after).unwrap().0;
    // "שלום " is 9 bytes and "אבג" 6: the link is bytes 9..15.
    let (after_link, _) = caret(&d, p, 15, CaretAffinity::Downstream);
    assert!(
        (edge(true, true) - after_link).abs() < 0.01,
        "{} vs {after_link}",
        edge(true, true)
    );
    assert!(
        (edge(true, false) - edge(true, true) - 5.0).abs() < 0.01,
        "inside: 5px to its right"
    );
    // The start edge sits right of the link's text: the byte caret at the
    // link's first character is inside it, at the box's left edge; the caret
    // before the link (where a character typed before it goes, joining
    // "שלום ") is past the box, 5px to the right.
    let (first, _) = caret(&d, p, 9, CaretAffinity::Downstream);
    assert!(
        (edge(false, true) - first).abs() < 0.01,
        "{} vs {first}",
        edge(false, true)
    );
    assert!(
        (edge(false, false) - first - 5.0).abs() < 0.01,
        "outside: 5px to its right, {}",
        edge(false, false)
    );
}
