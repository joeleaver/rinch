//! Issue #679 — a transition or animation tick writes `computed_style` with no
//! cascade, and an inline formatting context's Parley layout carries the
//! colour of every run as a baked brush. So text whose `color` is being
//! interpolated has to be drawn in the colour its element computes **now**,
//! not the one it was shaped with.
//!
//! Every fixture compares against something that is not derived from the
//! thing under test: either a **twin document** built directly in the end
//! state (pixel for pixel), or — mid-run, where no twin can be built because
//! the tick reads the wall clock — the exact colour the tick wrote into
//! `computed_style`, which a fully covered glyph pixel must then hold.
//!
//! The mid-run fixtures are the ones off the fixed point: at the end of a
//! transition the animated value equals the cascaded one, so anything that
//! re-reads the style once at the end passes there and only there.
//!
//! All text is the bundled Inter at a declared size and line height.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

const VW: f32 = 600.0;
const VH: f32 = 300.0;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Heavy glyphs, so every letter has fully covered interior pixels.
const BASE_CSS: &str = "body { font-family: T679; font-size: 48px; line-height: 60px; \
                        color: rgb(0, 0, 0); margin: 0; }";

type Rgb = (u8, u8, u8);
const RED: Rgb = (200, 10, 10);
const BLUE: Rgb = (10, 10, 200);
const GREEN: Rgb = (10, 160, 10);

fn doc_with(css: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("T679"),
            ..Default::default()
        }),
    );
    doc.load_css(BASE_CSS);
    doc.load_css(css);
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(e, "class", class);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn settle(doc: &mut RinchDocument) {
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
}

fn paint(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

/// Fully opaque pixels of exactly this colour.
fn exact(px: &[u8], rgb: Rgb) -> u32 {
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255 && (p[0], p[1], p[2]) == rgb)
        .count() as u32
}

/// Fully opaque pixels of exactly this colour in the rows `y0..y1`.
fn exact_rows(px: &[u8], rgb: Rgb, y0: usize, y1: usize) -> u32 {
    let w = VW as usize;
    exact(&px[y0 * w * 4..y1 * w * 4], rgb)
}

fn rgb_of(c: peniko::Color) -> Rgb {
    let c = c.to_rgba8();
    (c.r, c.g, c.b)
}

fn colour_of(doc: &RinchDocument, id: NodeId) -> Rgb {
    rgb_of(
        doc.tree
            .get(id.0)
            .unwrap()
            .computed_style
            .color
            .expect("a cascaded element has a colour"),
    )
}

/// Move every transition running on `id` so that it started `ago_ms` ago.
fn age_transitions(doc: &mut RinchDocument, id: NodeId, ago_ms: f64) {
    let now = now_ms();
    for t in doc
        .tree
        .active_transitions
        .get_mut(&id.0)
        .expect("the class change should have started a transition")
        .values_mut()
    {
        t.start_time_ms = now - ago_ms;
    }
}

