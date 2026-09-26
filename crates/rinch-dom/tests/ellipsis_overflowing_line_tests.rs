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
    let root = d
        .tree
        .get(text.0)
        .unwrap()
        .ifc_root
        .expect("text has an IFC root");
    let il = d
        .tree
        .get(root)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("root is laid out");
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
    assert_eq!(
        got.len(),
        2,
        "the lines after an overflowing one stay: {got:?}"
    );
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
    assert_eq!(
        got.len(),
        1,
        "{class}: Chrome 153: the span does not wrap; got {got:?}"
    );
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
    assert!(
        got.len() == 2 && got[0] == "ab" && is_cut_word(&got[1]),
        "{got:?}"
    );
    let root = d.tree.get(t.0).unwrap().ifc_root.unwrap();
    let il = d.tree.get(root).unwrap().text_layout.as_ref().unwrap();
    let m = *il.layout.lines().next().unwrap().metrics();
    let right = m.offset + m.advance - m.trailing_whitespace;
    assert!(
        (right - 90.0).abs() < 0.5,
        "`ab` ends at {right}, want the box edge 90"
    );
}

// ── #1103's review: content the flat rebuild cannot represent ──────────────
//
// A paragraph whose one overflowing word would otherwise be rebuilt as flat
// text in the root's style keeps its layout as on main (clipped, no "…"): a
// kept line's colour, an inline-block chip, a `visibility: hidden` span and a
// smaller span must all survive. Chrome 153 keeps all four *and* draws the
// "…"; that needs the cut made at paint time (#1100).

/// `<div class=c>{build(div)}</div>` with a 120px box; returns the div.
fn rich(build: impl FnOnce(&mut RinchDocument, NodeId)) -> (RinchDocument, NodeId) {
    let mut d = doc();
    d.load_css(
        ".w { width: 120px; } .red { color: rgb(255, 0, 0); } \
                .chip { display: inline-block; width: 14px; height: 10px; } \
                .hid { visibility: hidden; } .small { font-size: 8px; }",
    );
    let body = d.body();
    let div = d.create_element("div");
    d.set_attribute(div, "class", "c w");
    d.append_child(body, div);
    build(&mut d, div);
    d.resolve_layout(400.0, 300.0);
    (d, div)
}

fn span(d: &mut RinchDocument, parent: NodeId, class: &str, text: &str) -> NodeId {
    let s = d.create_element("span");
    d.set_attribute(s, "class", class);
    d.append_child(parent, s);
    let t = d.create_text(text);
    d.append_child(s, t);
    s
}

fn add_text(d: &mut RinchDocument, parent: NodeId, text: &str) {
    let t = d.create_text(text);
    d.append_child(parent, t);
}

/// The root's layout kept whole: text ranges present, no "…" anywhere.
fn assert_kept(d: &RinchDocument, div: NodeId, what: &str) -> usize {
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    assert!(
        !il.text_content.contains('\u{2026}'),
        "{what}: rebuilt flat"
    );
    assert!(!il.text_ranges.is_empty(), "{what}: text ranges lost");
    assert!(
        il.layout.len() >= 2,
        "{what}: the overflow is on a later line"
    );
    il.text_ranges.len()
}

/// Each glyph run's (brush, font size) on the root's layout.
fn runs(d: &RinchDocument, div: NodeId) -> Vec<(String, f32)> {
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    let mut out = Vec::new();
    for line in il.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(g) = item {
                out.push((format!("{:?}", g.style().brush), g.run().font_size()));
            }
        }
    }
    out
}

#[test]
fn a_coloured_span_on_a_kept_line_stays_coloured() {
    let (d, div) = rich(|d, div| {
        span(d, div, "red", "ab cd");
        add_text(d, div, &format!(" ef {WORD} gh"));
    });
    assert_kept(&d, div, "colour");
    let brushes: Vec<String> = runs(&d, div).into_iter().map(|r| r.0).collect();
    let distinct: std::collections::HashSet<_> = brushes.iter().collect();
    assert!(
        distinct.len() >= 2,
        "the red run kept its own brush: {brushes:?}"
    );
}

