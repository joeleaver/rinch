//! A differential against Chrome 155 for the seams #1243 sends paragraphs
//! with `pre` spaces through (review rounds of PR #1897).
//!
//! `pre_space_seams_1243/chrome155.tsv` holds, with Chrome 155's lines,
//! measured once so CI needs no browser:
//! - `N`: a `pre` or `nowrap` element ending in punctuation, glued to the
//!   word after it (`<span pre>cccc!</span>dddd`, `Say <code pre>hello!
//!   </code>world`), at widths every 12px from 60px. Chrome keeps `!`, `/`, `|` and `}`
//!   with a letter or digit after them (`ifc::chrome_glues_ascii`);
//! - `M`'s `a0`/`b0` families: a wrapping run of spaces, through a text node
//!   whose spaces collapsed away, before `pre` spaces (`ifc::pre_seam`);
//! - `AB`: the 15 cases of the first review's random corpus that #1897 made
//!   worse than main, each also with `letter-spacing`, `word-spacing`, font
//!   sizes, padding, tabs and all of those taken out.
//!
//! A line is compared without its spaces, tabs and newlines, so this pins
//! where lines break. A case whose fifth column is filled is a **known
//! difference** (#1898): rinch's lines are pinned there and the test fails
//! when they change. If they now match Chrome, clear the column; if they
//! differ in a new way, explain the move.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CORPUS: &str = include_str!("pre_space_seams_1243/chrome155.tsv");

/// Split every text node at each `~`, dropping the marker: two text nodes
/// where the markup has one.
fn split_marks(d: &mut RinchDocument, n: NodeId) {
    for c in d.get_children(n) {
        if d.is_text_node(c) {
            let t = d
                .tree
                .get(c.0)
                .and_then(|x| x.text_content())
                .unwrap_or("")
                .to_string();
            if t.contains('~') {
                let parts: Vec<&str> = t.split('~').collect();
                d.set_text_content(c, parts[0]);
                let next = d.next_sibling(c);
                for p in &parts[1..] {
                    let nt = d.create_text(p);
                    match next {
                        Some(r) => d.insert_before(n, nt, r),
                        None => d.append_child(n, nt),
                    }
                }
            }
        } else {
            split_marks(d, c);
        }
    }
}

fn lines_of(style: &str, inner: &str) -> String {
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
    split_marks(&mut d, c);
    d.resolve_layout(800.0, 600.0);
    let n = d.tree.get(c.0).unwrap();
    let Some(il) = n.text_layout.as_ref() else {
        return "NOIFC".into();
    };
    il.layout
        .lines()
        .filter_map(|l| {
            // A line holding only an inline box has no text range of its own.
            let s: String = il
                .text_content
                .get(l.text_range())
                .unwrap_or("")
                .chars()
                .filter(|c| !matches!(c, ' ' | '\t' | '\n'))
                .collect();
            (!s.is_empty()).then_some(s)
        })
        .collect::<Vec<_>>()
        .join("||")
}

#[test]
fn pre_space_seams_match_chrome_155() {
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
        let inner = html.replace("\\n", "\n").replace("\\t", "\t");
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
    assert!(cases > 450, "the corpus was read: {cases} cases");
    assert!(
        failures.is_empty(),
        "{} of {cases} cases ({known} known differences):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Each line's text (spaces left out) and the x of its first character,
/// for [`lines_of`]'s markup.
fn xs_of(style: &str, inner: &str) -> Vec<(String, f32)> {
    use parley::fontique::{Blob, FontInfoOverride};
    use rinch_core::dom::CaretAffinity;
    use rinch_dom::text_query as tq;
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
    split_marks(&mut d, c);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    il.layout
        .lines()
        .filter_map(|l| {
            let r = l.text_range();
            let text = il.text_content.get(r.clone())?;
            let s: String = text
                .chars()
                .filter(|c| !matches!(c, ' ' | '\t' | '\n'))
                .collect();
            let first = text.char_indices().find(|(_, ch)| *ch != '\n')?.0;
            let x = tq::caret_position_for_offset_layout_with_affinity(
                &il.layout,
                r.start + first,
                CaretAffinity::Downstream,
            )
            .0;
            Some((s, x))
        })
        .collect()
}

/// Known differences in where a line starts, not where it breaks (#1898):
/// cases of the first review's random corpus that #1897 makes worse than
/// main there. In three a line starts right after an inline box and rinch
/// puts it at the far end of the box's line (the after-box family of #1903
/// and #1082); in `s12_1047` (padding) Chrome puts the trailing `pre` spaces
/// on a line of their own. Pinned (rinch's x, Chrome's in the comment) so a
/// change shows.
#[test]
fn known_line_start_differences() {
    let pins: &[(&str, &[f32])] = &[
        // Chrome: 0, 0, 0 (the second line at the start of the box's line).
        ("AB:s11_734.noAll", &[0.0, 187.44, 0.0]),
        // Chrome: 32.50, 8.08, 12.52, 0, 0.
        ("AB:s12_115.noAll", &[32.5, 8.09, 12.52, 54.48, 0.0]),
        // Chrome: 0, 0, 0, 17.97, 0, 0.
        ("AB:s12_820.noAll", &[0.0, 0.0, 0.0, 18.0, 342.26, 0.0]),
        // Chrome: `xoec` at 13.78, then the `pre` spaces at 14.75.
        ("AB:s12_1047.orig", &[1.0]),
    ];
    let mut failures = Vec::new();
    for (id, want) in pins {
        let row = CORPUS
            .lines()
            .find(|l| l.split('\t').next() == Some(*id))
            .unwrap_or_else(|| panic!("{id} is in the corpus"));
        let cols: Vec<&str> = row.split('\t').collect();
        let inner = cols[2].replace("\\n", "\n").replace("\\t", "\t");
        let got: Vec<f32> = xs_of(cols[1], &inner).iter().map(|(_, x)| *x).collect();
        let same =
            got.len() == want.len() && got.iter().zip(*want).all(|(a, b)| (a - b).abs() < 0.06);
        if !same {
            failures.push(format!("{id}: pinned {want:?}, rinch {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
