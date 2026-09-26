//! `text-overflow: ellipsis` is drawn on each **line** that overflows its
//! block container, whatever `white-space` is (#1091).
//!
//! rinch used to draw the "…" only when the IFC root's own `white-space` was
//! `nowrap` or `pre`, and then only by truncating the whole text to one line.
//! CSS Overflow 3 §3.2 applies the ellipsis to every line box whose inline
//! content overflows the block container, and Chrome 153 does exactly that —
//! measured by a headless screenshot of the same markup with the same bundled
//! Inter (every expectation below is that screenshot, read line by line):
//!
//! | markup (90px, `overflow: hidden; text-overflow: ellipsis`) | Chrome 153 |
//! |---|---|
//! | `Supercalifragilisticexpialidocious` | `Supercali…` |
//! | `ab cd Supercalifragilisticexpialidocious ef gh` | `ab cd` / `Supercali…` / `ef gh` |
//! | `<span style="white-space:nowrap">long run much too long</span>` | `long run …` |
//! | `white-space: pre`, `short⏎Supercalifragilistic⏎tiny` | `short` / `Supercali…` / `tiny` |
//! | `ab cd ef gh ij kl mn` (wraps, nothing overflows) | `ab cd ef gh` / `ij kl mn`, no "…" |
//! | `ab Supercalifragilisticexpialidocious` beside a block child | `Supercali…` on the run's line |
//!
//! The text is set in the bundled Inter under an override name with a declared
//! `line-height`, so the line breaks are this face's and not the host's; the
//! truncation point is asserted only as "a non-empty prefix of the word".

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const WORD: &str = "Supercalifragilisticexpialidocious";
const CSS: &str = "
    body { margin: 0; font-family: ProbeFace; font-size: 16px; line-height: 20px; }
    .c { width: 90px; overflow: hidden; text-overflow: ellipsis; }
    .vis { overflow: visible; }
    .pre { white-space: pre; }
    .nw { white-space: nowrap; }
    .r { text-align: right; }
";

fn doc() -> RinchDocument {
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
    doc.load_css(CSS);
    doc
}

/// `<div class={class}>{text}</div>`, laid out; returns the div and its text.
fn build(class: &str, text: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut d = doc();
    let body = d.body();
    let div = d.create_element("div");
    d.set_attribute(div, "class", class);
    d.append_child(body, div);
    let t = d.create_text(text);
    d.append_child(div, t);
    d.resolve_layout(400.0, 300.0);
    (d, div, t)
}

/// The lines of the inline layout that draws `text`, each trimmed at its end.
fn lines(d: &RinchDocument, text: NodeId) -> Vec<String> {
    let root = d.tree.get(text.0).unwrap().ifc_root.expect("text has an IFC root");
    let il = d.tree.get(root).unwrap().text_layout.as_ref().expect("root is laid out");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].trim_end().to_string())
        .collect()
}

/// `line` is a non-empty proper prefix of [`WORD`] followed by "…".
fn is_cut_word(line: &str) -> bool {
    line.strip_suffix('\u{2026}')
        .is_some_and(|p| !p.is_empty() && p.len() < WORD.len() && WORD.starts_with(p))
}

#[test]
fn an_unbreakable_word_under_white_space_normal_gets_the_ellipsis() {
    let (d, _, t) = build("c", WORD);
    let got = lines(&d, t);
    assert!(
        got.len() == 1 && is_cut_word(&got[0]),
        "Chrome 153: one line, `Supercali…`; got {got:?}"
    );
}

#[test]
fn only_the_overflowing_line_of_a_wrapped_paragraph_gets_the_ellipsis() {
    let (d, _, t) = build("c", &format!("ab cd {WORD} ef gh"));
    let got = lines(&d, t);
    assert_eq!(got.len(), 3, "Chrome 153: three lines; got {got:?}");
    assert_eq!(got[0], "ab cd", "{got:?}");
    assert!(is_cut_word(&got[1]), "{got:?}");
    assert_eq!(got[2], "ef gh", "{got:?}");
}

#[test]
fn each_overflowing_line_of_pre_text_gets_its_own_ellipsis() {
    // The first line fits and must survive; the old whole-text truncation
    // kept only a prefix of the paragraph, which here happens to be the
    // same — so the overflowing line is put FIRST in the second case.
    let (d, _, t) = build("c pre", &format!("short\n{WORD}\ntiny"));
    let got = lines(&d, t);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0], "short", "{got:?}");
    assert!(is_cut_word(&got[1]), "{got:?}");
    assert_eq!(got[2], "tiny", "{got:?}");

    let (d, _, t) = build("c pre", &format!("{WORD}\ntiny"));
    let got = lines(&d, t);
    assert_eq!(got.len(), 2, "the lines after an overflowing one stay: {got:?}");
    assert!(is_cut_word(&got[0]), "{got:?}");
    assert_eq!(got[1], "tiny", "{got:?}");
}

#[test]
fn nowrap_on_an_inline_span_overflows_the_line_and_gets_the_ellipsis() {
    span_on_one_line("nw");
    // `pre` forbids wrapping too.
    span_on_one_line("pre");
}