#[test]
fn an_inline_block_chip_on_a_kept_line_stays_in_place() {
    let mut chip = None;
    let (d, div) = rich(|d, div| {
        add_text(d, div, "ab ");
        let c = d.create_element("span");
        d.set_attribute(c, "class", "chip");
        d.append_child(div, c);
        chip = Some(c);
        add_text(d, div, &format!(" cd {WORD}"));
    });
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    assert!(!il.text_content.contains('\u{2026}'), "rebuilt flat");
    assert_eq!(
        il.layout.inline_boxes().len(),
        1,
        "the chip is still on its line"
    );
    let x = d.tree.get(chip.unwrap().0).unwrap().layout.x;
    assert!(
        x > 10.0,
        "the chip sits after `ab`, not at the origin: x = {x}"
    );
}

#[test]
fn a_hidden_span_on_a_kept_line_stays_hidden() {
    let (d, div) = rich(|d, div| {
        span(d, div, "hid", "secret");
        add_text(d, div, &format!(" {WORD}"));
    });
    // The text ranges are what `TextMask` hides the span's glyphs by; a flat
    // rebuild carries none, and paints "secret".
    assert!(assert_kept(&d, div, "hidden") >= 2);
}

#[test]
fn a_smaller_span_on_a_kept_line_keeps_its_size() {
    let (d, div) = rich(|d, div| {
        span(d, div, "small", "tiny tiny tiny tiny tiny tiny");
        add_text(d, div, &format!(" {WORD}"));
    });
    assert_kept(&d, div, "small");
    assert!(
        runs(&d, div).iter().any(|r| r.1 == 8.0),
        "an 8px run survives: {:?}",
        runs(&d, div)
    );
}

#[test]
fn a_rich_nowrap_root_is_cut_whole_as_before() {
    // `nowrap` on the root with a coloured span: the rebuild cannot keep the
    // colour, and the root's single line is cut as it always was (flat, "…").
    let (d, div) = rich(|d, div| {
        d.set_attribute(div, "class", "c w nw");
        span(d, div, "red", "ab cd");
        add_text(d, div, &format!(" {WORD}"));
    });
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.layout.len(), 1);
    assert!(
        il.text_content.starts_with("ab cd "),
        "{:?}",
        il.text_content
    );
    assert!(
        il.text_content.ends_with('\u{2026}'),
        "{:?}",
        il.text_content
    );
}

#[test]
fn a_span_with_its_own_text_shadow_on_a_kept_line_keeps_it() {
    // A span's own `text-shadow` list is drawn from the text ranges (#1048);
    // a flat rebuild casts the root's list for every run (#1065).
    let (d, div) = rich(|d, div| {
        let s = span(d, div, "", "ab cd");
        d.set_attribute(s, "style", "text-shadow: 2px 2px rgb(0, 0, 255)");
        add_text(d, div, &format!(" ef {WORD}"));
    });
    assert_kept(&d, div, "text-shadow");
}

// ── #1103's second review ───────────────────────────────────────────────────

/// The widest line of the root laid out for `text`, less trailing white space.
fn widest(d: &RinchDocument, div: NodeId) -> f32 {
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    il.layout
        .lines()
        .map(|l| l.metrics().advance - l.metrics().trailing_whitespace)
        .fold(0.0, f32::max)
}

#[test]
fn a_mixed_direction_line_is_cut_inside_the_box() {
    // `Line::runs` is in visual order; summing clusters that way, an RTL line
    // holding a Latin run reached its logical end first and was not cut
    // (303.94px in a 120px box). Main cut it to `אבגד הוז abcd…`, 119.01px.
    for text in [
        "אבגד הוז abcdefghijklmnopqrstuvwxyz חטי כלמ נסע",
        "אבגד abcdefghijklmnop הוזחטיכלמנסעפצקרשת",
    ] {
        let (d, div) = rich(|d, div| {
            d.set_attribute(div, "class", "c w pre");
            add_text(d, div, text);
        });
        let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
        assert!(
            il.text_content.contains('\u{2026}'),
            "{:?}",
            il.text_content
        );
        let w = widest(&d, div);
        assert!(
            w <= 120.01,
            "{text:?}: the cut line is {w}px in a 120px box"
        );
    }
}

