//! #829 — `visibility: hidden` hides a box's **content**, not just its box.
//!
//! `visibility` is inherited, and a `hidden` (or `collapse`) box draws nothing
//! of its own: no background or border, and none of its text, text
//! decorations, text shadows, inline backgrounds, SVG shapes or scrollbar
//! thumbs. A descendant that sets `visibility: visible` is drawn again. Desktop
//! paint gated only the box's own background/border/shadow on it, so a hidden
//! IFC root still drew every glyph it laid out — which is what made a closed
//! `Drawer`, `Popover` and `HoverCard` leave their text on screen.
//!
//! Every oracle here is **local**: a colour that only the hidden content can
//! produce, counted over the whole surface, so the correct answer is provably
//! `0`. Each carries a positive control — the same document with the content
//! shown inks that colour — so a zero is a hidden draw and not an empty page.
//! The mixed cases (a hidden run next to a visible one) compare against a twin
//! document in which the hidden text is `color: transparent` instead: same
//! layout, same glyph positions, so the two surfaces must be equal pixel for
//! pixel.
//!
//! Every text-bearing box declares `font-size` and `line-height`, so nothing
//! here is a pin on the local font set's line box.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 240.0;

/// Shared by every fixture: the text box every case reuses.
const CSS: &str = "
    .t { font-size: 20px; line-height: 24px; font-family: sans-serif; }
    .red { color: rgb(255, 0, 0); }
    .blue { color: rgb(0, 0, 255); }
    .hide { visibility: hidden; }
    .show { visibility: visible; }
    .collapse { visibility: collapse; }
";

fn doc_with(html: &str, extra_css: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    if !extra_css.is_empty() {
        doc.load_css(extra_css);
    }
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc
}

fn paint(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

fn count(px: &[[u8; 4]], pred: impl Fn([u8; 4]) -> bool) -> usize {
    px.iter().filter(|p| pred(**p)).count()
}

fn reddish(p: [u8; 4]) -> bool {
    p[3] > 0 && p[0] as i32 > p[1] as i32 + 80 && p[0] as i32 > p[2] as i32 + 80
}

fn blueish(p: [u8; 4]) -> bool {
    p[3] > 0 && p[2] as i32 > p[0] as i32 + 80 && p[2] as i32 > p[1] as i32 + 80
}

fn greenish(p: [u8; 4]) -> bool {
    p[3] > 0 && p[1] as i32 > p[0] as i32 + 60 && p[1] as i32 > p[2] as i32 + 60
}

fn any_ink(p: [u8; 4]) -> bool {
    p[3] > 0
}

fn red_of(html: &str, css: &str) -> usize {
    count(&paint(&mut doc_with(html, css)), reddish)
}

/// The id of the one element carrying `id="…"`.
fn by_id(doc: &RinchDocument, id: &str) -> NodeId {
    let found: Vec<usize> = doc
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.attributes.get("id").is_some_and(|v| v == id))
        .map(|(k, _)| k)
        .collect();
    assert_eq!(found.len(), 1, "one `#{id}`");
    NodeId(found[0])
}

// ── Text of the hidden box itself ────────────────────────────────────────────