/// Move every animation running on `id` so that it started `ago_ms` ago.
fn age_animations(doc: &mut RinchDocument, id: NodeId, ago_ms: f64) {
    let now = now_ms();
    for a in doc
        .tree
        .active_animations
        .get_mut(&id.0)
        .expect("the element should be animating")
    {
        a.start_time_ms = now - ago_ms;
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0
}

/// One frame the way a host runs it: tick, lay out, paint — with the frame's
/// counters, which is how a fixture says the tick shaped nothing.
fn frame(doc: &mut RinchDocument) -> (Vec<u8>, rinch_dom::perf::FrameStats) {
    doc.tree.perf.reset();
    doc.tick_transitions();
    doc.tick_animations();
    doc.resolve_layout(VW, VH);
    let px = paint(doc);
    (px, doc.tree.perf.end_frame())
}

fn shapes(s: &rinch_dom::perf::FrameStats) -> u64 {
    [
        Counter::ShapeMeasureIfc,
        Counter::ShapeMeasureText,
        Counter::ShapeIfcBuild,
        Counter::ShapeAtomicInline,
        Counter::EllipsisShapes,
        Counter::ShapePaint,
    ]
    .iter()
    .map(|&c| s.get(c))
    .sum()
}

const LONG: f64 = 100_000.0;
/// A 100 s transition aged 40 s: far from both ends, and a test that takes a
/// second longer to reach its tick is still far from both ends.
const MID: f64 = 40_000.0;

const TRANSITION_CSS: &str = ".t { transition: color 100s linear, background-color 100s linear, \
                              font-size 100s linear; }";

/// A fixture document: `build` makes the tree and answers the element whose
/// class flips from `from` to `to`.
struct Case {
    css: &'static str,
    build: fn(&mut RinchDocument, &str) -> NodeId,
    from: &'static str,
    to: &'static str,
}

impl Case {
    /// The document laid out and painted in `from`, then switched to `to`.
    fn started(&self) -> (RinchDocument, NodeId) {
        let mut doc = doc_with(&format!("{TRANSITION_CSS} {}", self.css));
        let target = (self.build)(&mut doc, self.from);
        settle(&mut doc);
        paint(&mut doc);
        doc.set_attribute(target, "class", self.to);
        doc.resolve_layout(VW, VH);
        (doc, target)
    }

    /// The pixels of the same tree built directly in `to`, no transition run.
    fn twin(&self) -> Vec<u8> {
        let mut doc = doc_with(self.css);
        (self.build)(&mut doc, self.to);
        settle(&mut doc);
        paint(&mut doc)
    }

    /// The transition driven past its end, against the twin.
    fn finished_matches_the_twin(&self) {
        let (mut doc, target) = self.started();
        age_transitions(&mut doc, target, LONG * 2.0);
        let (got, _) = frame(&mut doc);
        let want = self.twin();
        let differing = got
            .as_chunks::<4>()
            .0
            .iter()
            .zip(want.as_chunks::<4>().0)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            differing, 0,
            "after the transition ends the frame should be the one a document \
             built in the end state paints; {differing} px differ"
        );
    }
}

// ── colour on an IFC root's own text ────────────────────────────────────────

fn root_text(doc: &mut RinchDocument, class: &str) -> NodeId {
    let body = doc.body();
    let p = el(doc, body, "div", class);
    text(doc, p, "MMMM HHHH");
    p
}

const ROOT_COLOUR: Case = Case {
    css: ".a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t a",
    to: "t b",
};

#[test]
fn a_finished_colour_transition_on_block_text_is_drawn_in_the_end_colour() {
    ROOT_COLOUR.finished_matches_the_twin();
}

#[test]
fn a_running_colour_transition_on_block_text_is_drawn_in_the_frames_colour() {
    let (mut doc, p) = ROOT_COLOUR.started();
    age_transitions(&mut doc, p, MID);
    let (px, stats) = frame(&mut doc);
    let now = colour_of(&doc, p);
    assert!(
        now != RED && now != BLUE,
        "the fixture must sample between the ends, got {now:?}"
    );
    assert!(
        exact(&px, now) > 500,
        "glyph interiors should hold the colour this frame's tick wrote \
         ({now:?}): {} px",
        exact(&px, now)
    );
    assert_eq!(exact(&px, RED), 0, "no glyph keeps the start colour");
    assert_eq!(shapes(&stats), 0, "a colour frame shapes nothing");
    assert_eq!(
        stats.get(Counter::TaffyRootComputes),
        0,
        "a colour frame runs no Taffy compute"
    );
}

// ── colour on an inline element inside a paragraph ──────────────────────────

fn span_in_paragraph(doc: &mut RinchDocument, class: &str) -> NodeId {
    let body = doc.body();
    let p = el(doc, body, "div", "para");
    text(doc, p, "MMM ");
    let s = el(doc, p, "span", class);
    text(doc, s, "HHH");
    text(doc, p, " MMM");
    s
}

const SPAN_COLOUR: Case = Case {
    css: ".para { color: rgb(10, 160, 10); } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: span_in_paragraph,
    from: "t a",
    to: "t b",
};

