//! `text-overflow: ellipsis` is cut at paint time over the lines as laid out
//! (#1100).
//!
//! Until #1100 the "…" was drawn by rebuilding the IFC root's layout as flat
//! text in the root's own style, so a line cut with an ellipsis lost its
//! inline styling, its inline boxes and its inline backgrounds, and a
//! paragraph whose rebuild would not have been the same picture got no "…"
//! at all. Now the lines are kept and paint hides what is past the cut on
//! each overflowing line and draws the "…" there, as Chrome does.
//!
//! Every expectation is Chrome 155 on the same markup, measured with the
//! bundled Inter at a declared size and line height (pixel extents from a
//! headless screenshot at device scale 1; the markup is [`PAGE`]):
//!
//! - the pill (Pimble's repro, 220px): the pill's background runs to the
//!   box's edge (x 0–219, clipped there, not rounded), the "…" is drawn over
//!   it in the block's black at 16px, its dots at x 201–211, baseline row 15;
//! - a wrapped paragraph (90px) `ab <red>cd</red> Supercalifragilistic…`:
//!   `cd` stays red on line 1, line 2 ends in a "…" (x ≈ 73–82);
//! - a 30px chip before the cut (120px) is drawn (green x 23–52) and the
//!   line ends in "…" (dots up to x 112);
//! - a 40px chip that does not fit before the "…" (60px) is hidden whole,
//!   and the "…" follows `ab ` (dots up to x 34);
//! - the "…" of a blue block whose cut text is a red 24px span is blue and
//!   16px (its dots at x 105–114, rows 209–217);
//! - a block `text-shadow` is cast by the "…" too.
//!
//! Set `RINCH_1100_PNG=<path>` to write the page as rinch paints it.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 320.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const CSS: &str = "
    body { margin: 0; font-family: T1100; font-size: 16px; line-height: 20px;
           background: rgb(255, 255, 255); color: rgb(0, 0, 0); }
    .c { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; height: 40px; }
    .p { line-height: 18px; padding: 0 7px; border-radius: 999px;
         background: rgb(28, 126, 214); color: rgb(255, 255, 255); font-size: 12px; }
    .chip { display: inline-block; width: 30px; height: 10px; background: rgb(0, 255, 0); }
";

/// The page Chrome was measured on (with an `@font-face` for the face).
const PAGE: &str = r#"<div class="c" style="min-width: 0; width: 220px"><span class="p">Porto and the long road along the coast</span></div><div class="c" style="white-space: normal; width: 90px; height: 80px">ab <span style="color: rgb(255, 0, 0)">cd</span> Supercalifragilisticexpialidocious</div><div class="c" style="width: 120px">ab <span class="chip"></span> cdefghijklmnop</div><div class="c" style="width: 60px">ab <span class="chip" style="width: 40px"></span> cdefghijklmnop</div><div class="c" style="width: 120px; color: rgb(0, 0, 255)">ab <span style="color: rgb(255, 0, 0); font-size: 24px">cdefghijklmnopqrst</span></div><div class="c" style="width: 120px; text-shadow: 0 6px 0 rgb(255, 0, 0)">ab cdefghijklmnopqrstuvw</div>"#;

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("T1100"),
            ..Default::default()
        }),
    );
    doc.load_css(CSS);
    doc
}

struct Page {
    doc: RinchDocument,
    px: Vec<u8>,
    /// The six blocks, in order.
    blocks: Vec<NodeId>,
}