fn span_on_one_line(class: &str) {
    let mut d = doc();
    let body = d.body();
    let div = d.create_element("div");
    d.set_attribute(div, "class", "c");
    d.append_child(body, div);
    let span = d.create_element("span");
    d.set_attribute(span, "class", class);
    d.append_child(div, span);
    let t = d.create_text("long run much too long");
    d.append_child(span, t);
    d.resolve_layout(400.0, 300.0);
    let got = lines(&d, t);
    assert_eq!(got.len(), 1, "{class}: Chrome 153: the span does not wrap; got {got:?}");
    let prefix = got[0]
        .strip_suffix('\u{2026}')
        .unwrap_or_else(|| panic!("Chrome 153: `long run …`; got {got:?}"));
    assert!(
        !prefix.is_empty() && "long run much too long".starts_with(prefix),
        "{got:?}"
    );
}

#[test]
fn an_unbreakable_word_beside_a_block_child_gets_the_ellipsis() {
    // The run is laid out by an anonymous block box (#1071).
    let mut d = doc();
    let body = d.body();
    let div = d.create_element("div");
    d.set_attribute(div, "class", "c");
    d.append_child(body, div);
    let t = d.create_text(WORD);
    d.append_child(div, t);
    let child = d.create_element("div");
    d.append_child(div, child);
    let ct = d.create_text("block");
    d.append_child(child, ct);
    d.resolve_layout(400.0, 300.0);
    let root = d.tree.get(t.0).unwrap().ifc_root.unwrap();
    assert!(d.tree.get(root).unwrap().is_anonymous_block_box);
    let got = lines(&d, t);
    assert!(got.len() == 1 && is_cut_word(&got[0]), "{got:?}");
}

#[test]
fn controls_draw_no_ellipsis() {
    // Measured in Chrome 153: text that wraps without overflowing, and an
    // overflowing word the box does not clip, draw no "…".
    for (class, text) in [("c", "ab cd ef gh ij kl mn"), ("c vis", WORD)] {
        let (d, _, t) = build(class, text);
        let got = lines(&d, t);
        assert!(
            got.iter().all(|l| !l.contains('\u{2026}')),
            "{class}: {got:?}"
        );
    }
    // The wrapped control does wrap (a positive control that the width bites).
    let (d, _, t) = build("c", "ab cd ef gh ij kl mn");
    assert!(lines(&d, t).len() >= 2);
}

#[test]
fn a_restyle_to_nowrap_and_back_re_decides_the_ellipsis() {
    // The decision now reads the lines; toggling `white-space` must still
    // re-shape (it is a text-layout input) and land where a fresh build does.
    let text = "ab cd ef gh ij kl mn";
    let (mut d, div, t) = build("c", text);
    assert!(lines(&d, t).iter().all(|l| !l.contains('\u{2026}')));
    d.set_attribute(div, "class", "c nw");
    d.resolve_layout(401.0, 300.0);
    let got = lines(&d, t);
    assert!(got.len() == 1 && got[0].ends_with('\u{2026}'), "{got:?}");
    d.set_attribute(div, "class", "c");
    d.resolve_layout(402.0, 300.0);
    let fresh = build("c", text);
    assert_eq!(lines(&d, t), lines(&fresh.0, fresh.2));
}

#[test]
fn nowrap_on_a_display_contents_wrapper_reaches_its_text() {
    // rsx emits a `display: contents` wrapper per `if`/`for`/component site;
    // one that declares only `white-space: nowrap` must still push it, or its
    // text wraps (the wrapper's span is skipped when it changes no text style).
    let mut d = doc();
    let body = d.body();
    let div = d.create_element("div");
    d.set_attribute(div, "class", "c");
    d.append_child(body, div);
    let w = d.create_element("div");
    d.set_attribute(w, "style", "display: contents; white-space: nowrap");
    d.append_child(div, w);
    let t = d.create_text("long run much too long");
    d.append_child(w, t);
    d.resolve_layout(400.0, 300.0);
    let got = lines(&d, t);
    assert!(
        got.len() == 1 && got[0].ends_with('\u{2026}'),
        "Chrome 153: `long run …`; got {got:?}"
    );
}

#[test]
fn text_align_still_lines_up_against_the_box_after_the_cut() {
    // The rebuilt layout is broken at the container's width, so a
    // right-aligned line ends at the box's right edge (90px), not at the
    // widest rebuilt line's (the cut line, ~85px in Inter).
    let (d, _, t) = build("c r", &format!("ab {WORD}"));
    let got = lines(&d, t);
    assert!(got.len() == 2 && got[0] == "ab" && is_cut_word(&got[1]), "{got:?}");
    let root = d.tree.get(t.0).unwrap().ifc_root.unwrap();
    let il = d.tree.get(root).unwrap().text_layout.as_ref().unwrap();
    let m = il.layout.lines().next().unwrap().metrics().clone();
    let right = m.offset + m.advance - m.trailing_whitespace;
    assert!((right - 90.0).abs() < 0.5, "`ab` ends at {right}, want the box edge 90");
}
