//! Review of PR #1641: entries whose clip chains differ keep the outer clips
//! they share open.
//!
//! * `entries_under_one_outer_clipper_push_it_once` pins the push count the PR
//!   is for: N positioned boxes, each inside its own small clipper under one
//!   shared outer clipper, interleaved with N boxes under the outer clipper
//!   alone. Re-pushing the whole chain per entry pushes the outer clip once per
//!   entry (4N + 1 clips in all here, brackets included); keeping the shared prefix gives 2N + 2.
//! * `random_stacking_documents_paint_balanced` paints random stacking
//!   documents through a counting painter and checks the clip/layer stack never
//!   underflows and ends empty.
//! * `random_stacking_documents_pixel_hashes` writes one pixel hash per seed to
//!   `$RINCH_1641_OUT` (when set), so the same seeds can be painted by the PR
//!   and by the base and the two files compared byte for byte.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

#[derive(Default)]
struct Count {
    clips: usize,
    layers: usize,
    depth: isize,
    min_depth: isize,
}

impl Painter for Count {
    fn reset(&mut self) {}
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        _: f32,
        _: Affine,
        _: Option<Affine>,
        _: &Brush,
        _: bool,
        _: &[i16],
        _: &[PaintGlyph],
    ) {
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {
        self.clips += 1;
        self.depth += 1;
    }
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {
        self.layers += 1;
        self.depth += 1;
    }
    fn pop_layer(&mut self) {
        self.depth -= 1;
        self.min_depth = self.min_depth.min(self.depth);
    }
}

const VP: (f32, f32) = (300.0, 300.0);

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", style);
    doc.append_child(parent, d);
    d
}

fn count(doc: &mut RinchDocument, scale: f64) -> Count {
    let mut c = Count::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut c,
        scale,
        VP,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    c
}

fn many_under_one(n: usize) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = el(
        &mut doc,
        body,
        "overflow: hidden; width: 200px; height: 250px; margin: 7px",
    );
    for i in 0..n {
        let c = el(
            &mut doc,
            outer,
            "overflow: hidden; width: 100px; height: 3px",
        );
        el(
            &mut doc,
            c,
            &format!(
                "position: relative; width: 300px; height: 6px; \
                 background-color: rgb({}, 0, 0)",
                10 + i
            ),
        );
        el(
            &mut doc,
            outer,
            "position: relative; width: 300px; height: 2px; \
             background-color: rgb(0, 200, 0)",
        );
    }
    doc.resolve_layout(VP.0, VP.1);
    doc
}

#[test]
fn entries_under_one_outer_clipper_push_it_once() {
    for n in [10usize, 40] {
        let mut doc = many_under_one(n);
        let c = count(&mut doc, 1.0);
        assert_eq!(c.depth, 0, "balanced");
        assert!(c.min_depth >= 0, "never underflows");
        // n inner chain clips + the outer clip once + each inner clipper's own
        // bracket (n) + the outer clipper's bracket. Re-pushing the whole chain
        // per entry (the base) is 4n + 1.
        assert_eq!(
            c.clips,
            2 * n + 2,
            "{n} inner clippers under one outer clipper: re-pushing the shared \
             outer clip per entry pushes 4n+1"
        );
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn of<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.pick(xs.len() as u64) as usize]
    }
}

fn random_doc(seed: u64) -> RinchDocument {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5);
    let mut doc = RinchDocument::new();
    let body = doc.body();
    if r.pick(2) == 0 {
        doc.set_attribute(body, "style", "position: relative");
    }
    let mut scrolled = Vec::new();
    let mut frontier = vec![(body, 0u32)];
    let mut made = 0;
    while let Some((parent, depth)) = frontier.pop() {
        let kids = 1 + r.pick(4);
        for _ in 0..kids {
            if made > 45 {
                break;
            }
            made += 1;
            let overflow = r.of(&["visible", "visible", "hidden", "hidden", "auto", "clip"]);
            let position = r.of(&[
                "static", "static", "relative", "relative", "absolute", "fixed",
            ]);
            let z = r.of(&[
                "",
                "",
                "z-index: -1;",
                "z-index: 0;",
                "z-index: 2;",
                "z-index: 5;",
            ]);
            let opacity = r.of(&["", "", "", "opacity: 0.6;"]);
            let transform = r.of(&[
                "",
                "",
                "",
                "transform: translate(5px, 3px);",
                "transform: rotate(10deg);",
            ]);
            let radius = r.of(&["", "", "border-radius: 12px;"]);
            let border = r.of(&["", "", "border: 3px solid rgb(0, 0, 255);"]);
            let shadow = r.of(&[
                "",
                "",
                "box-shadow: 6px 4px 5px 2px rgba(0, 120, 0, 0.7);",
                "box-shadow: inset 3px 3px 4px rgb(90, 0, 90), -5px -2px 0px rgb(200, 200, 0);",
            ]);
            let w = 20 + r.pick(220);
            let h = 10 + r.pick(160);
            let (l, t) = (r.pick(80) as i64 - 20, r.pick(80) as i64 - 20);
            let color = format!("rgb({}, {}, {})", r.pick(256), r.pick(256), r.pick(256));
            let style = format!(
                "overflow: {overflow}; position: {position}; {z} {opacity} {transform} \
                 {radius} {border} {shadow} width: {w}px; height: {h}px; left: {l}px; top: {t}px; \
                 margin: {}px; background-color: {color}",
                r.pick(6)
            );
            let d = el(&mut doc, parent, &style);
            if overflow == "auto" || overflow == "hidden" {
                scrolled.push(d);
            }
            if depth < 4 {
                frontier.push((d, depth + 1));
            }
        }
    }
    doc.resolve_layout(VP.0, VP.1);
    for (i, d) in scrolled.into_iter().enumerate() {
        if i % 2 == 0 {
            doc.tree.nodes[d.0].scroll_offset = (0.0, 7.0 + i as f64);
        }
    }
    doc
}