#[test]
fn a_justified_paragraph_keeps_its_justification() {
    // Kept lines are rejoined by hard breaks, which parley does not justify,
    // so a justified root is left clipped and justified, as on main.
    let (d, div) = rich(|d, div| {
        d.set_attribute(div, "style", "text-align: justify");
        add_text(
            d,
            div,
            &format!("aa bb cc dd ee ff gg hh ii jj {WORD} kk ll mm nn oo pp qq rr"),
        );
    });
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    assert!(!il.text_content.contains('\u{2026}'));
    let first = il.layout.lines().next().unwrap();
    let sum: f32 = first
        .runs()
        .flat_map(|r| r.clusters().map(|c| c.advance()).collect::<Vec<_>>())
        .sum();
    assert!(sum >= 119.0, "line 0 is justified to the box: {sum}");
}

#[test]
fn a_span_background_on_a_kept_line_survives() {
    let (d, div) = rich(|d, div| {
        let s = span(d, div, "", "ab cd");
        d.set_attribute(s, "style", "background-color: rgb(255, 255, 0)");
        add_text(d, div, &format!(" ef {WORD}"));
    });
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    assert!(
        !il.background_spans.is_empty(),
        "the span's background survives"
    );
}

#[test]
fn a_wavy_underline_on_the_root_survives() {
    let (d, div) = rich(|d, div| {
        d.set_attribute(
            div,
            "style",
            "text-decoration: underline wavy rgb(255, 0, 0)",
        );
        add_text(d, div, &format!("ab cd {WORD} ef"));
    });
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    assert!(
        !il.decoration_spans.is_empty(),
        "the wavy underline survives"
    );
}

#[test]
fn a_line_of_hanging_spaces_is_not_cut() {
    // `pre-wrap` spaces hang past the box; the line's content is `ab`, which
    // fits. Only the word's line is cut.
    let (d, div) = rich(|d, div| {
        d.set_attribute(div, "style", "white-space: pre-wrap");
        add_text(d, div, &format!("ab{} {WORD}", " ".repeat(60)));
    });
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    let first = &il.text_content[il.layout.lines().next().unwrap().text_range()];
    assert!(
        !first.contains('\u{2026}'),
        "the hanging-space line fits: {first:?}"
    );
    assert!(
        il.text_content.contains('\u{2026}'),
        "the word's line is cut"
    );
}

#[test]
fn a_cut_line_shaped_again_stays_inside_the_box() {
    // Kerning pairs (AV, Ty, WA) broken by the cut make the rebuilt line up
    // to 1.26px wider than the sum it was cut by; the cut steps back a cluster
    // when that happens. Swept over 400 widths (without the step back, 23 of
    // 1480 overflowed in the review's sweep).
    let text = "AVATAR office Type Ty fi fl affluent WAVY Toyota. WAVE ffi LT.AV";
    let mut bad = Vec::new();
    let mut w = 30.0f32;
    while w < 230.0 {
        let mut d = doc();
        d.load_css(&format!(
            ".s {{ width: {w}px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}"
        ));
        let body = d.body();
        let div = d.create_element("div");
        d.set_attribute(div, "class", "s");
        d.append_child(body, div);
        add_text(&mut d, div, text);
        d.resolve_layout(800.0, 300.0);
        // Against the box as laid out (Taffy rounds it to whole pixels), not
        // the declared width.
        let bx = d.tree.get(div.0).unwrap().layout.width;
        let over = widest(&d, div) - bx;
        if over > 0.01 {
            bad.push((w, over));
        }
        w += 0.5;
    }
    assert!(
        bad.is_empty(),
        "{} widths overflow: {:?}",
        bad.len(),
        &bad[..bad.len().min(8)]
    );
}