/// The issue's own table, row 2: hidden from the first render. Broken: the
/// text paints exactly as the shown control does.
#[test]
fn hidden_from_the_first_render_paints_no_text() {
    let shown = red_of(r#"<div class="t red">hello world</div>"#, "");
    assert!(
        shown > 50,
        "positive control: the shown text inks red ({shown})"
    );
    assert_eq!(
        red_of(r#"<div class="t red hide">hello world</div>"#, ""),
        0,
        "a `visibility: hidden` IFC root draws none of its own text"
    );
}

/// `collapse` is `hidden` for everything that is not a table part.
#[test]
fn collapse_paints_no_text() {
    assert!(red_of(r#"<div class="t red">hello world</div>"#, "") > 50);
    assert_eq!(
        red_of(r#"<div class="t red collapse">hello world</div>"#, ""),
        0
    );
}

/// Hidden by an ancestor — the root of the text inherits it.
#[test]
fn hidden_through_an_ancestor_paints_no_text() {
    assert!(red_of(r#"<div><div class="t red">hello world</div></div>"#, "") > 50);
    assert_eq!(
        red_of(
            r#"<div class="hide"><div class="t red">hello world</div></div>"#,
            ""
        ),
        0
    );
}

/// Toggled after a shown frame: the class lands on an already-painted box.
/// Re-resolved at a different viewport so layout is not skipped.
#[test]
fn hidden_after_a_shown_frame_paints_no_text() {
    let mut doc = doc_with(r#"<div id="d" class="t red">hello world</div>"#, "");
    let before = count(&paint(&mut doc), reddish);
    assert!(before > 50, "positive control: shown first ({before})");
    let d = by_id(&doc, "d");
    doc.set_attribute(d, "class", "t red hide");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(
        count(&paint(&mut doc), reddish),
        0,
        "hidden after a shown frame"
    );
    // And back: the text returns, so the gate reads the live value.
    doc.set_attribute(d, "class", "t red");
    doc.resolve_layout(VW, VH);
    assert!(count(&paint(&mut doc), reddish) > 50, "shown again");
}

/// Through an inline style write, the issue table's `set_style` row.
#[test]
fn hidden_through_set_style_paints_no_text() {
    let mut doc = doc_with(r#"<div id="d" class="t red">hello world</div>"#, "");
    assert!(count(&paint(&mut doc), reddish) > 50);
    let d = by_id(&doc, "d");
    doc.set_style(d, "visibility", "hidden");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(count(&paint(&mut doc), reddish), 0);
}

// ── Mixed runs: per-element visibility inside one inline formatting context ──

/// A hidden root with a `visibility: visible` span: only the span's text is
/// drawn. Its twin writes the root's text `transparent` instead of hidden, so
/// the glyph positions are identical and the surfaces must be equal.
#[test]
fn a_visible_span_under_a_hidden_root_is_drawn_alone() {
    let hidden = paint(&mut doc_with(
        r#"<div class="t red hide">hidden <span class="show">SHOWN</span> tail</div>"#,
        "",
    ));
    let twin = paint(&mut doc_with(
        r#"<div class="t" style="color: transparent">hidden <span class="red">SHOWN</span> tail</div>"#,
        "",
    ));
    assert!(
        count(&twin, reddish) > 50,
        "positive control: the span inks red in the twin"
    );
    assert!(
        count(&hidden, reddish) > 50,
        "the visible span is drawn under its hidden root"
    );
    assert_eq!(
        count(&hidden, reddish),
        count(&twin, reddish),
        "exactly the span is drawn — no more of the hidden root's text"
    );
    assert!(hidden == twin, "pixel-identical to the transparent twin");
}

/// The other direction: a hidden span inside a shown root. The root's text
/// is blue, the span's red.
#[test]
fn a_hidden_span_inside_a_shown_root_is_not_drawn() {
    let shown = paint(&mut doc_with(
        r#"<div class="t blue">before <span class="red">SPAN</span> after</div>"#,
        "",
    ));
    assert!(
        count(&shown, reddish) > 50,
        "positive control: the span inks red"
    );
    let hidden = paint(&mut doc_with(
        r#"<div class="t blue">before <span class="red hide">SPAN</span> after</div>"#,
        "",
    ));
    assert_eq!(count(&hidden, reddish), 0, "the hidden span draws no text");
    assert_eq!(
        count(&hidden, blueish),
        count(&shown, blueish),
        "the root's own text is untouched"
    );
}

/// Text that shares one Parley glyph run with hidden text. A `display: inline`
/// span always opens a style span of its own, which Parley numbers as a new
/// style and so a new glyph run, whatever its values — but a `display:
/// contents` wrapper whose text style matches its parent pushes none (#574),
/// and `visibility` is not a text style. So the wrapper's text and its
/// neighbours are one glyph run and the hidden stretch has to be cut out of it
/// glyph by glyph. A gate per glyph run draws all of it or none of it.
#[test]
fn a_hidden_span_sharing_a_glyph_run_is_cut_out_of_it() {
    let hidden = paint(&mut doc_with(
        r#"<div class="t red">before <span class="hide" style="display: contents">SPAN</span> after</div>"#,
        "",
    ));
    let twin = paint(&mut doc_with(
        r#"<div class="t red">before <span style="color: transparent">SPAN</span> after</div>"#,
        "",
    ));
    assert!(count(&twin, reddish) > 50);
    assert!(hidden == twin, "only the span's glyphs are dropped");
}

// ── What else an IFC draws ───────────────────────────────────────────────────

/// Underline and line-through are drawn with the text, and hidden with it.
#[test]
fn hidden_text_draws_no_decoration() {
    let css = ".u { text-decoration: underline line-through; }";
    assert!(red_of(r#"<div class="t red u">hello world</div>"#, css) > 50);
    assert_eq!(
        red_of(r#"<div class="t red u hide">hello world</div>"#, css),
        0
    );
    // A decoration on a hidden span inside a shown root: the root's text is
    // blue, so anything red is the span's glyphs or its lines.
    assert_eq!(
        red_of(
            r#"<div class="t blue">a <span class="red u hide">SPAN</span> b</div>"#,
            css
        ),
        0
    );
}

/// `text-shadow` is drawn from the IFC root's style over the whole layout; a
/// hidden root draws none, and a hidden span casts none under a shown root.
#[test]
fn hidden_text_casts_no_shadow() {
    let css = ".s { text-shadow: 3px 3px rgb(0, 160, 0); }";
    let green = |html: &str| count(&paint(&mut doc_with(html, css)), greenish);
    assert!(green(r#"<div class="t red s">hello world</div>"#) > 50);
    assert_eq!(green(r#"<div class="t red s hide">hello world</div>"#), 0);
    let whole = green(r#"<div class="t red s">abc <span>SPAN</span> def</div>"#);
    let cut = green(r#"<div class="t red s">abc <span class="hide">SPAN</span> def</div>"#);
    let twin =
        green(r#"<div class="t red s">abc <span style="visibility: hidden">SPAN</span> def</div>"#);
    assert!(
        cut < whole,
        "the hidden span's shadow is gone ({cut} vs {whole})"
    );
    assert_eq!(cut, twin);
    assert!(cut > 20, "the shown text's shadow stays ({cut})");
}

/// An inline element's background is a span in its IFC, not a box of its own.
#[test]
fn a_hidden_inline_background_is_not_drawn() {
    let css = ".bg { background: rgb(0, 200, 0); padding: 0 4px; }";
    let green = |html: &str| count(&paint(&mut doc_with(html, css)), greenish);
    assert!(green(r#"<div class="t blue">a <span class="bg">SPAN</span> b</div>"#) > 100);
    assert_eq!(
        green(r#"<div class="t blue">a <span class="bg hide">SPAN</span> b</div>"#),
        0
    );
    assert_eq!(
        green(r#"<div class="t blue hide">a <span class="bg">SPAN</span> b</div>"#),
        0,
        "hidden through the root, which the span inherits"
    );
}

/// The read-only text selection highlight belongs to the text it highlights.
#[test]
fn a_hidden_roots_selection_highlight_is_not_drawn() {
    let ink = |class: &str| {
        let mut doc = doc_with(
            &format!(
                r#"<div id="d" class="t {class}" style="color: transparent">hello world</div>"#
            ),
            "",
        );
        let d = by_id(&doc, "d");
        for (k, v) in [
            ("data-text-sel", "true"),
            ("data-text-sel-start", "0"),
            ("data-text-sel-end", "5"),
        ] {
            doc.set_attribute(d, k, v);
        }
        doc.resolve_layout(VW + 1.0, VH);
        count(&paint(&mut doc), any_ink)
    };
    assert!(ink("") > 50, "positive control: the highlight paints");
    assert_eq!(ink("hide"), 0);
}

// ── SVG ─────────────────────────────────────────────────────────────────────

#[test]
fn a_hidden_svgs_shapes_are_not_drawn_and_a_visible_child_is() {
    let svg = |class: &str, child: &str| {
        format!(
            r#"<svg class="{class}" viewBox="0 0 24 24" style="width: 48px; height: 48px">
                 <rect class="{child}" x="0" y="0" width="24" height="24" fill="rgb(255,0,0)"></rect>
               </svg>"#
        )
    };
    assert!(red_of(&svg("", ""), "") > 500, "positive control");
    assert_eq!(red_of(&svg("hide", ""), ""), 0);
    assert!(
        red_of(&svg("hide", "show"), "") > 500,
        "a `visibility: visible` shape inside a hidden <svg> is drawn"
    );
}

// ── Scrollbars ──────────────────────────────────────────────────────────────

/// A hidden scroll container draws no thumb. Its content is `transparent`, so
/// the thumb is the only ink the shown control can produce.
#[test]
fn a_hidden_scroll_container_draws_no_thumb() {
    let html = |class: &str| {
        format!(
            r#"<div class="{class}" style="width: 100px; height: 100px; overflow: auto">
                 <div style="height: 400px"></div>
               </div>"#
        )
    };
    let shown = count(&paint(&mut doc_with(&html(""), "")), any_ink);
    assert!(shown > 50, "positive control: the thumb inks ({shown})");
    assert_eq!(count(&paint(&mut doc_with(&html("hide"), "")), any_ink), 0);
}

// ── `filter` ────────────────────────────────────────────────────────────────

/// The `brightness` approximation is an overlay over the box, so it would
/// darken the page where a hidden box sits.
#[test]
fn a_hidden_filtered_box_draws_no_overlay() {
    let html = |class: &str| {
        format!(
            r#"<div class="{class}" style="width: 100px; height: 50px; filter: brightness(0.5)"></div>"#
        )
    };
    assert!(count(&paint(&mut doc_with(&html(""), "")), any_ink) > 100);
    assert_eq!(count(&paint(&mut doc_with(&html("hide"), "")), any_ink), 0);
}

/// A decoration drawn across a glyph run that is only partly hidden stops at
/// the hidden glyphs, and resumes after them.
///
/// **Measured in Chrome 153**, since a decoration propagated from an ancestor
/// is otherwise drawn across a descendant's text in the ancestor's colour: for
/// `before <span style="visibility: hidden">SPAN</span> after` under
/// `text-decoration: underline line-through`, both lines leave a gap of about
/// 53px — the span's width — whether the span is `display: inline` or
/// `display: contents`. So a `color: transparent` twin is **not** the oracle
/// here (its span keeps the ancestor's red lines); the gap is.
///
/// The hidden stretch is a `display: contents` wrapper, so it shares its
/// neighbours' glyph run (see
/// `a_hidden_span_sharing_a_glyph_run_is_cut_out_of_it`), and the lines are
/// drawn once per run: only the shown stretches may keep them. The wrapper
/// declares the decoration itself because `text-decoration-line` does not
/// inherit: without it the wrapper's text style differs from its parent's, it
/// opens a style span, and its text is a glyph run of its own — skipped whole,
/// which never reaches the per-glyph cut this fixture is about.
#[test]
fn a_decoration_is_cut_where_its_run_is_hidden() {
    let css = ".u { text-decoration: underline line-through; }";
    let rows = |px: &[[u8; 4]]| -> Vec<Vec<usize>> {
        (0..VH as usize)
            .map(|y| {
                (0..VW as usize)
                    .filter(|&x| reddish(px[y * VW as usize + x]))
                    .collect()
            })
            .collect()
    };
    // The widest gap between two red pixels of one row.
    let widest_gap = |xs: &[usize]| xs.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
    // A decoration row is one the unhidden document inks red, unbroken, for
    // longer than any glyph could.
    let whole = rows(&paint(&mut doc_with(
        r#"<div class="t red u">before <span class="u" style="display: contents">SPAN</span> after</div>"#,
        css,
    )));
    let deco_rows: Vec<usize> = (0..whole.len())
        .filter(|&y| whole[y].len() > 120 && widest_gap(&whole[y]) <= 2)
        .collect();
    assert!(
        deco_rows.len() >= 2,
        "positive control: the underline and the line-through each ink an \
         unbroken red row when nothing is hidden ({deco_rows:?})"
    );
    let hidden = rows(&paint(&mut doc_with(
        r#"<div class="t red u">before <span class="u hide" style="display: contents">SPAN</span> after</div>"#,
        css,
    )));
    for y in deco_rows {
        let gap = widest_gap(&hidden[y]);
        assert!(
            gap > 25,
            "row {y}: the lines stop under the hidden span and resume after it \
             (widest gap {gap}px)"
        );
    }
}

/// A font change starts a new Parley run inside the line, and the glyph
/// numbering starts again with it. A hidden span after one has to be found by
/// its own run's count, not the line's.
#[test]
fn a_hidden_span_after_a_font_change_is_cut_out_of_its_own_run() {
    let hidden = paint(&mut doc_with(
        r#"<div class="t red">ab <b>BOLD</b> cd <span class="hide">SPAN</span> ef</div>"#,
        "",
    ));
    let twin = paint(&mut doc_with(
        r#"<div class="t red">ab <b>BOLD</b> cd <span style="color: transparent">SPAN</span> ef</div>"#,
        "",
    ));
    assert!(count(&twin, reddish) > 50);
    assert!(hidden == twin, "only the span's glyphs are dropped");
}

/// `text-overflow: ellipsis` rebuilds the root's layout from its flattened
/// text, with no per-node ranges — so the root's own visibility is what
/// answers for all of it.
#[test]
fn a_hidden_ellipsis_layout_paints_no_text() {
    let css = ".e { width: 80px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }";
    let text = "a long line of text that is cut";
    assert!(red_of(&format!(r#"<div class="t red e">{text}</div>"#), css) > 50);
    assert_eq!(
        red_of(&format!(r#"<div class="t red e hide">{text}</div>"#), css),
        0
    );
}

// ── RenderSurface ───────────────────────────────────────────────────────────

/// A `RenderSurface`'s pixels are the box's own content, like an `<img>`'s,
/// and a hidden one draws none of them. The surface-with-pixels branch used
/// to draw its background and frame before any visibility check; only the
/// no-pixels fallback was gated. (The review of PR #844's P7.)
#[test]
fn a_hidden_render_surface_draws_no_pixels() {
    let red = |class: &str| {
        let mut doc = doc_with(
            &format!(
                r#"<div class="{class}" data-render-surface="7" style="width: 50px; height: 50px; background: rgb(200, 0, 0)"></div>"#
            ),
            "",
        );
        let mut map = std::collections::HashMap::new();
        map.insert(
            7usize,
            rinch_dom::paint::SurfacePixelData {
                data: [255u8, 0, 0, 255].repeat(10 * 10),
                width: 10,
                height: 10,
            },
        );
        rinch_dom::paint::set_surface_pixels(Some(map));
        let px = paint(&mut doc);
        rinch_dom::paint::set_surface_pixels(None);
        count(&px, reddish)
    };
    assert!(
        red("") >= 2500,
        "positive control: the shown surface draws its frame"
    );
    assert_eq!(
        red("hide"),
        0,
        "a hidden surface draws neither frame nor background"
    );
}

// ── Wavy underline (#847) ───────────────────────────────────────────────────

/// A wavy underline is not a Parley decoration: `paint_wavy_decorations`
/// draws it from a byte range after the text. It follows the text's
/// visibility glyph by glyph, like the straight underline — gone under a
/// hidden span, still drawn under the shown text around it.
#[test]
fn a_wavy_underline_is_cut_where_its_text_is_hidden() {
    // A hidden span's own squiggle: nothing red may be drawn at all.
    let own = |class: &str| {
        red_of(
            &format!(
                r#"<div class="t blue">before <span class="{class}" style="text-decoration: underline wavy rgb(255, 0, 0)">SPAN</span> after</div>"#
            ),
            "",
        )
    };
    assert!(own("") > 20, "positive control: the span's wave inks red");
    assert_eq!(own("hide"), 0, "a hidden span draws no wave");

    // The root's squiggle across a hidden span: cut under the span only. The
    // span's columns come from a twin that paints its text green.
    let root = "t blue\" style=\"text-decoration: underline wavy rgb(255, 0, 0)";
    let cols = |px: &[[u8; 4]], pred: fn([u8; 4]) -> bool| -> Vec<bool> {
        (0..VW as usize)
            .map(|x| (0..VH as usize).any(|y| pred(px[y * VW as usize + x])))
            .collect()
    };
    let green =
        |p: [u8; 4]| p[3] > 0 && p[1] as i32 > p[0] as i32 + 80 && p[1] as i32 > p[2] as i32 + 80;
    let twin = paint(&mut doc_with(
        &format!(
            r#"<div class="{root}">before <span style="color: rgb(0, 255, 0)">SPAN</span> after</div>"#
        ),
        "",
    ));
    let span_cols = cols(&twin, green);
    let x0 = span_cols
        .iter()
        .position(|&c| c)
        .expect("the span inks green");
    let x1 = span_cols.iter().rposition(|&c| c).unwrap();
    let hidden = paint(&mut doc_with(
        &format!(r#"<div class="{root}">before <span class="hide">SPAN</span> after</div>"#),
        "",
    ));
    let red_cols = cols(&hidden, reddish);
    assert!(
        red_cols[..x0].iter().filter(|&&c| c).count() > 20,
        "the wave under the shown text before the span stays"
    );
    assert!(
        red_cols[x1 + 1..].iter().filter(|&&c| c).count() > 20,
        "and after it"
    );
    let under = red_cols[x0 + 3..x1.saturating_sub(2)]
        .iter()
        .filter(|&&c| c)
        .count();
    assert_eq!(under, 0, "no wave under the hidden span ({x0}..{x1})");
}
