//! `display: -webkit-box`, `-webkit-box-orient` and `-webkit-line-clamp`
//! (#1703). Stylo's servo build parses none of them; rinch carries them
//! through the cascade (`crate::webkit_box`).
//!
//! Every number is Chrome 155's on the same markup: the bundled Inter
//! (`Inter-Regular.ttf`, loaded with `@font-face` there and registered as
//! `ProbeFace` here), `body { margin: 0; font: 16px/24px ProbeFace }`, a
//! `width: 200px` box (`.c`). Boxes are from `getBoundingClientRect`, a
//! line's end from a `Range` around its last letter.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DisplayValue;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const TOL: f32 = 0.05;
const TEXT: &str =
    "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
const CSS: &str = "
    body { margin: 0; font: 16px/24px ProbeFace; }
    .c { width: 200px; }
    .clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }
    .child { width: auto; }
";

fn page(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d.load_css(&format!("{CSS}{css}"));
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(400.0, 800.0);
    d
}

fn id(d: &RinchDocument, name: &str) -> u64 {
    rinch_dom::testing::query_selector(&d.tree, &format!("#{name}"))[0]
}

/// `(x, y, width, height)` from the page's origin.
fn rect(d: &RinchDocument, node: u64) -> (f32, f32, f32, f32) {
    let n = d.tree.get(node).unwrap();
    let (mut x, mut y) = (n.layout.x, n.layout.y);
    let mut p = n.parent;
    while let Some(pid) = p {
        let pn = d.tree.get(pid).unwrap();
        x += pn.layout.x;
        y += pn.layout.y;
        p = pn.parent;
    }
    (x, y, n.layout.width, n.layout.height)
}

#[track_caller]
fn assert_rect(got: (f32, f32, f32, f32), want: (f32, f32, f32, f32)) {
    let ok = [
        (got.0, want.0),
        (got.1, want.1),
        (got.2, want.2),
        (got.3, want.3),
    ]
    .iter()
    .all(|(g, w)| (g - w).abs() <= TOL);
    assert!(ok, "got {got:?}, Chrome 155 {want:?}");
}

/// The ellipsis marks of a box's lines: `(line, x)`.
fn marks(d: &RinchDocument, node: u64) -> Vec<(usize, f32)> {
    let n = d.tree.get(node).unwrap();
    let Some(il) = n.text_layout.as_ref() else {
        // Not an IFC root (a flex container's text is in an anonymous
        // item): the marks of the boxes under it.
        return n.children.iter().flat_map(|&c| marks(d, c)).collect();
    };
    il.ellipsis
        .as_ref()
        .map(|e| e.marks.iter().map(|m| (m.line, m.x)).collect())
        .unwrap_or_default()
}

fn lines(d: &RinchDocument, node: u64) -> usize {
    let il = d.tree.get(node).unwrap().text_layout.as_ref().unwrap();
    il.layout.len()
}

/// Chrome lays the four lines out, makes the box two of them tall, and
/// ends the second, `seven eight nine ten`, in "…" right after `ten`
/// (152.11), with or without `overflow: hidden`.
#[test]
fn a_vertical_clamp_ends_the_box_at_its_nth_line_issue_1703() {
    for style in ["", "overflow: hidden"] {
        let d = page(
            "",
            &format!("<div id=a class=\"c clamp\" style=\"{style}\">{TEXT}</div>"),
        );
        let a = id(&d, "a");
        assert_rect(rect(&d, a), (0.0, 0.0, 200.0, 48.0));
        assert_eq!(
            d.tree.get(a).unwrap().computed_style.display,
            DisplayValue::Block
        );
        assert_eq!(lines(&d, a), 4);
        let m = marks(&d, a);
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(m[0].0, 1);
        assert!((m[0].1 - 152.11).abs() <= TOL, "{m:?}");
    }
}