fn page() -> Page {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(body, PAGE);
    let blocks: Vec<NodeId> = d
        .get_children(body)
        .into_iter()
        .filter(|&n| d.tree.get(n.0).is_some_and(|n| n.is_element()))
        .collect();
    assert_eq!(blocks.len(), 6, "the six blocks");
    let px = paint(&mut d);
    if let Ok(path) = std::env::var("RINCH_1100_PNG") {
        let img = image::RgbaImage::from_raw(VW as u32, VH as u32, px.clone()).unwrap();
        img.save(path).unwrap();
    }
    Page { doc: d, px, blocks }
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

/// What paint draws of IFC root `root`'s lines; asserts a line is cut.
fn painted(d: &RinchDocument, root: NodeId) -> String {
    let il = d.tree.get(root.0).unwrap().text_layout.as_ref().unwrap();
    il.painted_text()
        .unwrap_or_else(|| panic!("no line is cut: {:?}", il.text_content))
}

/// The x extents (inclusive) of the pixels in rows `y0..y1` for which `f`
/// holds, merged across gaps of at most 2px.
fn runs(px: &[u8], y0: usize, y1: usize, f: impl Fn(u8, u8, u8) -> bool) -> Vec<(usize, usize)> {
    let w = VW as usize;
    let mut cols: Vec<usize> = (y0..y1)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = &px[(y * w + x) * 4..];
            f(p[0], p[1], p[2])
        })
        .map(|(x, _)| x)
        .collect();
    cols.sort_unstable();
    cols.dedup();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for c in cols {
        match out.last_mut() {
            Some(last) if c <= last.1 + 2 => last.1 = c,
            _ => out.push((c, c)),
        }
    }
    out
}

fn pill_blue(r: u8, g: u8, b: u8) -> bool {
    b > 150 && r < 100 && (60..160).contains(&g)
}
fn pure_blue(r: u8, g: u8, b: u8) -> bool {
    b > 200 && r < 80 && g < 80
}
fn red(r: u8, g: u8, b: u8) -> bool {
    r > 200 && g < 90 && b < 90
}
fn green(r: u8, g: u8, b: u8) -> bool {
    g > 200 && r < 80 && b < 80
}
fn dark(r: u8, g: u8, b: u8) -> bool {
    r < 90 && g < 90 && b < 90
}

/// The `n` right-most runs of `rs`.
fn last(rs: &[(usize, usize)], n: usize) -> &[(usize, usize)] {
    &rs[rs.len().saturating_sub(n)..]
}

fn near(a: usize, b: usize, tol: usize) -> bool {
    a.abs_diff(b) <= tol
}

/// Pimble's repro: the pill keeps its background, cut at the box's edge, and
/// the "…" is drawn over it in the block's colour.
#[test]
fn a_pill_cut_by_the_ellipsis_keeps_its_background() {
    let p = page();
    let text = painted(&p.doc, p.blocks[0]);
    assert!(
        text.starts_with("Porto and the long road along the ") && text.ends_with('\u{2026}'),
        "{text:?}"
    );
    // Chrome 155: blue from 0 to 219 — the pill runs to the box's edge.
    let blue = runs(&p.px, 0, 40, pill_blue);
    assert_eq!(blue.len(), 1, "one pill: {blue:?}");
    assert!(
        blue[0].0 <= 1 && blue[0].1 == 219,
        "the pill runs to the box's edge: {blue:?}"
    );
    // The "…": three black dots over the pill, Chrome's at x 201–211.
    let dots = runs(&p.px, 0, 40, dark);
    assert_eq!(dots.len(), 3, "three dots and no other black: {dots:?}");
    assert!(
        near(dots[0].0, 201, 2) && near(dots[2].1, 211, 2),
        "the dots where Chrome draws them (201–211): {dots:?}"
    );
}

/// The issue's case: a wrapped paragraph keeps its inline styling on the
/// line that is not cut, and the line that overflows ends in a "…".
#[test]
fn a_wrapped_paragraph_keeps_its_styling_and_cuts_its_long_line() {
    let p = page();
    let text = painted(&p.doc, p.blocks[1]);
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), 2, "{text:?}");
    assert_eq!(lines[0].trim_end(), "ab cd");
    assert!(
        lines[1].starts_with("Supercali") && lines[1].ends_with('\u{2026}'),
        "{text:?}"
    );
    // `cd` is still red: Chrome's red at x 24–40 on line 1.
    let r = runs(&p.px, 40, 60, red);
    assert!(
        !r.is_empty() && near(r[0].0, 24, 2) && near(r.last().unwrap().1, 40, 2),
        "the red span on line 1: {r:?}"
    );
    // Line 2's ink ends at the "…", inside the 90px box (Chrome: x 82).
    let ink = runs(&p.px, 60, 80, dark);
    assert!(
        near(ink.last().unwrap().1, 82, 2),
        "line 2 ends at the \u{2026}: {ink:?}"
    );
}

