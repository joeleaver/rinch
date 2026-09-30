//! Each text node collapses white space by its **own** `white-space`, not the
//! IFC root's (#1192).
//!
//! CSS Text 3 §4.1.1 applies `white-space-collapse` per character, from the
//! element the text belongs to. rinch chose one mode for the whole inline
//! formatting context from the root, so a `pre` span inside a `normal`
//! paragraph collapsed its spaces, and a `normal` span inside `pre` kept them.
//!
//! At a boundary between modes: a preserved space is not collapsible, so a
//! collapsible space after one is kept (phase I removes a collapsible space
//! only after another *collapsible* space), and a collapsible space before
//! one is not at a line's end. A preserved newline is a forced break: the
//! collapsible spaces before it end a line and go, the ones after it start
//! one and go. `pre-line` collapses spaces and tabs and keeps newlines as
//! forced breaks, removing the spaces around them.
//!
//! Every number is Chrome 153's on the same markup (bundled Inter registered
//! as `ProbeFace`, 16px/25px, `* { margin: 0 }`), measured as the width and
//! height of a shrink-to-fit `inline-block` around the content. `a` is 8.98,
//! `b` 9.8, `c` 8.64 and a space 4.5 wide.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::{dom_cursor_to_ifc_offset, ifc_offset_to_dom_cursor};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d
}

fn find(d: &RinchDocument, id: usize, want: &str) -> Option<usize> {
    let n = d.tree.get(id)?;
    if n.attributes.get("id").map(String::as_str) == Some(want) {
        return Some(id);
    }
    n.children.iter().find_map(|&c| find(d, c, want))
}

/// Lay out `inner` in a shrink-to-fit `inline-block` `#m` inside a 300px
/// container styled `container_style`; returns `#m`'s laid-out text, width
/// and height.
fn measure(container_style: &str, inner: &str) -> (String, f32, f32) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("width:300px;font:16px/25px ProbeFace;{container_style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(
        c,
        &format!("<span id=\"m\" style=\"display:inline-block\">{inner}</span>"),
    );
    d.resolve_layout(800.0, 600.0);
    let m = find(&d, c.0, "m").expect("#m");
    let n = d.tree.get(m).unwrap();
    let text = n
        .text_layout
        .as_ref()
        .map(|l| l.text_content.clone())
        .unwrap_or_default();
    (text, n.layout.width, n.layout.height)
}

const PRE: &str = "white-space:pre";
const PRE_LINE: &str = "white-space:pre-line";

fn span(ws: &str, inner: &str) -> String {
    format!("<span style=\"{ws}\">{inner}</span>")
}