#[test]
fn a_finished_colour_transition_on_an_inline_span_is_drawn_in_the_end_colour() {
    SPAN_COLOUR.finished_matches_the_twin();
}

#[test]
fn a_running_colour_transition_on_an_inline_span_recolours_only_the_span() {
    let (mut doc, s) = SPAN_COLOUR.started();
    // The paragraph's own text, before anything moves.
    let green_before = exact(&paint(&mut doc), GREEN);
    assert!(green_before > 500, "positive control: {green_before} green px");
    age_transitions(&mut doc, s, MID);
    let (px, stats) = frame(&mut doc);
    let now = colour_of(&doc, s);
    assert!(now != RED && now != BLUE, "between the ends, got {now:?}");
    assert!(exact(&px, now) > 200, "the span: {} px", exact(&px, now));
    assert_eq!(exact(&px, RED), 0, "the span keeps none of its start colour");
    assert_eq!(
        exact(&px, GREEN),
        green_before,
        "the text around the span keeps the paragraph's colour"
    );
    assert_eq!(shapes(&stats), 0, "a colour frame shapes nothing");
}

/// A `display: contents` wrapper whose text style is its parent's pushes no
/// span of its own, so its text and the text before it are one Parley glyph
/// run — and stay one when the wrapper's colour starts to move, since the
/// re-shape at the start of the transition still sees two equal styles. The
/// split between the two colours then falls inside a run.
fn wrapper_sharing_a_run(doc: &mut RinchDocument, class: &str) -> NodeId {
    let body = doc.body();
    let p = el(doc, body, "div", "a");
    text(doc, p, "MMMM");
    let w = el(doc, p, "span", class);
    doc.set_attribute(w, "style", "display: contents");
    text(doc, w, "HHHH");
    w
}

const SHARED_RUN: Case = Case {
    css: ".a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: wrapper_sharing_a_run,
    from: "t a",
    to: "t b",
};

#[test]
fn text_sharing_a_glyph_run_with_still_text_is_recoloured_alone() {
    let (mut doc, w) = SHARED_RUN.started();
    let red_before = exact(&paint(&mut doc), RED);
    age_transitions(&mut doc, w, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, w);
    let (moved, still) = (exact(&px, now), exact(&px, RED));
    assert!(moved > 200, "the animated text: {moved} px of {now:?}");
    assert!(
        still > 200 && still < red_before,
        "the text before it keeps red and the animated text gives it up: \
         {still} red px now, {red_before} before"
    );
    SHARED_RUN.finished_matches_the_twin();
}

// ── decorations ─────────────────────────────────────────────────────────────

/// The rows of the first line that an underline can be in and no Inter
/// capital reaches: below the baseline of a 48px face in a 60px line.
const UNDER: (usize, usize) = (47, 60);

const UNDERLINE: Case = Case {
    css: ".u { text-decoration: underline; } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t u a",
    to: "t u b",
};

#[test]
fn a_currentcolor_underline_follows_a_running_colour_transition() {
    let (mut doc, p) = UNDERLINE.started();
    let red_line = exact_rows(&paint(&mut doc), RED, UNDER.0, UNDER.1);
    assert!(red_line > 100, "positive control: the underline, {red_line} px");
    age_transitions(&mut doc, p, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, p);
    assert!(
        exact_rows(&px, now, UNDER.0, UNDER.1) > 100,
        "the underline is drawn in the text's colour this frame"
    );
    assert_eq!(exact_rows(&px, RED, UNDER.0, UNDER.1), 0);
    UNDERLINE.finished_matches_the_twin();
}

const FIXED_UNDERLINE: Case = Case {
    css: ".u { text-decoration: underline; text-decoration-color: rgb(10, 160, 10); } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t u a",
    to: "t u b",
};

