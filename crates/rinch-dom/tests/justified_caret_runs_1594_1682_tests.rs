//! Carets, glyph bounds and hit tests in a `text-align: justify` line of
//! several runs land where the glyphs are drawn (#1594, #1682).
//!
//! On parley 0.11.1 a caret after a letter-spaced cluster (a tab, a span
//! with `letter-spacing`, #1594), and every caret in a run after a justified
//! space of an earlier run (#1682), was one justification share short of its
//! glyph, while paint drew the glyph where Chrome does. In parley 0.12 a run's
//! advance (what a caret or a press in a later run adds up), a cluster's
//! advance and a glyph run's offset (what paint draws at) all come from one
//! `EffectiveSpacing` — word and letter spacing plus the line's justification
//! share — so they agree, and these cases pin that against Chrome. Dropping
//! the share from `Run::advance` alone, or from `Cluster::advance` alone,
//! fails both tests.
//!
//! Every expected number is Chrome 157's left edge of each character
//! (`Range.getClientRects`), measured with the same markup and the same face
//! (the bundled Inter, registered under an override name so the host's fonts
//! cannot answer instead) at 16px with a 25px line box. `L1:` marks a
//! character on the second line; later lines are not listed.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::{
    byte_offset_from_position, caret_position_for_offset, glyph_bounds_for_offset,
};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 25.0;
/// Chrome rounds a client rect to 1/64 px and its neighbouring edges can
/// differ by that; a share short is several pixels.
const TOLERANCE: f32 = 0.1;

struct Case {
    id: &'static str,
    width: &'static str,
    white_space: &'static str,
    /// Plain text (`""` style) or a span with that style.
    parts: &'static [(&'static str, &'static str)],
    /// Chrome's left edge of every character on the first two lines, in order.
    chrome: &'static str,
}

/// The issues' own cases (`i1`–`i3` are #1594's table, `c11`, `c19`, `c27`,
/// `c32` #1682's fixtures) and two that mix both shapes: a bold run, then a
/// larger letter-spaced run after justified spaces (`i4`), and a tab before
/// a larger run (`i5`).
const CASES: &[Case] = &[
    Case {
        id: "i1",
        width: "200px",
        white_space: "pre-wrap",
        parts: &[("", "a b cc dd ee ff gg hh ii jj kk ll mm nn")],
        chrome: "0.00 8.98 15.58 25.38 31.98 41.13 50.27 56.86 66.66 76.45 83.06 92.39 101.72 108.31 113.61 119.53 126.14 135.95 145.77 152.36 161.83 171.28 177.89 181.77 185.64 192.25 196.13 200.00 L1:0.00 L1:8.78 L1:17.56 L1:22.06 L1:25.94 L1:29.81 L1:34.31 L1:48.33 L1:62.34 L1:66.84 L1:76.30",
    },
    Case {
        id: "i2",
        width: "200px",
        white_space: "pre-wrap",
        parts: &[
            ("", "a"),
            ("letter-spacing:3px", " "),
            ("", "b cc dd ee ff gg hh ii jj kk ll mm nn"),
        ],
        chrome: "0.00 8.98 18.27 28.06 34.33 43.47 52.61 58.88 68.67 78.47 84.73 94.06 103.39 109.66 114.95 120.88 127.16 136.97 146.78 153.05 162.50 171.97 178.23 182.11 185.98 192.25 196.13 200.02 L1:0.00 L1:8.78 L1:17.56 L1:22.06 L1:25.94 L1:29.81 L1:34.31 L1:48.33 L1:62.34 L1:66.84 L1:76.30",
    },
    Case {
        id: "i3",
        width: "200px",
        white_space: "pre-wrap",
        parts: &[("", "a\tb cc dd ee ff gg hh ii jj kk ll mm nn")],
        chrome: "0.00 8.98 37.09 46.89 52.47 61.61 70.75 76.33 86.13 95.92 101.50 110.83 120.16 125.75 131.05 136.97 142.55 152.36 162.17 167.75 177.22 186.67 192.25 196.13 200.02 L1:0.00 L1:3.88 L1:7.75 L1:12.25 L1:21.03 L1:29.81 L1:34.31 L1:38.19 L1:42.06 L1:46.56 L1:60.58 L1:74.59 L1:79.09 L1:88.55",
    },
    Case {
        id: "i4",
        width: "120px",
        white_space: "normal",
        parts: &[
            ("", "aa "),
            ("font-weight:bold", "bb cc"),
            ("", " dd "),
            ("font-size:20px;letter-spacing:2px", "ee ff"),
            ("", " gg hh ii jj"),
        ],
        chrome: "0.00 8.98 17.97 32.83 42.63 52.42 67.27 76.41 85.56 100.41 110.20 120.00 L1:0.00 L1:13.66 L1:27.31 L1:36.75 L1:45.38 L1:54.78 L1:61.08 L1:70.89 L1:80.70 L1:87.02 L1:96.48 L1:105.94 L1:112.25 L1:116.13 L1:120.00",
    },
    Case {
        id: "i5",
        width: "110px",
        white_space: "pre-wrap",
        parts: &[
            ("", "a\tb "),
            ("font-size:20px", "cc dd"),
            ("", " ee ff gg hh"),
        ],
        chrome: "0.00 8.98 38.25 48.05 54.80 66.22 77.64 85.52 97.75 110.02 L1:0.00 L1:9.33 L1:18.66 L1:23.16 L1:28.45 L1:34.38 L1:38.88 L1:48.69 L1:58.50 L1:63.00 L1:72.45",
    },
    Case {
        id: "c11",
        width: "90px",
        white_space: "pre-wrap",
        parts: &[
            ("", "aa "),
            ("font-size:20px", "bb cc"),
            ("", "   dd ee   ff gg"),
        ],
        chrome: "0.00 8.98 17.97 29.75 41.98 54.23 67.14 78.56 90.00 94.50 99.00 L1:0.00 L1:9.80 L1:19.59 L1:29.72 L1:39.05 L1:48.38 L1:58.52 L1:68.64 L1:78.78 L1:84.08 L1:90.00",
    },
    Case {
        id: "c19",
        width: "80px",
        white_space: "pre-wrap",
        parts: &[("", "a\tb cc   dd ee ff")],
        chrome: "0.00 8.98 41.72 51.52 61.72 70.86 80.02 84.52 89.02 L1:0.00 L1:9.80 L1:19.59 L1:24.09 L1:33.42 L1:42.75 L1:47.25 L1:52.55",
    },
    Case {
        id: "c27",
        width: "90px",
        white_space: "normal",
        parts: &[
            ("", "aa "),
            ("font-size:20px", "bb cc dd"),
            ("", " ee ff gg hh ii"),
        ],
        chrome: "0.00 8.98 17.97 29.75 41.98 54.23 67.14 78.56 90.00 L1:0.00 L1:12.23 L1:24.50 L1:29.83 L1:39.16 L1:48.48 L1:53.81 L1:59.11 L1:65.03 L1:70.38 L1:80.19 L1:90.00",
    },
    Case {
        id: "c32",
        width: "100px",
        white_space: "pre-wrap",
        parts: &[("", "aa\tbb   cc\tdd ee   ff")],
        chrome: "0.00 8.98 17.97 39.16 48.95 58.75 66.41 74.06 81.72 90.86 100.00 L1:0.00 L1:9.80 L1:19.59 L1:24.09 L1:33.42 L1:42.75 L1:47.25 L1:51.75 L1:56.25 L1:61.55",
    },
];

