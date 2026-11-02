//! A differential against Chrome 155 for soft wraps around atomic inlines,
//! `white-space` seams and padded spans (#1238; the review of PR #1878).
//!
//! `soft_wrap_1238/chrome155.tsv` holds 437 random nested inline trees (any
//! `white-space` on any span, inline-blocks and images, padded spans,
//! preserved newlines and `<br>`s, NBSPs) with Chrome 155's lines, measured
//! once, so CI needs no browser. The review's random corpora were 3,580 cases.
//! The ones checked in are every case where rinch at main, rinch at the PR's
//! first head, or Chrome disagreed, plus a sample of the rest, and only cases
//! in ASCII (a complex script's widths depend on the host's fallback font).
//!
//! A line is compared without its spaces and NBSPs: where a space sits at a
//! soft wrap is not ink, and Chrome's per-character rects put a hanging one
//! on either line. So this pins where lines break. A case whose fifth column
//! is filled is a **known difference**: rinch's lines are pinned there, and
//! the test fails when they change. If they now match Chrome, clear the
//! column (a fix); if they differ in a new way, explain the move.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CORPUS: &str = include_str!("soft_wrap_1238/chrome155.tsv");
const GIF: &str =
    "data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";

fn lines_of(style: &str, inner: &str) -> String {
    use parley::PositionedLayoutItem;
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
    d.set_attribute(c, "style", &format!("font:16px/20px ProbeFace;{style}"));
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    let n = d.tree.get(c.0).unwrap();
    let Some(il) = n.text_layout.as_ref() else {
        return "NOIFC".into();
    };
    let mut lines = Vec::new();
    for line in il.layout.lines() {
        let mut s = String::new();
        // A shaped run is drawn as one glyph run per style in it: read each
        // run's clusters once.
        let mut last_run = None;
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(g) => {
                    let range = g.run().text_range();
                    if last_run.replace(range.clone()) == Some(range) {
                        continue;
                    }
                    for cluster in g.run().clusters() {
                        s.extend(
                            il.text_content[cluster.text_range()]
                                .chars()
                                .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\n' | '\t')),
                        );
                    }
                }
                PositionedLayoutItem::InlineBox(ib) => {
                    last_run = None;
                    if let Some(id) = d.get_attribute(NodeId(ib.id), "id") {
                        s.push_str(&format!("[{id}]"));
                    }
                }
            }
        }
        if !s.is_empty() {
            lines.push(s);
        }
    }
    lines.join("||")
}

#[test]
fn soft_wraps_match_chrome_155() {
    let mut cases = 0;
    let mut known = 0;
    let mut failures = Vec::new();
    for row in CORPUS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<&str> = row.split('\t').collect();
        let [id, style, html, chrome, pinned] = cols[..] else {
            panic!("malformed row: {row}");
        };
        let inner = html
            .replace("GIF", GIF)
            .replace("\\\\", "\u{0}")
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace('\u{0}', "\\");
        let got = lines_of(style, &inner);
        cases += 1;
        if pinned.is_empty() {
            if got != chrome {
                failures.push(format!("{id}: Chrome {chrome}\n    rinch {got}"));
            }
        } else {
            known += 1;
            if got == chrome {
                failures.push(format!(
                    "{id}: now matches Chrome ({chrome}) — clear its known difference"
                ));
            } else if got != pinned {
                failures.push(format!(
                    "{id}: known difference moved\n    Chrome {chrome}\n    pinned {pinned}\n    rinch  {got}"
                ));
            }
        }
    }
    assert!(cases > 400, "the corpus was read: {cases} cases");
    assert!(
        failures.is_empty(),
        "{} of {cases} cases ({known} known differences):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