#[test]
fn a_declared_decoration_colour_stays_through_a_colour_transition() {
    let (mut doc, p) = FIXED_UNDERLINE.started();
    let green_line = exact_rows(&paint(&mut doc), GREEN, UNDER.0, UNDER.1);
    assert!(green_line > 100, "positive control: {green_line} px");
    age_transitions(&mut doc, p, MID);
    let (px, _) = frame(&mut doc);
    assert_eq!(
        exact_rows(&px, GREEN, UNDER.0, UNDER.1),
        green_line,
        "`text-decoration-color` is not `color`"
    );
    assert!(exact(&px, colour_of(&doc, p)) > 500, "the glyphs did move");
    FIXED_UNDERLINE.finished_matches_the_twin();
}

/// An explicit decoration colour that **equals** the text's start colour: a
/// rule that told the two apart by comparing baked brushes would move it.
const SAME_AS_START_UNDERLINE: Case = Case {
    css: ".u { text-decoration: underline; text-decoration-color: rgb(200, 10, 10); } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t u a",
    to: "t u b",
};

#[test]
fn a_declared_decoration_colour_equal_to_the_start_colour_stays_too() {
    let (mut doc, p) = SAME_AS_START_UNDERLINE.started();
    let line = exact_rows(&paint(&mut doc), RED, UNDER.0, UNDER.1);
    assert!(line > 100, "positive control: {line} px");
    age_transitions(&mut doc, p, MID);
    let (px, _) = frame(&mut doc);
    assert_eq!(exact_rows(&px, RED, UNDER.0, UNDER.1), line);
    SAME_AS_START_UNDERLINE.finished_matches_the_twin();
}

/// A `text-decoration-color` declared **above** the IFC root is not the
/// underline's: the property is not inherited, and the layout is built from
/// the root down.
const DECLARED_ABOVE: Case = Case {
    css: "body { text-decoration-color: rgb(10, 160, 10); } \
          .u { text-decoration: underline; } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t u a",
    to: "t u b",
};

#[test]
fn a_decoration_colour_declared_above_the_root_does_not_hold_the_underline() {
    let (mut doc, p) = DECLARED_ABOVE.started();
    let red_line = exact_rows(&paint(&mut doc), RED, UNDER.0, UNDER.1);
    assert!(red_line > 100, "positive control: {red_line} px");
    age_transitions(&mut doc, p, MID);
    let (px, _) = frame(&mut doc);
    assert_eq!(exact_rows(&px, RED, UNDER.0, UNDER.1), 0);
    assert!(exact_rows(&px, colour_of(&doc, p), UNDER.0, UNDER.1) > 100);
}

const WAVY: Case = Case {
    css: ".u { text-decoration: underline wavy; } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: span_in_paragraph,
    from: "t u a",
    to: "t u b",
};

/// Pixels in rows `y0..y1` that are mostly red: the start colour at three
/// quarters coverage or more, and nothing a colour part-way to blue reaches.
fn reddish_rows(px: &[u8], y0: usize, y1: usize) -> u32 {
    let w = VW as usize;
    px[y0 * w * 4..y1 * w * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 150 && p[2] < 60)
        .count() as u32
}

#[test]
fn a_wavy_underline_follows_a_colour_transition() {
    // Below every glyph of the line: only the wave is here.
    const WAVE: (usize, usize) = (50, 60);
    let (mut doc, s) = WAVY.started();
    let before = reddish_rows(&paint(&mut doc), WAVE.0, WAVE.1);
    assert!(before > 20, "positive control: the red wave, {before} px");
    age_transitions(&mut doc, s, MID);
    let (px, _) = frame(&mut doc);
    assert_eq!(
        reddish_rows(&px, WAVE.0, WAVE.1),
        0,
        "the wave left the start colour with the text"
    );
    WAVY.finished_matches_the_twin();
}

// ── the other shapes an IFC's text arrives in ───────────────────────────────

/// Text beside a block child is laid out by an anonymous block box, which no
/// ancestor walk from the element finds.
fn text_beside_a_block(doc: &mut RinchDocument, class: &str) -> NodeId {
    let body = doc.body();
    let d = el(doc, body, "div", class);
    text(doc, d, "MMMM");
    let inner = el(doc, d, "div", "still");
    text(doc, inner, "HHHH");
    d
}

