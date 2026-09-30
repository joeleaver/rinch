//! Review of PR #1174 (#1154): cases the PR's fixtures do not reach. Every
//! number is Chrome 153's (bundled Inter as `ProbeFace`, 16px/25px).
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn measure(container_style: &str, html: &str) -> (f32, f32) {
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
    d.set_attribute(
        c,
        "style",
        &format!("width:300px;font:16px/25px ProbeFace;{container_style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, html);
    d.resolve_layout(800.0, 600.0);
    fn find(d: &RinchDocument, id: usize) -> Option<usize> {
        let n = d.tree.get(id)?;
        if n.attributes.get("id").map(String::as_str) == Some("m") {
            return Some(id);
        }
        n.children.iter().find_map(|&c| find(d, c))
    }
    let m = find(&d, c.0).expect("#m");
    let _ = NodeId(m);
    let l = d.tree.get(m).unwrap().layout;
    (l.width, l.height)
}

fn ib(s: &str) -> String {
    format!("<span id=\"m\" style=\"display:inline-block\">{s}</span>")
}

/// Kills: the collapse mode not restored after the run (`\u{a0}  x` keeps
/// both spaces), only the first run protected (`\u{a0}x\u{a0}`), the spaces
/// after a line-final NBSP not hung (`x\u{a0} <br>y`: the space before the
/// `<br>` is committed under `Preserve` by the `<br>` arm), and the clusters
/// read in visual order (`direction: rtl`, where the NBSP is its own
/// right-to-left run, visually first).
#[test]
fn nbsp_edges_the_pr_fixtures_do_not_reach() {
    let cases: &[(&str, String, f32, f32)] = &[
        ("nbsp 2sp x", ib("\u{a0}  x"), 17.73, 25.0),
        ("nbsp x nbsp", ib("\u{a0}x\u{a0}"), 17.73, 25.0),
        ("nbsp x sp sp nbsp", ib("\u{a0}x  \u{a0}"), 22.23, 25.0),
        ("x nbsp sp br y", ib("x\u{a0} <br>y"), 13.23, 50.0),
        ("xxxx nbsp sp br y", ib("xxxx\u{a0} <br>y"), 39.44, 50.0),
        (
            "rtl x nbsp",
            "<span id=\"m\" style=\"display:inline-block;direction:rtl\">x\u{a0}</span>".into(),
            13.23,
            25.0,
        ),
    ];
    for (name, html, w, h) in cases {
        let (gw, gh) = measure("", html);
        assert!((gw - w).abs() <= 0.5 + 1e-3, "{name}: Chrome 153 {w}, rinch {gw}");
        assert_eq!(gh, *h, "{name}: height");
    }
}

/// U+2028 / U+2029 are `White_Space` and not ASCII, so the PR commits one at
/// a text node's edge under `Preserve` — and parley takes both as a forced
/// line break (`Whitespace::Newline`). At the base they were trimmed; Chrome
/// 153 renders each as an ordinary character on the same line (4.5px wide).
/// A one-line `x\u{2028}` became two lines.
#[test]
fn a_line_separator_at_an_edge_does_not_break_the_line() {
    for (name, html) in [
        ("x 2028", ib("x\u{2028}")),
        ("2028 x", ib("\u{2028}x")),
        ("x 2029", ib("x\u{2029}")),
        ("x span 2028", ib("x<span>\u{2028}</span>")),
        ("nbsp 2028", ib("x\u{a0}\u{2028}")),
    ] {
        let (_, gh) = measure("", &html);
        assert_eq!(gh, 25.0, "{name}: Chrome 153 and the base: one 25px line");
    }
}

/// An NBSP never hangs, so a min-content size includes it. The PR's
/// `width_keeping_nbsp` clamps its answer to the available width, which is 0
/// in a min-content measure: the box is sized without its NBSP and the NBSP
/// then wraps onto a line of its own. At the base (NBSP trimmed away) each was
/// one 25px line; Chrome 153 is one line, 39.44 wide.
#[test]
fn a_min_content_box_keeps_its_trailing_nbsp_on_its_line() {
    for (name, html, w, h) in [
        (
            "flex item",
            "<div style=\"display:flex;width:20px\"><span id=\"m\">xxxx\u{a0}</span></div>",
            39.44,
            25.0,
        ),
        (
            "grid min-content column",
            "<div style=\"display:grid;grid-template-columns:min-content\"><span id=\"m\">xxxx\u{a0}</span></div>",
            39.44,
            25.0,
        ),
        (
            "flex item, a wrap after the NBSP",
            "<div style=\"display:flex;width:20px\"><span id=\"m\">aa\u{a0} bb</span></div>",
            22.47,
            50.0,
        ),
    ] {
        let (gw, gh) = measure("", html);
        assert!((gw - w).abs() <= 0.5 + 1e-3, "{name}: Chrome 153 {w}, rinch {gw}");
        assert_eq!(gh, h, "{name}: Chrome 153 {h} tall, rinch {gh}");
    }
}
