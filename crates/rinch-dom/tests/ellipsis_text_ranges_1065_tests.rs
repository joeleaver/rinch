//! A `text-overflow: ellipsis` layout keeps its text ranges (#1065, #853).
//!
//! The "…" used to be drawn by rebuilding the IFC as flat text in the root's
//! own style, and that rebuild first recorded no `text_ranges`, which every
//! per-element paint decision reads: a span's own `text-shadow` list (#1048),
//! its `visibility` (#829) and its live colour (#679). With none, every byte
//! took the root's answer: a span's shadow was not drawn (#1065), a hidden
//! span was drawn (#853), and a span's colour was the root's. Since #1100 the
//! lines are kept and cut at paint, so the ranges are the layout's own; the
//! "…" is drawn apart in the root's style, as Chrome 155 draws it in the
//! block's style.
//!
//! Chrome 155 on the same markup (bundled Inter): the span's shadow is drawn
//! under its visible text and not under the "…"; a hidden span's glyphs are
//! not drawn and keep their room; a span's colour is kept.
//!
//! Every fixture checks it is on the ellipsis path first (the "…" is in the
//! laid-out text), so a fixture that silently stopped cutting fails rather
//! than passing for the wrong reason. Text is the bundled Inter at a declared
//! size and line height.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 600.0;
const VH: f32 = 200.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const WORD: &str = "Supercalifragilisticexpialidocious";
const CSS: &str = "
    body { margin: 0; font-family: T1065; font-size: 32px; line-height: 40px;
           color: rgb(0, 0, 0); }
    .c { width: 260px; overflow: hidden; text-overflow: ellipsis; }
    .nw { white-space: nowrap; }
    .sh { text-shadow: 6px 0 0 rgb(255, 0, 0); }
    .root-sh { text-shadow: 0 6px 0 rgb(0, 0, 255); }
    .none { text-shadow: none; }
    .hid { visibility: hidden; }
    .red { color: rgb(255, 0, 0); }
";

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("T1065"),
            ..Default::default()
        }),
    );
    doc.load_css(CSS);
    doc
}

fn div(d: &mut RinchDocument, class: &str) -> NodeId {
    let body = d.body();
    let e = d.create_element("div");
    d.set_attribute(e, "class", class);
    d.append_child(body, e);
    e
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

fn paint(d: &mut RinchDocument) -> Vec<u8> {
    d.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    painter.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &d.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut d.font_cx,
        &mut cx,
    );
    painter.pixels().to_vec()
}

/// What paint draws of IFC root `root`'s lines; asserts the "…" was drawn.
fn cut_text(d: &RinchDocument, root: NodeId) -> String {
    let il = d.tree.get(root.0).unwrap().text_layout.as_ref().unwrap();
    il.painted_text()
        .unwrap_or_else(|| panic!("not on the ellipsis path: {:?}", il.text_content))
}

/// Pixels in rows `y0..y1` for which `f(r, g, b)` holds.
fn count(px: &[u8], y0: usize, y1: usize, f: impl Fn(u8, u8, u8) -> bool) -> usize {
    let w = VW as usize;
    px[y0 * w * 4..y1 * w * 4]
        .chunks(4)
        .filter(|p| f(p[0], p[1], p[2]))
        .count()
}

fn reddish(r: u8, g: u8, b: u8) -> bool {
    r > 200 && g < 90 && b < 90
}

fn bluish(r: u8, g: u8, b: u8) -> bool {
    b > 200 && r < 90 && g < 90
}

fn dark(r: u8, g: u8, b: u8) -> bool {
    r < 100 && g < 100 && b < 100
}

/// The x of the right-most pixel in rows `y0..y1` for which `f` holds.
fn max_x(px: &[u8], y0: usize, y1: usize, f: impl Fn(u8, u8, u8) -> bool) -> Option<usize> {
    let w = VW as usize;
    (y0..y1)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = &px[(y * w + x) * 4..];
            f(p[0], p[1], p[2])
        })
        .map(|(x, _)| x)
        .max()
}

// ── #1065: a span's own text-shadow ─────────────────────────────────────────

/// `nowrap` root, the span cut by the "…": its shadow is drawn, and not
/// under the "…" (which casts the root's list: none).
#[test]
fn a_spans_shadow_is_drawn_on_a_nowrap_line_cut_whole() {
    let mut d = doc();
    let root = div(&mut d, "c nw");
    add_text(&mut d, root, "Ha");
    span(&mut d, root, "sh", "mburgburgburgburgburg");
    let px = paint(&mut d);
    let text = cut_text(&d, root);
    assert!(text.starts_with("Hamburg"), "{text:?}");
    let red = count(&px, 0, 40, reddish);
    assert!(red > 100, "the span's shadow is drawn: {red} red px");
    // The "…" is the last glyph and takes the root's (empty) list: shadowed,
    // its red would reach 6px past the last ink; unshadowed, the red stops
    // at the span's last glyph, left of the "…"'s last dot.
    let last_black = max_x(&px, 0, 40, dark).unwrap();
    let last_red = max_x(&px, 0, 40, reddish).unwrap();
    assert!(
        last_red < last_black,
        "the \u{2026} casts no red: red to {last_red}, ink to {last_black}"
    );
}