/// `(name, container style, inner html, laid-out text, Chrome width, Chrome
/// height)`.
type Case = (&'static str, &'static str, String, &'static str, f32, f32);

fn check(cases: &[Case]) {
    let mut failures = Vec::new();
    for (name, cs, inner, text, w, h) in cases {
        let (got_t, got_w, got_h) = measure(cs, inner);
        if got_t != *text || (got_w - w).abs() > 0.5 + 1e-3 || (got_h - h).abs() > 1e-3 {
            failures.push(format!(
                "{name}: Chrome 153 lays out {text:?} {w}x{h}, rinch {got_t:?} {got_w}x{got_h}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The issue's table: a `pre` span keeps its spaces in a `normal` root, a
/// `normal` span collapses its own in a `pre` root.
#[test]
fn a_text_node_collapses_by_its_own_elements_white_space() {
    check(&[
        (
            "pre span in a normal root",
            "",
            format!("a{}", span(PRE, "   b")),
            "a   b",
            32.28,
            25.0,
        ),
        (
            "normal span in a pre root",
            PRE,
            format!("a{}", span("white-space:normal", "   b")),
            "a b",
            23.28,
            25.0,
        ),
        (
            "pre span between collapsed runs",
            "",
            format!("a   {}c", span(PRE, "b   ")),
            "a b   c",
            45.92,
            25.0,
        ),
        (
            "nowrap span in a pre root collapses",
            PRE,
            format!("a{}", span("white-space:nowrap", "   b")),
            "a b",
            23.28,
            25.0,
        ),
        (
            "pre-wrap span's spaces at the end are kept and counted",
            "",
            format!("a{}", span("white-space:pre-wrap", "  ")),
            "a  ",
            17.98,
            25.0,
        ),
    ]);
}

/// A preserved space is not collapsible: a collapsible space after it is not
/// removed by phase I, and a collapsible space before it is not at the end of
/// a line.
#[test]
fn a_collapsible_space_beside_a_preserved_one_is_kept() {
    check(&[
        (
            "collapsible space after a preserved one",
            "",
            format!("{} b", span(PRE, "a ")),
            "a  b",
            27.78,
            25.0,
        ),
        (
            "collapsible space before preserved spaces at the end",
            "",
            format!("a {}", span(PRE, "  ")),
            "a   ",
            22.48,
            25.0,
        ),
        (
            "preserved space between two collapsible ones",
            "",
            format!("a {} b", span(PRE, " ")),
            "a   b",
            32.28,
            25.0,
        ),
        (
            "preserved spaces at the start, then a collapsible run",
            "",
            format!("{}  a", span(PRE, "  ")),
            "   a",
            22.48,
            25.0,
        ),
        (
            "normal span in a pre root, preserved spaces on both sides",
            PRE,
            format!("a  {}  c", span("white-space:normal", "  b  ")),
            "a   b   c",
            54.92,
            25.0,
        ),
        (
            "pre span in a pre-line root",
            PRE_LINE,
            format!("a  {}  c", span(PRE, "  b  ")),
            "a   b   c",
            54.92,
            25.0,
        ),
        (
            "a preserved space after a <br> makes the next space not line-initial",
            "",
            format!("a<br>{} b", span(PRE, " ")),
            "a\n  b",
            18.8,
            50.0,
        ),
    ]);
}

/// A preserved newline is a forced break: the collapsible spaces after it
/// start a line and go.
#[test]
fn a_preserved_newline_ends_the_line_for_collapsible_spaces() {
    check(&[(
        "collapsible spaces after a pre span's newline",
        "",
        format!("{}  b", span(PRE, "a\n")),
        "a\nb",
        9.8,
        50.0,
    )]);
    // The collapsible space *before* a preserved newline is removed from the
    // line as it is before a `<br>` (CSS Text 3 §4.1.2 phase II). Chrome 153
    // lays it out removed too — `a` is right-aligned flush against the edge —
    // but counts it in the max-content width (13.48, not 9.8), which is why
    // only the text and the height are pinned here.
    let (text, _, h) = measure("", &format!("a {}", span(PRE, "\nb")));
    assert_eq!((text.as_str(), h), ("a\nb", 50.0));
}

/// `pre-line` collapses spaces and tabs, keeps newlines as forced breaks, and
/// removes the spaces around them and at the end of the text (#1043's first
/// half).
#[test]
fn pre_line_collapses_spaces_and_keeps_newlines() {
    check(&[
        (
            "a run of spaces",
            PRE_LINE,
            "a    b".into(),
            "a b",
            23.28,
            25.0,
        ),
        (
            "spaces around a newline",
            PRE_LINE,
            "a  \n  b".into(),
            "a\nb",
            9.8,
            50.0,
        ),
        (
            "trailing spaces",
            PRE_LINE,
            "abc   ".into(),
            "abc",
            27.92,
            25.0,
        ),
        (
            "pre-line span in a normal root",
            "",
            format!("a{}", span(PRE_LINE, "x\ny")),
            "ax\ny",
            17.72,
            50.0,
        ),
        (
            "normal span in a pre-line root converts its newline",
            PRE_LINE,
            format!("a{}", span("white-space:normal", "x\ny")),
            "ax y",
            31.22,
            25.0,
        ),
        (
            "pre span after a pre-line newline",
            PRE_LINE,
            format!("a \n{}b", span(PRE, " ")),
            "a\n b",
            14.3,
            50.0,
        ),
    ]);
}

/// DOM ↔ flat offsets across a collapsed run followed by a preserved one:
/// the `pre` text node maps byte for byte after the collapsed node shrank.
#[test]
fn offsets_across_mixed_modes_round_trip() {
    let mut d = doc();
    let body = d.body();
    let p = d.create_element("div");
    d.set_attribute(p, "style", "width:300px;font:16px/25px ProbeFace");
    d.append_child(body, p);
    let t1 = d.create_text("a   ");
    d.append_child(p, t1);
    let s = d.create_element("span");
    d.set_attribute(s, "style", PRE);
    let t2 = d.create_text("b\t ");
    d.append_child(s, t2);
    d.append_child(p, s);
    let t3 = d.create_text("  c");
    d.append_child(p, t3);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    // `a ` + `b` + a tab as four spaces + ` ` + ` c` (kept: after a
    // preserved space).
    assert_eq!(il.text_content, "a b      c");
    let d2f = |node: NodeId, n: usize| -> Vec<usize> {
        (0..=n)
            .map(|o| dom_cursor_to_ifc_offset(&il.text_ranges, node.0, o).unwrap())
            .collect()
    };
    assert_eq!(d2f(t1, 4), vec![0, 1, 2, 2, 2]);
    assert_eq!(d2f(t2, 3), vec![2, 3, 7, 8]);
    assert_eq!(d2f(t3, 3), vec![8, 9, 9, 10]);
    let f2d: Vec<(usize, usize)> = (0..=10)
        .map(|f| {
            let (node, off) = ifc_offset_to_dom_cursor(&il.text_ranges, f, false).unwrap();
            (node, off)
        })
        .collect();
    // Flat 2 is the end of `t1` and the start of `t2`; a forward lookup takes
    // the later node.
    assert_eq!(f2d[3], (t2.0, 1));
    assert_eq!(f2d[7], (t2.0, 2));
    assert_eq!(f2d[9], (t3.0, 2));
    assert_eq!(f2d[10], (t3.0, 3));
}

/// Restyling a span's `white-space` re-collapses its text on the next layout
/// — a plain span's, and a `display: contents` wrapper's (what `rsx!` puts
/// around a reactive text).
#[test]
fn restyling_a_spans_white_space_relays_its_text() {
    restyle_relays("");
    restyle_relays("display:contents;");
}

fn restyle_relays(extra: &str) {
    let mut d = doc();
    let body = d.body();
    let p = d.create_element("div");
    d.set_attribute(p, "style", "width:300px;font:16px/25px ProbeFace");
    d.append_child(body, p);
    let a = d.create_text("a");
    d.append_child(p, a);
    let s = d.create_element("span");
    d.set_attribute(s, "style", extra);
    let t = d.create_text("   b");
    d.append_child(s, t);
    d.append_child(p, s);
    d.resolve_layout(800.0, 600.0);
    let text = |d: &RinchDocument| {
        d.tree
            .get(p.0)
            .unwrap()
            .text_layout
            .as_ref()
            .unwrap()
            .text_content
            .clone()
    };
    assert_eq!(text(&d), "a b");
    d.set_attribute(s, "style", &format!("{extra}{PRE}"));
    d.resolve_layout(800.0, 600.0);
    assert_eq!(text(&d), "a   b", "{extra}");
    d.set_attribute(s, "style", &format!("{extra}white-space:normal"));
    d.resolve_layout(800.0, 600.0);
    assert_eq!(text(&d), "a b");
}