/// A chip before the cut is drawn, and the "…" follows the text after it.
#[test]
fn a_chip_before_the_cut_is_drawn() {
    let p = page();
    let text = painted(&p.doc, p.blocks[2]);
    assert!(
        text.starts_with("ab ") && text.ends_with('\u{2026}'),
        "{text:?}"
    );
    let g = runs(&p.px, 120, 160, green);
    assert!(
        g.len() == 1 && near(g[0].0, 23, 1) && near(g[0].1, 52, 1),
        "the chip where Chrome draws it (23–52): {g:?}"
    );
    let ink = runs(&p.px, 120, 160, dark);
    assert!(
        near(ink.last().unwrap().1, 112, 2),
        "the \u{2026} ends where Chrome's does (112): {ink:?}"
    );
}

/// A chip that does not fit before the "…" is hidden whole.
#[test]
fn a_chip_past_the_cut_is_hidden() {
    let p = page();
    let text = painted(&p.doc, p.blocks[3]);
    assert_eq!(text, "ab \u{2026}");
    let g = runs(&p.px, 160, 200, green);
    assert!(g.is_empty(), "the chip is not drawn: {g:?}");
    let ink = runs(&p.px, 160, 200, dark);
    assert!(
        near(ink.last().unwrap().1, 34, 2),
        "the \u{2026} follows `ab ` (Chrome: 34): {ink:?}"
    );
}

/// The "…" is drawn in the block's font and colour, not the cut span's.
#[test]
fn the_ellipsis_takes_the_blocks_colour_and_size() {
    let p = page();
    painted(&p.doc, p.blocks[4]);
    let b = runs(&p.px, 200, 240, pure_blue);
    // `ab` and the three dots; Chrome's dots at 105–114.
    let dots = last(&b, 3);
    assert!(
        near(dots[0].0, 105, 2) && near(dots[2].1, 114, 2),
        "three blue dots where Chrome draws them (105–114): {b:?}"
    );
    // Small, as the 16px block's: no red past them.
    let r = runs(&p.px, 200, 240, red);
    assert!(
        r.last().unwrap().1 < dots[0].0,
        "the cut span's red ends before the \u{2026}: {r:?}"
    );
}

/// A block `text-shadow` is cast by the "…" too, and not past it.
#[test]
fn the_ellipsis_casts_the_blocks_shadow() {
    let p = page();
    painted(&p.doc, p.blocks[5]);
    let ink = runs(&p.px, 240, 280, dark);
    let shadow = runs(&p.px, 240, 280, red);
    let (i, s) = (ink.last().unwrap().1, shadow.last().unwrap().1);
    assert!(near(i, 108, 2), "Chrome's last dot at 108: {ink:?}");
    assert!(
        near(s, i, 1),
        "the shadow under the last dot: ink to {i}, shadow to {s}"
    );
}

/// A right-to-left paragraph's overflowing line leaves the box by its left
/// (parley aligns it against the box), so its "…" goes on the left and what
/// is kept is its logical start, on the right — as Chrome draws the
/// ellipsis of a `direction: rtl` block. (rinch takes a paragraph's direction
/// from its text, as parley infers it, and does not read `direction`, #1666.)
/// Structural: the bundled Inter has no Hebrew, so the face is the host's.
#[test]
fn a_right_to_left_line_is_cut_at_its_left() {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(
        body,
        r#"<div class="c" style="width: 120px; white-space: normal">אבג דהוזחטיכלמנסעפצקרשתאבגדהוזחטיכלמנסעפצקרשת</div>"#,
    );
    let div = d.get_children(body)[0];
    d.resolve_layout(VW, VH);
    let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    let cut = il.ellipsis.as_ref().expect("the long word's line is cut");
    let mark = cut.marks.last().unwrap();
    let line = il.layout.lines().nth(mark.line).unwrap();
    // Every kept cluster is right of the "…", and inside the box.
    let mut kept = Vec::new();
    for run in line.runs() {
        let mut x = run
            .visual_clusters()
            .next()
            .unwrap()
            .visual_offset()
            .unwrap();
        for c in run.visual_clusters() {
            let hidden = cut.hidden.iter().any(|r| r.contains(&c.text_range().start));
            if !hidden {
                kept.push((x, x + c.advance()));
            }
            x += c.advance();
        }
    }
    assert!(!kept.is_empty(), "something is kept");
    let end = mark.x + cut.glyph.width();
    assert!(
        mark.x >= -0.01,
        "the \u{2026} is inside the box: {}",
        mark.x
    );
    assert!(
        kept.iter()
            .all(|&(x0, x1)| x0 >= end - 0.01 && x1 <= 120.01),
        "kept clusters right of the \u{2026} ({end}) and inside the box: {kept:?}"
    );
}