/// `-webkit-line-clamp: 3`: 72 tall, `…` after `thirteen` (166.28). A
/// declared height wins over the clamp's (100), and the "…" stays.
#[test]
fn the_clamp_counts_lines_and_a_height_still_wins_issue_1703() {
    let d = page(
        "",
        &format!(
            "<div id=p class=c style=\"display: -webkit-box; -webkit-box-orient: vertical; \
             -webkit-line-clamp: 3\">{TEXT}</div>\
             <div id=l class=\"c clamp\" style=\"height: 100px; overflow: hidden\">{TEXT}</div>"
        ),
    );
    let p = id(&d, "p");
    assert_rect(rect(&d, p), (0.0, 0.0, 200.0, 72.0));
    let m = marks(&d, p);
    assert!(
        m.len() == 1 && m[0].0 == 2 && (m[0].1 - 166.28).abs() <= TOL,
        "{m:?}"
    );
    let l = id(&d, "l");
    assert_rect(rect(&d, l), (0.0, 72.0, 200.0, 100.0));
    let m = marks(&d, l);
    assert!(
        m.len() == 1 && m[0].0 == 1 && (m[0].1 - 152.11).abs() <= TOL,
        "{m:?}"
    );
}

/// Fewer lines than the clamp: as tall as they are, no "…".
#[test]
fn a_box_within_its_clamp_is_untouched_issue_1703() {
    let d = page("", "<div id=o class=\"c clamp\">one two</div>");
    let o = id(&d, "o");
    assert_rect(rect(&d, o), (0.0, 0.0, 200.0, 24.0));
    assert!(marks(&d, o).is_empty());
}

/// A kept line too long for the "…" is cut where the "…" fits: in a
/// 150px box the second line, 16 `b`s (156.75), keeps 13 (127.36), as
/// Chrome draws `bbbbbbbbbbbbb…`.
#[test]
fn a_full_last_line_is_cut_for_the_ellipsis_issue_1703() {
    let d = page(
        "",
        "<div id=n class=\"c clamp\" style=\"width: 150px\">\
         aaaaaaaaaaaaaaa bbbbbbbbbbbbbbbb cccccccc</div>",
    );
    let n = id(&d, "n");
    assert_rect(rect(&d, n), (0.0, 0.0, 150.0, 48.0));
    let m = marks(&d, n);
    assert!(
        m.len() == 1 && m[0].0 == 1 && (m[0].1 - 127.36).abs() <= 0.1,
        "{m:?}"
    );
    let il = d.tree.get(n).unwrap().text_layout.as_ref().unwrap();
    // The last three `b`s, and the line's trailing space after them.
    let hidden: String = il
        .ellipsis
        .as_ref()
        .unwrap()
        .hidden
        .iter()
        .map(|r| &il.text_content[r.clone()])
        .collect();
    assert_eq!(hidden, "bbb ");
}

/// Its inline children stay inline: one paragraph, `<i>nine ten</i>` the
/// last words before the "…" (152.11).
#[test]
fn a_clamped_boxs_inline_children_are_not_blockified_issue_1703() {
    let d = page(
        "",
        "<div id=q class=\"c clamp\">one <b>two</b> three four five six seven eight \
         <i id=i>nine ten</i> eleven twelve</div>",
    );
    let q = id(&d, "q");
    assert_rect(rect(&d, q), (0.0, 0.0, 200.0, 48.0));
    assert_eq!(
        d.tree.get(id(&d, "i")).unwrap().computed_style.display,
        DisplayValue::Inline
    );
    assert_eq!(lines(&d, q), 3);
    let m = marks(&d, q);
    assert!(
        m.len() == 1 && m[0].0 == 1 && (m[0].1 - 152.11).abs() <= TOL,
        "{m:?}"
    );
}

/// `-webkit-line-clamp` does not inherit: a parent's does not clamp a
/// vertical `-webkit-box` inside it, which is then a column (96 tall).
#[test]
fn the_clamp_does_not_inherit_issue_1703() {
    let d = page(
        "",
        &format!(
            "<div style=\"-webkit-line-clamp: 2\"><div id=mi class=c \
             style=\"display: -webkit-box; -webkit-box-orient: vertical\">{TEXT}</div></div>"
        ),
    );
    let mi = id(&d, "mi");
    assert_rect(rect(&d, mi), (0.0, 0.0, 200.0, 96.0));
    assert_eq!(d.tree.get(mi).unwrap().computed_style.line_clamp, None);
}