const SEEDS: u64 = 400;

#[test]
fn random_stacking_documents_paint_balanced() {
    for seed in 0..SEEDS {
        let mut doc = random_doc(seed);
        let c = count(&mut doc, if seed % 3 == 0 { 1.5 } else { 1.0 });
        assert_eq!(c.depth, 0, "seed {seed}: unbalanced");
        assert!(c.min_depth >= 0, "seed {seed}: underflow");
    }
}

#[test]
fn random_stacking_documents_pixel_hashes() {
    let mut out = String::new();
    let mut total_clips = 0usize;
    for seed in 0..SEEDS {
        let mut doc = random_doc(seed);
        let scale = if seed % 3 == 0 { 1.5 } else { 1.0 };
        total_clips += count(&mut doc, scale).clips;
        let (w, h) = ((VP.0 as f64 * scale) as u32, (VP.1 as f64 * scale) as u32);
        let mut painter = TinySkiaPainter::new(w, h);
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            scale,
            VP,
            &mut doc.font_cx,
            &mut layout_cx,
        );
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut nonzero = 0usize;
        for &b in painter.pixels() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
            nonzero += (b != 0) as usize;
        }
        out.push_str(&format!("{seed} {hash:016x} {nonzero}\n"));
    }
    out.push_str(&format!("clips {total_clips}\n"));
    if let Ok(path) = std::env::var("RINCH_1641_OUT") {
        std::fs::write(path, out).unwrap();
    }
}

/// Two consecutive entries whose clips have the same rect and different radii
/// share no prefix: keeping the square clip open for the rounded one's entry
/// paints its corner. `A` (square) holds a red box, `B` (rounded, same rect,
/// pulled up over `A`) a blue one painted after it.
#[test]
fn a_clip_with_the_same_rect_and_other_radii_is_not_shared() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = el(
        &mut doc,
        body,
        "overflow: hidden; width: 100px; height: 100px",
    );
    el(
        &mut doc,
        a,
        "position: relative; width: 150px; height: 150px; background-color: rgb(255, 0, 0)",
    );
    let b = el(
        &mut doc,
        body,
        "overflow: hidden; width: 100px; height: 100px; margin-top: -100px; \
         border-radius: 40px",
    );
    el(
        &mut doc,
        b,
        "position: relative; width: 150px; height: 150px; background-color: rgb(0, 0, 255)",
    );
    doc.resolve_layout(VP.0, VP.1);
    let mut painter = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = |x: u32, y: u32| {
        let i = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[i], d[i + 1], d[i + 2], d[i + 3]]
    };
    assert_eq!(
        px(3, 3),
        [255, 0, 0, 255],
        "B's rounded corner shows A's box"
    );
    assert_eq!(px(50, 50), [0, 0, 255, 255], "B paints inside its clip");
    assert_eq!(px(120, 50), [0, 0, 0, 0], "nothing outside both clips");
}

/// The shape `ContextMenu` now emits near the bottom-right of the window: a
/// fixed full-window portal holding an absolute dropdown anchored by `right`
/// and `bottom`. Its far corner lands on the pointer whatever its size.
#[test]
fn a_bottom_right_anchored_dropdown_ends_at_the_pointer() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let portal = el(
        &mut doc,
        body,
        "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 200",
    );
    let dd = el(
        &mut doc,
        portal,
        "position: absolute; right: 50px; bottom: 100px; min-width: 180px; padding: 4px 0",
    );
    el(&mut doc, dd, "width: 230px; height: 300px");
    doc.resolve_layout(800.0, 600.0);
    let l = doc.tree.nodes[dd.0].layout;
    assert_eq!((l.width, l.height), (230.0, 308.0), "{l:?}");
    assert_eq!((l.x + l.width, l.y + l.height), (750.0, 500.0), "{l:?}");
}