/// The twin: a span declaring `text-shadow: none` under a shadowed root
/// casts nothing, while the root's own text casts the root's list.
#[test]
fn a_span_with_no_shadow_under_a_shadowed_root_casts_none() {
    let mut with = doc();
    let root = div(&mut with, "c nw root-sh");
    add_text(&mut with, root, "Ha");
    span(&mut with, root, "none", "mburgburgburgburgburg");
    let px = paint(&mut with);
    cut_text(&with, root);
    let blue_none = count(&px, 0, 60, bluish);

    let mut all = doc();
    let root = div(&mut all, "c nw root-sh");
    add_text(&mut all, root, "Ha");
    span(&mut all, root, "", "mburgburgburgburgburg");
    let px_all = paint(&mut all);
    cut_text(&all, root);
    let blue_all = count(&px_all, 0, 60, bluish);

    assert!(
        blue_none > 20,
        "`Ha` and the \u{2026} cast the root's: {blue_none}"
    );
    assert!(
        blue_none * 2 < blue_all,
        "the span casts nothing: {blue_none} blue px against {blue_all} with no `none`"
    );
}

// ── #853: a `visibility: hidden` span ───────────────────────────────────────

/// `nowrap` root: the hidden span's glyphs are not drawn.
#[test]
fn a_hidden_span_is_not_drawn_on_a_nowrap_line_cut_whole() {
    let ink = |class: &str| {
        let mut d = doc();
        let root = div(&mut d, "c nw");
        add_text(&mut d, root, "ab ");
        span(&mut d, root, class, "HHHH");
        add_text(&mut d, root, " and a long tail that is cut off");
        let px = paint(&mut d);
        cut_text(&d, root);
        count(&px, 0, 40, dark)
    };
    let shown = ink("");
    let hidden = ink("hid");
    assert!(hidden > 200, "the rest of the line is drawn: {hidden}");
    assert!(
        hidden + 300 < shown,
        "the four Hs are not drawn: {hidden} dark px hidden, {shown} shown"
    );
}

// ── The colour of a span on a line cut whole (part of #1100) ────────────────

#[test]
fn a_red_span_stays_red_on_a_nowrap_line_cut_whole() {
    let mut d = doc();
    let root = div(&mut d, "c nw");
    add_text(&mut d, root, "ab ");
    span(&mut d, root, "red", "HHHH");
    add_text(&mut d, root, " and a long tail that is cut off");
    let px = paint(&mut d);
    cut_text(&d, root);
    let red = count(&px, 0, 40, reddish);
    assert!(red > 200, "the span is drawn red: {red} red px");
}

// ── A wrapped paragraph: the ranges are mapped past a cut line ──────────────

/// Line 1 is the word, cut to `Super…` — many bytes shorter than it was —
/// and the span is on line 2, so its range lands on the rebuilt text only if
/// it is moved back by what the cut removed.
#[test]
fn a_spans_shadow_on_the_line_after_a_cut_is_drawn() {
    let mut d = doc();
    let root = div(&mut d, "c");
    add_text(&mut d, root, &format!("{WORD} ab "));
    span(&mut d, root, "sh", "cdcd");
    add_text(&mut d, root, " ef");
    let px = paint(&mut d);
    let text = cut_text(&d, root);
    assert!(text.contains("ab cdcd"), "{text:?}");
    assert_eq!(count(&px, 0, 40, reddish), 0, "nothing red on the cut line");
    let red = count(&px, 40, 80, reddish);
    assert!(red > 100, "the span's shadow on line 2: {red} red px");
}

#[test]
fn a_hidden_span_on_the_line_after_a_cut_is_not_drawn() {
    let ink = |class: &str| {
        let mut d = doc();
        let root = div(&mut d, "c");
        add_text(&mut d, root, &format!("{WORD} ab "));
        span(&mut d, root, class, "HHHH");
        add_text(&mut d, root, " ef");
        let px = paint(&mut d);
        cut_text(&d, root);
        (count(&px, 0, 40, dark), count(&px, 40, 80, dark))
    };
    let (cut_shown, line2_shown) = ink("");
    let (cut_hidden, line2_hidden) = ink("hid");
    assert_eq!(cut_shown, cut_hidden, "the cut line is the same picture");
    assert!(
        line2_hidden > 100,
        "`ab` and `ef` are drawn: {line2_hidden}"
    );
    assert!(
        line2_hidden + 300 < line2_shown,
        "the Hs are not drawn: {line2_hidden} hidden, {line2_shown} shown"
    );
}