/// The cascade decides, as for any property: a later `display: block` in
/// the same block, and a more specific rule, beat `-webkit-box` (96 tall,
/// no clamp); `!important` beats a more specific rule (48); and the three
/// can come from different rules (48).
#[test]
fn the_cascade_decides_issue_1703() {
    let d = page(
        "#x { display: block }
         .clampi { display: -webkit-box !important; -webkit-box-orient: vertical;
                   -webkit-line-clamp: 2; }
         #y { display: block }
         .o2 { -webkit-box-orient: vertical } .v { display: -webkit-box; -webkit-line-clamp: 2 }",
        &format!(
            "<div id=j2 class=c style=\"display: -webkit-box; -webkit-box-orient: vertical; \
             -webkit-line-clamp: 2; display: block\">{TEXT}</div>\
             <div id=x class=\"c clamp\">{TEXT}</div>\
             <div id=y class=\"c clampi\">{TEXT}</div>\
             <div id=z class=\"c o2 v\">{TEXT}</div>"
        ),
    );
    assert_rect(rect(&d, id(&d, "j2")), (0.0, 0.0, 200.0, 96.0));
    assert_rect(rect(&d, id(&d, "x")), (0.0, 96.0, 200.0, 96.0));
    assert_rect(rect(&d, id(&d, "y")), (0.0, 192.0, 200.0, 48.0));
    assert_rect(rect(&d, id(&d, "z")), (0.0, 240.0, 200.0, 48.0));
}

/// Neither property clamps without `-webkit-box` and `vertical` both: 96.
#[test]
fn a_clamp_needs_a_vertical_webkit_box_issue_1703() {
    let d = page(
        "",
        &format!(
            "<div id=h class=c style=\"display: -webkit-box; -webkit-line-clamp: 2; \
             overflow: hidden\">{TEXT}</div>\
             <div id=i class=c style=\"-webkit-box-orient: vertical; -webkit-line-clamp: 2; \
             overflow: hidden\">{TEXT}</div>"
        ),
    );
    assert_rect(rect(&d, id(&d, "h")), (0.0, 0.0, 200.0, 96.0));
    assert_rect(rect(&d, id(&d, "i")), (0.0, 96.0, 200.0, 96.0));
}

/// The text of a `-webkit-box` is an anonymous flex item, which does not
/// clip: `text-overflow: ellipsis` draws no "…" (the issue's report).
#[test]
fn a_legacy_box_draws_no_text_overflow_ellipsis_issue_1703() {
    let d = page(
        "",
        "<div id=e style=\"display: -webkit-box; width: 90px; overflow: hidden; \
         white-space: nowrap; text-overflow: ellipsis\">one two three four five</div>",
    );
    let e = id(&d, "e");
    assert_rect(rect(&d, e), (0.0, 0.0, 90.0, 24.0));
    assert!(marks(&d, e).is_empty());
}

/// `-webkit-inline-box` sits in its line: `x` (8.73) before it, its text
/// 13.33 wide in Chrome 155, 13 in rinch (whole pixels, as an `inline-block`).
#[test]
fn an_inline_box_is_inline_level_issue_1703() {
    let html = "<div><span>x</span><div id=f style=\"display: -webkit-inline-box\">in</div>\
         <span>y</span></div>";
    let d = page("", html);
    assert_rect(rect(&d, id(&d, "f")), (8.73, 0.0, 13.0, 24.0));
}

/// A restyle of the clamp alone lays the box out again.
#[test]
fn a_clamp_change_relays_the_box_out_issue_1703() {
    let mut d = page("", &format!("<div id=a class=\"c clamp\">{TEXT}</div>"));
    let a = id(&d, "a");
    d.set_attribute(rinch_core::dom::NodeId(a), "style", "-webkit-line-clamp: 3");
    d.resolve_layout(400.0, 800.0);
    assert_rect(rect(&d, a), (0.0, 0.0, 200.0, 72.0));
    d.set_attribute(rinch_core::dom::NodeId(a), "style", "display: block");
    d.resolve_layout(400.0, 800.0);
    assert_rect(rect(&d, a), (0.0, 0.0, 200.0, 96.0));
    assert!(marks(&d, a).is_empty());
}