/// Text beside a block child is laid out by an anonymous block box, whose
/// "…" is its block container's (#1071): drawn in the container's colour as
/// it is now, not the copy the box took when it was made (which a restyle of
/// the container does not refresh, #1450).
#[test]
fn an_anonymous_boxs_ellipsis_takes_its_containers_colour() {
    let mut d = doc();
    d.load_css(".r { color: rgb(255, 0, 0); } .b { color: rgb(0, 0, 255); }");
    let body = d.body();
    d.set_inner_html(
        body,
        r#"<div class="c r" style="width: 120px">ab cdefghijklmnopqrstuvw<div>x</div></div>"#,
    );
    let div = d.get_children(body)[0];
    paint(&mut d);
    d.set_attribute(div, "class", "c b");
    let px = paint(&mut d);
    let blue = runs(&px, 0, 20, pure_blue);
    let red = runs(&px, 0, 20, red);
    let last_blue = blue.last().map_or(0, |r| r.1);
    let last_red = red.last().map_or(0, |r| r.1);
    assert!(
        last_blue > last_red && near(last_blue, 108, 3),
        "the \u{2026} is the container's blue: blue {blue:?}, red {red:?}"
    );
}

// ── review of #1860 ─────────────────────────────────────────────────────────

/// The red text of `<div class=c>` boxes of each width drawn right of its
/// "…"'s left edge: `(width, red px)` for every width where any is.
fn red_past_the_ellipsis(
    family: &str,
    size: &str,
    widths: &[f32],
    text: &str,
) -> Vec<(f32, usize)> {
    let mut bad = Vec::new();
    let mut cuts = 0;
    for &w in widths {
        let mut d = doc();
        let body = d.body();
        d.set_inner_html(
            body,
            &format!(
                r#"<div class="c" style="width: {w}px; font-family: {family}; {size}"><span style="color: rgb(255, 0, 0)">{text}</span></div>"#
            ),
        );
        let div = d.get_children(body)[0];
        let px = paint(&mut d);
        let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
        let Some(cut) = il.ellipsis.as_ref() else {
            continue;
        };
        cuts += 1;
        let from = cut.marks[0].x.ceil() as usize + 1;
        let vw = VW as usize;
        let red = (0..40)
            .flat_map(|y| (from..vw).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = &px[(y * vw + x) * 4..];
                p[0] > 150 && p[1] < 140 && p[2] < 140
            })
            .count();
        if red > 0 {
            bad.push((w, red));
        }
    }
    assert!(
        cuts > widths.len() / 2,
        "positive control: the lines are cut ({cuts})"
    );
    bad
}

/// A ligature is kept or hidden whole with the cluster its glyph starts at:
/// paint draws the glyph with that cluster, so hiding only the clusters past
/// the cut drew the whole ligature under the "…" (Chrome 155 with DejaVu
/// Sans in a 92px box: `offi affi fi…`). DejaVu Sans (the CI host's face)
/// has the `fi`/`ffl` ligatures; the bundled Inter has none.
#[test]
fn a_ligature_is_not_drawn_under_the_ellipsis() {
    let text = "offi affi fiffl office affluent fifl";
    {
        // Positive control: the face makes ligatures here.
        let mut d = doc();
        let body = d.body();
        d.set_inner_html(
            body,
            &format!(
                r#"<div class="c" style="width: 92px; font-family: 'DejaVu Sans'">{text}</div>"#
            ),
        );
        let div = d.get_children(body)[0];
        d.resolve_layout(VW, VH);
        let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
        let ligs = il
            .layout
            .lines()
            .flat_map(|l| l.runs().collect::<Vec<_>>())
            .flat_map(|r| {
                r.visual_clusters()
                    .map(|c| c.is_ligature_continuation())
                    .collect::<Vec<_>>()
            })
            .filter(|&c| c)
            .count();
        assert!(
            ligs > 0,
            "DejaVu Sans makes no ligature here: is it installed?"
        );
    }
    let widths: Vec<f32> = (40..140).map(|w| w as f32).collect();
    let bad = red_past_the_ellipsis("'DejaVu Sans'", "", &widths, text);
    assert!(bad.is_empty(), "text drawn under the \u{2026}: {bad:?}");
}