const ANON_BOX: Case = Case {
    css: ".still { color: rgb(10, 160, 10); } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: text_beside_a_block,
    from: "t a",
    to: "t b",
};

#[test]
fn text_in_an_anonymous_block_box_follows_its_containers_colour_transition() {
    let (mut doc, d) = ANON_BOX.started();
    age_transitions(&mut doc, d, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, d);
    assert!(exact(&px, now) > 300, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
    ANON_BOX.finished_matches_the_twin();
}

const ELLIPSIS: Case = Case {
    css: ".e { width: 200px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t e a",
    to: "t e b",
};

#[test]
fn an_ellipsis_line_follows_a_colour_transition() {
    let (mut doc, p) = ELLIPSIS.started();
    age_transitions(&mut doc, p, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, p);
    assert!(exact(&px, now) > 200, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
    ELLIPSIS.finished_matches_the_twin();
}

fn inline_block_label(doc: &mut RinchDocument, class: &str) -> NodeId {
    let body = doc.body();
    let p = el(doc, body, "div", "");
    let chip = el(doc, p, "span", class);
    text(doc, chip, "MMMM");
    chip
}

const INLINE_BLOCK: Case = Case {
    css: ".ib { display: inline-block; } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: inline_block_label,
    from: "t ib a",
    to: "t ib b",
};

#[test]
fn an_inline_blocks_own_text_follows_its_colour_transition() {
    let (mut doc, chip) = INLINE_BLOCK.started();
    age_transitions(&mut doc, chip, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, chip);
    assert!(exact(&px, now) > 300, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
    INLINE_BLOCK.finished_matches_the_twin();
}

/// Positive control for the harness, green before this issue: a flex item's
/// own text is a leaf, which paint has coloured from the live style since
/// #904.
const LEAF: Case = Case {
    css: ".f { display: flex; } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: root_text,
    from: "t f a",
    to: "t f b",
};

#[test]
fn a_text_leaf_follows_a_colour_transition_as_it_did() {
    let (mut doc, p) = LEAF.started();
    age_transitions(&mut doc, p, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, p);
    assert!(exact(&px, now) > 500);
    assert_eq!(exact(&px, RED), 0);
    LEAF.finished_matches_the_twin();
}

// ── an inline element's background ──────────────────────────────────────────

const SPAN_BACKGROUND: Case = Case {
    css: ".a { background-color: rgb(200, 10, 10); } \
          .b { background-color: rgb(10, 10, 200); }",
    build: span_in_paragraph,
    from: "t a",
    to: "t b",
};

#[test]
fn an_inline_spans_background_follows_its_transition() {
    let (mut doc, s) = SPAN_BACKGROUND.started();
    age_transitions(&mut doc, s, MID);
    let (px, stats) = frame(&mut doc);
    let now = rgb_of(
        doc.tree
            .get(s.0)
            .unwrap()
            .computed_style
            .background_color()
            .unwrap(),
    );
    assert!(now != RED && now != BLUE, "between the ends, got {now:?}");
    assert!(exact(&px, now) > 1000, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
    assert_eq!(shapes(&stats), 0, "a background frame shapes nothing");
    SPAN_BACKGROUND.finished_matches_the_twin();
}

/// A background that fades **in**: the layout was built with no span for it,
/// so the frame it appears on has to rebuild the layout — once.
const SPAN_BACKGROUND_IN: Case = Case {
    css: ".b { background-color: rgb(10, 10, 200); }",
    build: span_in_paragraph,
    from: "t",
    to: "t b",
};

#[test]
fn an_inline_spans_background_fades_in_from_none() {
    let (mut doc, s) = SPAN_BACKGROUND_IN.started();
    // A background part-way in from nothing is translucent, over a surface
    // nothing else paints: count what is neither clear nor opaque. Before the
    // run that is the glyphs' antialiased edges.
    let translucent = |px: &[u8]| {
        let px = px.as_chunks::<4>().0;
        px.iter().filter(|p| p[3] > 0 && p[3] < 255).count()
    };
    let edges = translucent(&paint(&mut doc));
    age_transitions(&mut doc, s, MID);
    let (px, first) = frame(&mut doc);
    assert!(
        translucent(&px) > edges + 1000,
        "the half-faded rectangle: {edges} -> {} translucent px",
        translucent(&px)
    );
    assert_eq!(
        first.get(Counter::ShapeIfcBuild),
        1,
        "the frame the background appears on builds its span"
    );
    let (_, second) = frame(&mut doc);
    assert_eq!(shapes(&second), 0, "and no later frame shapes anything");
    SPAN_BACKGROUND_IN.finished_matches_the_twin();
}

/// A background that fades **out** ends at nothing, not at the last colour
/// the layout recorded.
const SPAN_BACKGROUND_OUT: Case = Case {
    css: ".a { background-color: rgb(200, 10, 10); }",
    build: span_in_paragraph,
    from: "t a",
    to: "t",
};

#[test]
fn an_inline_spans_background_fades_out_to_none() {
    SPAN_BACKGROUND_OUT.finished_matches_the_twin();
}

/// A padded inline background is the element's box as it is styled now.
const SPAN_PADDING: Case = Case {
    css: ".a { background-color: rgb(200, 10, 10); padding: 0 2px; } \
          .b { background-color: rgb(200, 10, 10); padding: 0 30px; } \
          .t { transition: padding-left 100s linear, padding-right 100s linear; }",
    build: span_in_paragraph,
    from: "t a",
    to: "t b",
};

#[test]
fn an_inline_spans_background_follows_a_padding_transition() {
    let (mut doc, s) = SPAN_PADDING.started();
    let before = exact(&paint(&mut doc), RED);
    age_transitions(&mut doc, s, MID);
    let (px, _) = frame(&mut doc);
    assert!(
        exact(&px, RED) > before + 1000,
        "mid-run the rectangle is wider: {before} -> {} px",
        exact(&px, RED)
    );
    SPAN_PADDING.finished_matches_the_twin();
}

// ── once the colour stops moving ────────────────────────────────────────────

/// The colour the first text range of `root`'s layout was shaped in.
fn shaped_colour(doc: &RinchDocument, root: NodeId) -> Rgb {
    let layout = doc.tree.get(root.0).unwrap().text_layout.as_ref();
    rgb_of(layout.expect("an IFC root").text_ranges[0].color)
}

/// While the colour moves, paint recolours a layout shaped in the start
/// colour. The frame that ends the run rebuilds it, so every later paint is
/// on the layout's own brushes again and compares nothing.
#[test]
fn the_frame_that_ends_a_colour_transition_reshapes_the_text_once() {
    let (mut doc, p) = ROOT_COLOUR.started();
    age_transitions(&mut doc, p, MID);
    let (_, running) = frame(&mut doc);
    assert_eq!(shapes(&running), 0);
    assert_eq!(shaped_colour(&doc, p), RED, "still the layout it started with");

    age_transitions(&mut doc, p, LONG * 2.0);
    let (_, last) = frame(&mut doc);
    assert_eq!(last.get(Counter::ShapeIfcBuild), 1, "the ending frame");
    assert_eq!(last.get(Counter::TaffyRootComputes), 0, "moves no box");
    assert_eq!(shaped_colour(&doc, p), BLUE);

    let (_, after) = frame(&mut doc);
    assert_eq!(shapes(&after), 0, "and nothing after it");
}

#[test]
fn the_frame_that_settles_a_colour_animation_reshapes_the_text_once() {
    let mut doc = doc_with(KEYFRAMES);
    let p = root_text(&mut doc, "k");
    settle(&mut doc);
    paint(&mut doc);
    age_animations(&mut doc, p, MID);
    let (_, running) = frame(&mut doc);
    assert_eq!(shapes(&running), 0);

    age_animations(&mut doc, p, LONG * 2.0);
    let (_, last) = frame(&mut doc);
    assert_eq!(last.get(Counter::ShapeIfcBuild), 1, "the settling frame");
    assert_eq!(shaped_colour(&doc, p), BLUE);
    let (_, after) = frame(&mut doc);
    assert_eq!(shapes(&after), 0, "a settled fill is re-applied quietly");
}

/// The same end, for text an anonymous block box lays out: no walk from the
/// element finds that box.
#[test]
fn the_frame_that_ends_a_colour_transition_reshapes_an_anonymous_boxs_text() {
    let (mut doc, d) = ANON_BOX.started();
    age_transitions(&mut doc, d, LONG * 2.0);
    let (_, last) = frame(&mut doc);
    assert_eq!(last.get(Counter::ShapeIfcBuild), 1);
    let (_, after) = frame(&mut doc);
    assert_eq!(shapes(&after), 0);
}

/// And for a split inline (#513): its text is laid out by the boxes around
/// its fragments, which belong to its container and are found only through
/// the text itself.
fn split_inline(doc: &mut RinchDocument, class: &str) -> NodeId {
    let body = doc.body();
    let p = el(doc, body, "div", "");
    let s = el(doc, p, "span", class);
    text(doc, s, "MMMM");
    let block = el(doc, s, "div", "still");
    text(doc, block, "HHHH");
    text(doc, s, "MMMM");
    s
}

const SPLIT_INLINE: Case = Case {
    css: ".still { color: rgb(10, 160, 10); } \
          .a { color: rgb(200, 10, 10); } .b { color: rgb(10, 10, 200); }",
    build: split_inline,
    from: "t a",
    to: "t b",
};

#[test]
fn a_split_inlines_text_follows_its_colour_transition_and_settles() {
    let (mut doc, s) = SPLIT_INLINE.started();
    age_transitions(&mut doc, s, MID);
    let (px, running) = frame(&mut doc);
    let now = colour_of(&doc, s);
    assert!(exact(&px, now) > 600, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
    assert_eq!(shapes(&running), 0);

    age_transitions(&mut doc, s, LONG * 2.0);
    let (_, last) = frame(&mut doc);
    assert_eq!(
        last.get(Counter::ShapeIfcBuild),
        2,
        "the box around each of its two fragments is rebuilt"
    );
    SPLIT_INLINE.finished_matches_the_twin();
}

/// A flex item's text is a leaf: paint colours it from the live style and no
/// layout holds its colour, so the end of its transition owes nothing.
#[test]
fn the_end_of_a_text_leafs_colour_transition_reshapes_nothing() {
    let (mut doc, p) = LEAF.started();
    age_transitions(&mut doc, p, LONG * 2.0);
    let (_, last) = frame(&mut doc);
    assert_eq!(shapes(&last), 0);
    assert_eq!(last.get(Counter::TaffyRootComputes), 0);
}

// ── @keyframes ──────────────────────────────────────────────────────────────

const KEYFRAMES: &str = "@keyframes tint { from { color: rgb(200, 10, 10); } \
                         to { color: rgb(10, 10, 200); } } \
                         .k { animation: tint 100s linear forwards; }";

#[test]
fn a_running_colour_animation_on_block_text_is_drawn_in_the_frames_colour() {
    let mut doc = doc_with(KEYFRAMES);
    let p = root_text(&mut doc, "k");
    settle(&mut doc);
    paint(&mut doc);
    age_animations(&mut doc, p, MID);
    let (px, stats) = frame(&mut doc);
    let now = colour_of(&doc, p);
    assert!(now != RED && now != BLUE, "between the ends, got {now:?}");
    assert!(exact(&px, now) > 500, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
    assert_eq!(shapes(&stats), 0, "a colour frame shapes nothing");
    assert_eq!(stats.get(Counter::TaffyRootComputes), 0);

    // And its fill, which is written by one tick and by no cascade.
    age_animations(&mut doc, p, LONG * 2.0);
    let (px, _) = frame(&mut doc);
    assert!(exact(&px, BLUE) > 500, "{} px of the fill", exact(&px, BLUE));
}

#[test]
fn a_running_colour_animation_on_an_inline_span_is_drawn_in_the_frames_colour() {
    let mut doc = doc_with(KEYFRAMES);
    let s = span_in_paragraph(&mut doc, "k");
    settle(&mut doc);
    paint(&mut doc);
    age_animations(&mut doc, s, MID);
    let (px, _) = frame(&mut doc);
    let now = colour_of(&doc, s);
    assert!(exact(&px, now) > 200, "{} px of {now:?}", exact(&px, now));
    assert_eq!(exact(&px, RED), 0);
}

// ── font-size ───────────────────────────────────────────────────────────────

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// `line-height: 1.5` so the line box follows the font size.
const SPAN_FONT_SIZE: &str = ".para { width: 500px; line-height: 1.5; font-size: 20px; } \
                              .a { font-size: 20px; } .b { font-size: 80px; }";

fn sized_span(doc: &mut RinchDocument, class: &str, wrapper: &str) -> (NodeId, NodeId) {
    let body = doc.body();
    let p = el(doc, body, "div", "para");
    text(doc, p, "MMM ");
    let s = el(doc, p, "span", class);
    if !wrapper.is_empty() {
        doc.set_attribute(s, "style", wrapper);
    }
    text(doc, s, "HHH");
    (p, s)
}

fn font_size_transition_on_a_span(wrapper: &str) {
    let mut twin = doc_with(SPAN_FONT_SIZE);
    let (twin_p, _) = sized_span(&mut twin, "b", wrapper);
    settle(&mut twin);
    let want = height_of(&twin, twin_p);
    let want_px = paint(&mut twin);

    let mut doc = doc_with(&format!("{TRANSITION_CSS} {SPAN_FONT_SIZE}"));
    let (p, s) = sized_span(&mut doc, "t a", wrapper);
    settle(&mut doc);
    paint(&mut doc);
    let small = height_of(&doc, p);
    assert!(want > small + 20.0, "off the fixed point: {small} -> {want}");

    doc.set_attribute(s, "class", "t b");
    doc.resolve_layout(VW, VH);
    age_transitions(&mut doc, s, MID);
    frame(&mut doc);
    let mid = height_of(&doc, p);
    assert!(
        mid > small + 5.0 && mid < want - 5.0,
        "mid-run the paragraph is as tall as the frame's font size makes it: \
         {small} < {mid} < {want}"
    );

    age_transitions(&mut doc, s, LONG * 2.0);
    let (px, _) = frame(&mut doc);
    assert_eq!(height_of(&doc, p), want, "the finished height (was {small})");
    assert!(px == want_px, "and the finished frame is the twin's");
}

#[test]
fn a_font_size_transition_on_an_inline_span_resizes_its_paragraph() {
    font_size_transition_on_a_span("");
}

#[test]
fn a_font_size_transition_on_a_contents_wrapper_resizes_its_paragraph() {
    font_size_transition_on_a_span("display: contents");
}

#[test]
fn a_font_size_animation_on_an_inline_span_resizes_its_paragraph() {
    let css = "@keyframes grow { from { font-size: 20px; } to { font-size: 80px; } } \
               .k { animation: grow 100s linear forwards; }";
    let mut doc = doc_with(&format!("{SPAN_FONT_SIZE} {css}"));
    let (p, s) = sized_span(&mut doc, "k", "");
    settle(&mut doc);
    paint(&mut doc);
    let small = height_of(&doc, p);
    age_animations(&mut doc, s, MID);
    frame(&mut doc);
    let mid = height_of(&doc, p);
    assert!(mid > small + 5.0, "mid-run: {small} -> {mid}");
    age_animations(&mut doc, s, LONG * 2.0);
    frame(&mut doc);
    assert_eq!(height_of(&doc, p), 120.0, "80px at line-height 1.5");
}