/// A legacy box that does not clamp keeps its inline content in one run,
/// as Chrome's anonymous flex item does (review of #1703: as a flex
/// container each inline child was an item). Chrome 155: a vertical box
/// with `Hello <b>world</b> and more` is 24 tall, `<b>` at 43.06 (its edge
/// spaces kept); a horizontal one the same; an email cell
/// `Name: <span>Joe</span><br>Line two` 48; a clamp without
/// `-webkit-box-orient`, text with an `<a>`, 96.
#[test]
fn a_legacy_boxs_inline_content_is_one_run_issue_1703() {
    let d = page(
        "",
        "<div id=A class=c style=\"display: -webkit-box; -webkit-box-orient: vertical\">\
         Hello <b id=Ab>world</b> and more</div>\
         <div id=B class=c style=\"display: -webkit-box\">Name: <span>Joe</span><br>Line two</div>\
         <div id=C class=c style=\"display: -webkit-box; -webkit-line-clamp: 2\">one two three \
         four <a href=\"#\">five six</a> seven eight nine ten eleven twelve thirteen fourteen</div>\
         <div id=D class=c style=\"display: -webkit-box\">Hello <b id=Db>world</b> and more</div>",
    );
    assert_rect(rect(&d, id(&d, "A")), (0.0, 0.0, 200.0, 24.0));
    let x = text_x(&d, "A", "Ab");
    assert!((x - 43.06).abs() <= TOL, "{x}");
    assert_rect(rect(&d, id(&d, "B")), (0.0, 24.0, 200.0, 48.0));
    assert_rect(rect(&d, id(&d, "C")), (0.0, 72.0, 200.0, 96.0));
    assert_rect(rect(&d, id(&d, "D")), (0.0, 168.0, 200.0, 24.0));
    let x = text_x(&d, "D", "Db");
    assert!((x - 43.06).abs() <= TOL, "{x}");
}

/// Each declaration counts at its own importance: `.x`'s normal clamp of 2
/// sits in a block whose `display` is `!important`, and `#y`'s normal 3
/// beats it by specificity (Chrome 155: 72, three lines).
#[test]
fn a_normal_declaration_in_an_important_rule_stays_normal_issue_1703() {
    let d = page(
        ".x { -webkit-line-clamp: 2; display: -webkit-box !important; -webkit-box-orient: vertical }
         #y { -webkit-line-clamp: 3 }",
        &format!("<div id=y class=\"c x\">{TEXT}</div>"),
    );
    assert_rect(rect(&d, id(&d, "y")), (0.0, 0.0, 200.0, 72.0));
}

/// `@supports (-webkit-line-clamp: 2)` holds, as in Chrome 155 (48).
#[test]
fn a_supports_condition_on_the_clamp_holds_issue_1703() {
    let d = page(
        "@supports (-webkit-line-clamp: 2) { .s { display: -webkit-box;
             -webkit-box-orient: vertical; -webkit-line-clamp: 2 } }",
        &format!("<div id=s class=\"c s\">{TEXT}</div>"),
    );
    assert_rect(rect(&d, id(&d, "s")), (0.0, 0.0, 200.0, 48.0));
}

/// The caret x before the first text of element `el`, in IFC root `root`.
fn text_x(d: &RinchDocument, root: &str, el: &str) -> f32 {
    let root = id(d, root);
    let text = d.tree.get(id(d, el)).unwrap().children[0];
    let il = d.tree.get(root).unwrap().text_layout.as_ref().unwrap();
    let flat = rinch_dom::text_query::dom_cursor_to_ifc_offset(&il.text_ranges, text, 0).unwrap();
    d.query_caret_position(root, flat).unwrap().0
}