/// A grapheme is kept or hidden whole: Inter draws `e` + U+0301 as one
/// glyph, which parley makes two clusters, so hiding the accent's cluster
/// alone drew `é` under the "…".
#[test]
fn a_grapheme_is_not_split_by_the_cut() {
    let e = "e\u{301}";
    let widths: Vec<f32> = (40..100).map(|w| w as f32).collect();
    let bad = red_past_the_ellipsis(
        "T1100",
        "font-size: 24px; line-height: 30px",
        &widths,
        &format!("caf{e} caf{e}s caf{e}"),
    );
    assert!(bad.is_empty(), "text drawn under the \u{2026}: {bad:?}");
}

/// Paint walks a cut line only up to its cut: the glyphs it collects do not
/// grow with the hidden text (review of #1860: a 40-line `pre` block of
/// 6,400-character lines took 12.4 ms a paint, against 2.3 ms when the cut
/// text was rebuilt short).
#[test]
fn a_paint_of_cut_lines_does_not_walk_the_hidden_text() {
    let glyphs = |len: usize| {
        let mut d = doc();
        let body = d.body();
        let line = "abcdefghij ".repeat(len / 11 + 1);
        let text = vec![line.as_str(); 40].join("\n");
        d.set_inner_html(
            body,
            &format!(r#"<div class="c" style="width: 200px; height: auto; white-space: pre">{text}</div>"#),
        );
        d.resolve_layout(VW, VH);
        rinch_dom::paint::take_text_glyphs_collected();
        paint(&mut d);
        rinch_dom::paint::take_text_glyphs_collected()
    };
    let (short, long) = (glyphs(400), glyphs(6400));
    assert!(short > 0, "positive control: paint collected glyphs");
    assert_eq!(
        short, long,
        "glyphs collected for 400- and 6400-character lines"
    );
}

/// No cut falls inside a grapheme, whatever clusters parley makes of it and
/// whether or not the face draws it as one glyph: stacked combining marks,
/// emoji ZWJ sequences, flags, Thai.
#[test]
fn no_cut_falls_inside_a_grapheme() {
    let texts = [
        "a\u{308}\u{323}b\u{308}\u{323}c\u{308}\u{323}d\u{308}\u{323}e\u{308}\u{323}f\u{308}\u{323}g\u{308}\u{323}h\u{308}\u{323}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "\u{1F1FA}\u{1F1F8}\u{1F1EC}\u{1F1E7}\u{1F1EB}\u{1F1F7}\u{1F1E9}\u{1F1EA}\u{1F1EF}\u{1F1F5}\u{1F1FA}\u{1F1F8}",
        "\u{0E01}\u{0E33}\u{0E19}\u{0E49}\u{0E33}\u{0E2A}\u{0E35}\u{0E48}\u{0E01}\u{0E33}\u{0E19}\u{0E49}\u{0E33}\u{0E2A}\u{0E35}\u{0E48}",
    ];
    let mut cuts = 0;
    let mut bad = Vec::new();
    for t in texts {
        for w in (20..120).step_by(3) {
            let mut d = doc();
            let body = d.body();
            d.set_inner_html(
                body,
                &format!(r#"<div class="c" style="width: {w}px">{t}</div>"#),
            );
            let div = d.get_children(body)[0];
            d.resolve_layout(VW, VH);
            let il = d.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
            let Some(cut) = il.ellipsis.as_ref() else {
                continue;
            };
            cuts += 1;
            for r in &cut.hidden {
                if !rinch_words::is_grapheme_boundary(&il.text_content, r.start) {
                    bad.push((w, r.clone()));
                }
            }
        }
    }
    assert!(cuts > 40, "positive control: the lines are cut ({cuts})");
    assert!(bad.is_empty(), "cuts inside a grapheme: {bad:?}");
}
