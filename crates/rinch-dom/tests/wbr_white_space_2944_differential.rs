//! #2944 (review): `<wbr>`s among elements of mixed `white-space`, against
//! Chrome 158.
//!
//! `wbr_2944/chrome158.tsv` holds 970 boxes, one per line: id, the box's
//! `white-space` (and any more of its style, after a `;`), Chrome 158's
//! height (headless), and the markup. The box is a `div` with `width: 1px`
//! before that style; the page is that of `wbr_empty_line_2944_tests.rs`
//! (the bundled Inter as `ProbeFace`, `font: 16px/20px`, no margins).
//!
//! The ids name the pieces' `white-space` in order:
//! - `g1`, `g2`, `t3` are `aaaa`, one, two or three `<wbr>`s and `bbbb`,
//!   each word in an element; `l1`, `l2` are one or two `<wbr>`s and
//!   `bbbb`.
//! - `s1`, `s2` put a space in an element between `aaaa` and one or two
//!   `<wbr>`s, and `sa` after the `<wbr>`; `mx` splits `aaaa` over two
//!   elements; `ib` puts a 20px inline-block in `aaaa`'s place; `in` puts
//!   each `<wbr>` in an element whose `white-space` it inherits.
//! - `ow1`, `ow20` are `g2` under `overflow-wrap: anywhere`, 1px and 20px
//!   wide; `fs0`, `fs0b` have text of `font-size: 0`.
//! - `a*`, `b*`, `q*`, `r*` are the review's rows and their neighbours, and
//!   `k*` the review's base-level differences.
//!
//! A `<wbr>` after content that overflows stays on the line or starts the
//! next by its own `white-space` and by whether Blink has noticed the
//! overflow yet (parley's `TextStyle::wbr`; `docs/internals/dependencies.md`
//! "Parley Fork"). Where rinch is not Chrome yet, [`KNOWN`] holds rinch's
//! answer.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CORPUS: &str = include_str!("wbr_2944/chrome158.tsv");

/// Blink lays a line out by items, one per DOM text node, and notices that
/// a line overflows at the text node after the one that overflowed. parley
/// lays out text with adjacent equal styles merged, so it cannot see where
/// one text node ends: text that does not wrap, split over two elements,
/// is one text here (#2996). These were 40px before #2944 too, by another way:
/// every `<wbr>` stayed on the line.
const SPLIT_TEXT: &str = "text that does not wrap in two text nodes (#2996)";

/// `(id, rinch's height, why it is not Chrome's)`.
const KNOWN: &[(&str, f32, &str)] = &[
    ("a2", 60.0, SPLIT_TEXT),
    ("a4", 60.0, SPLIT_TEXT),
    ("a8", 60.0, SPLIT_TEXT),
    ("a9", 60.0, SPLIT_TEXT),
    ("mx_nowrap_nowrap_nowrap", 60.0, SPLIT_TEXT),
    ("s2_nowrap_nowrap_normal_nowrap_normal", 60.0, SPLIT_TEXT),
    ("s2_nowrap_nowrap_normal_nowrap_nowrap", 60.0, SPLIT_TEXT),
    ("s2_nowrap_nowrap_nowrap_nowrap_normal", 60.0, SPLIT_TEXT),
    ("s2_nowrap_nowrap_nowrap_nowrap_nowrap", 60.0, SPLIT_TEXT),
    // Before #2944 as well.
    (
        "ib_nowrap_nowrap_normal",
        60.0,
        "an opportunity between a `nowrap` inline-block and a `nowrap` `<wbr>` (#2996)",
    ),
    (
        "fs0_normal",
        52.0,
        "a line of `font-size: 0` text ends at its space (#2996)",
    ),
    (
        "fs0b_normal",
        52.0,
        "a line of `font-size: 0` text ends at its space (#2996)",
    ),
    (
        "k1",
        80.0,
        "`bbbb` split after an empty inline-block between `<wbr>`s (#2996)",
    ),
    ("k3", 40.0, "`break-spaces` space after a `<wbr>` (#2996)"),
];

struct Row {
    id: &'static str,
    style: &'static str,
    chrome: f32,
    markup: &'static str,
}

fn corpus() -> Vec<Row> {
    CORPUS
        .lines()
        .map(|line| {
            let mut cols = line.splitn(4, '\t');
            let mut col = || cols.next().expect("four columns");
            let (id, style, chrome, markup) = (col(), col(), col(), col());
            Row {
                id,
                style,
                chrome: chrome.parse().expect("a height"),
                markup,
            }
        })
        .collect()
}

fn page(rows: &[Row]) -> RinchDocument {
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
    let markup: String = rows
        .iter()
        .map(|r| {
            format!(
                "<div id={} style='width:1px;white-space:{}'>{}</div>",
                r.id, r.style, r.markup
            )
        })
        .collect();
    let body = d.body();
    d.set_inner_html(body, &markup);
    d.resolve_layout(800.0, 600.0);
    d
}

fn height(d: &RinchDocument, id: &str) -> f32 {
    let n: NodeId = d.query_selector(&format!("#{id}")).expect(id);
    d.tree.get(n.0).unwrap().layout.height
}

#[test]
fn the_corpus_is_whole() {
    let rows = corpus();
    assert_eq!(rows.len(), 970);
    let mut ids: Vec<&str> = rows.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), rows.len(), "unique ids");
    for (id, _, _) in KNOWN {
        assert!(ids.binary_search(id).is_ok(), "{id} is in the corpus");
    }
}

#[test]
fn each_box_is_as_tall_as_in_chrome() {
    let rows = corpus();
    let d = page(&rows);
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !KNOWN.iter().any(|(id, _, _)| *id == r.id))
        .filter_map(|r| {
            let got = height(&d, r.id);
            (got != r.chrome).then(|| {
                format!(
                    "#{} ({}) {:?}: {got}px, Chrome {}px",
                    r.id, r.style, r.markup, r.chrome
                )
            })
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {}:\n{}",
        bad.len(),
        rows.len(),
        bad.join("\n")
    );
}

/// Each row asserts rinch's answer, with Chrome's beside it, so a fix
/// shows up here.
#[test]
fn known_differences_from_chrome() {
    let rows = corpus();
    let d = page(&rows);
    for &(id, rinch, why) in KNOWN {
        let row = rows.iter().find(|r| r.id == id).expect(id);
        assert_ne!(rinch, row.chrome, "#{id} is a difference");
        assert_eq!(
            height(&d, id),
            rinch,
            "#{id} {:?}: rinch's known answer (Chrome 158: {}px; {why})",
            row.markup,
            row.chrome
        );
    }
}