fn layout(case: &Case) -> (RinchDocument, NodeId, String) {
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
    let body = d.body();
    let b = d.create_element("div");
    d.set_attribute(
        b,
        "style",
        &format!(
            "width: {}; white-space: {}; text-align: justify; font-family: ProbeFace; \
             font-size: 16px; line-height: 25px",
            case.width, case.white_space
        ),
    );
    d.append_child(body, b);
    let mut flat = String::new();
    for (style, text) in case.parts {
        let t = d.create_text(text);
        if style.is_empty() {
            d.append_child(b, t);
        } else {
            let span = d.create_element("span");
            d.set_attribute(span, "style", style);
            d.append_child(b, span);
            d.append_child(span, t);
        }
        flat.push_str(text);
    }
    d.resolve_layout(400.0, 300.0);
    (d, b, flat)
}

/// `(byte offset, line, Chrome's left edge)` of each listed character.
fn chrome_edges(case: &Case, flat: &str) -> Vec<(usize, usize, f32)> {
    let edges: Vec<(usize, f32)> = case
        .chrome
        .split_whitespace()
        .map(|e| match e.strip_prefix("L1:") {
            Some(x) => (1, x.parse().unwrap()),
            None => (0, e.parse().unwrap()),
        })
        .collect();
    flat.char_indices()
        .zip(edges)
        .map(|((byte, _), (line, x))| (byte, line, x))
        .collect()
}

#[test]
fn carets_in_a_justified_line_of_several_runs_are_at_chromes_glyph_edges_issues_1594_1682() {
    let (mut wrong, mut checked) = (Vec::new(), 0);
    for case in CASES {
        let (d, b, flat) = layout(case);
        for (byte, line, x) in chrome_edges(case, &flat) {
            checked += 1;
            let (cx, cy) = caret_position_for_offset(&d, b.0, byte).expect("a text layout");
            let glyph = glyph_bounds_for_offset(&d, b.0, byte).expect("a glyph");
            if (cy / LINE).round() as usize != line
                || (cx - x).abs() > TOLERANCE
                || (glyph.x - x).abs() > TOLERANCE
            {
                wrong.push(format!(
                    "{} byte {byte}: Chrome line {line} x {x}, caret ({cx}, {cy}), glyph x {}",
                    case.id, glyph.x
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!(checked, 246, "every listed character was checked");
}

/// A press on the left part of a character puts the caret before it: the
/// hit test is the caret's inverse, by the same justified advances.
#[test]
fn a_press_in_a_justified_line_of_several_runs_finds_the_character_drawn_there_issues_1594_1682() {
    let (mut wrong, mut pressed) = (Vec::new(), 0);
    for case in CASES {
        let (d, b, flat) = layout(case);
        let node = &d.tree.nodes[b.0];
        let layout = &node.text_layout.as_ref().expect("an inline layout").layout;
        let edges = chrome_edges(case, &flat);
        for pair in edges.windows(2) {
            let ((byte, line, x), (_, next_line, next_x)) = (pair[0], pair[1]);
            // A character a press can land on: not the last on its line, and
            // wide enough to hold a point a quarter of the way in.
            if line != next_line || next_x - x < 2.0 {
                continue;
            }
            let px = x + (next_x - x) * 0.25;
            let py = line as f32 * LINE + LINE / 2.0;
            pressed += 1;
            let hit = byte_offset_from_position(layout, px, py);
            if hit != byte {
                wrong.push(format!(
                    "{}: press at ({px}, {py}) found byte {hit}, Chrome draws byte {byte} there",
                    case.id
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(pressed > 150, "only {pressed} presses were made");
}
