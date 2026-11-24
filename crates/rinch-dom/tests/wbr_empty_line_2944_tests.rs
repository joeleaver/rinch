//! #2944: a `<wbr>` that starts a line, where what follows it does not fit,
//! gives an empty line, as in Chrome 158.
//!
//! Every height is Chrome 158's (headless), on the same markup: the bundled
//! Inter registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0 }`,
//! each box `width: 1px`. A `<wbr>` is laid out as a zero width space
//! (#2386) in a span that wraps whatever the `white-space` (#2261).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn html(markup: &str) -> RinchDocument {
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
    d.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 600.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

/// The text of each line, without the zero width spaces.
fn visible_lines(d: &RinchDocument, id: &str) -> Vec<String> {
    let n = d.tree.get(by_id(d, id).0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].replace('\u{200b}', ""))
        .collect()
}

/// `(id, white-space, markup, Chrome 158's height)`.
const CASES: &[(&str, &str, &str, f32)] = &[
    // The issue's rows.
    ("lead_normal", "normal", "<wbr>aaaa bbbb", 60.0),
    ("lead_nowrap", "nowrap", "<wbr>aaaa bbbb", 40.0),
    ("dbl_nowrap", "nowrap", "aaaa<wbr><wbr>bbbb", 60.0),
    ("dbl_normal", "normal", "aaaa<wbr><wbr>bbbb", 40.0),
    ("end_normal", "normal", "aaaa bbbb<wbr>", 40.0),
    ("end_nowrap", "nowrap", "aaaa bbbb<wbr>", 20.0),
    // And around them.
    ("lead_pre", "pre", "<wbr>aaaa bbbb", 40.0),
    ("lead1_normal", "normal", "<wbr>aaaa", 40.0),
    ("lead2_normal", "normal", "<wbr><wbr>aaaa", 40.0),
    ("lead2_nowrap", "nowrap", "<wbr><wbr>aaaa", 40.0),
    ("dbl_pre", "pre", "aaaa<wbr><wbr>bbbb", 60.0),
    ("tpl_nowrap", "nowrap", "aaaa<wbr><wbr><wbr>bbbb", 60.0),
    ("sp_normal", "normal", "aaaa <wbr> bbbb", 40.0),
    ("sp_nowrap", "nowrap", "aaaa <wbr> bbbb", 40.0),
    ("sp_pre", "pre", "aaaa <wbr> bbbb", 40.0),
    ("spl_nowrap", "nowrap", " <wbr>aaaa", 40.0),
    ("only_normal", "normal", "<wbr>", 20.0),
];

fn page() -> RinchDocument {
    let markup: String = CASES
        .iter()
        .map(|(id, ws, m, _)| format!("<div id={id} style='width:1px;white-space:{ws}'>{m}</div>"))
        .collect();
    html(&markup)
}

#[test]
fn each_box_is_as_tall_as_in_chrome() {
    let d = page();
    let bad: Vec<String> = CASES
        .iter()
        .filter_map(|&(id, _, m, h)| {
            let got = d.tree.get(by_id(&d, id).0).unwrap().layout.height;
            (got != h).then(|| {
                format!(
                    "#{id} {m:?}: {got}px {:?}, Chrome {h}px",
                    visible_lines(&d, id)
                )
            })
        })
        .collect();
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

/// Where the empty line is: first, or between the two words.
#[test]
fn the_empty_line_is_where_the_wbr_starts_a_line() {
    let d = page();
    assert_eq!(visible_lines(&d, "lead_normal"), ["", "aaaa ", "bbbb"]);
    assert_eq!(visible_lines(&d, "lead_nowrap"), ["", "aaaa bbbb"]);
    assert_eq!(visible_lines(&d, "dbl_nowrap"), ["aaaa", "", "bbbb"]);
    assert_eq!(visible_lines(&d, "dbl_normal"), ["aaaa", "bbbb"]);
}
