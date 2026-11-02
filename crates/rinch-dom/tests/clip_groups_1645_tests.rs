//! Entries under clippers that share no pixel are gathered by clipper (#1645):
//! on the software painter the picture must be the one the true sequence paints.
//! `PaintOrder`'s own tests pin the permutation; these pin what reaches a painter
//! from a document. Vello's picture does change at clip edges: see
//! `gpu_clip_groups_1645_tests` in `rinch`.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VP: (f32, f32) = (320.0, 240.0);
const RED: [u8; 4] = [200, 30, 40, 255];
const GREEN: [u8; 4] = [20, 170, 60, 255];
const BLUE: [u8; 4] = [30, 50, 210, 255];

fn css([r, g, b, _]: [u8; 4]) -> String {
    format!("rgb({r}, {g}, {b})")
}

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn page() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    (doc, body)
}

fn pane(doc: &mut RinchDocument, body: NodeId, left: f32, width: f32) -> NodeId {
    el(
        doc,
        body,
        &format!(
            "position: absolute; left: {left}px; top: 11px; width: {width}px; height: 170px; \
             overflow: hidden"
        ),
    )
}

/// A band across its pane, `17px` down, at `z`.
fn band(doc: &mut RinchDocument, pane: NodeId, z: i32, color: [u8; 4]) {
    el(
        doc,
        pane,
        &format!(
            "position: absolute; left: 0; top: 17px; width: 100%; height: 43px; z-index: {z}; \
             background-color: {}",
            css(color)
        ),
    );
}

/// Two panes, A with bands at `z` 1 and 3 and B with one at `z` 2 between them.
/// Paint gathering A's bands would leave B's on top.
fn alternating(a: (f32, f32), b: (f32, f32)) -> RinchDocument {
    let (mut doc, body) = page();
    let pa = pane(&mut doc, body, a.0, a.1);
    let pb = pane(&mut doc, body, b.0, b.1);
    band(&mut doc, pa, 1, RED);
    band(&mut doc, pb, 2, GREEN);
    band(&mut doc, pa, 3, BLUE);
    doc
}

fn render(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    doc.resolve_layout(VP.0, VP.1);
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
    painter
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

#[derive(Default)]
struct Clips(usize);

impl Painter for Clips {
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
        self.0 += 1;
    }
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

fn clips(doc: &mut RinchDocument, scale: f64) -> usize {
    doc.resolve_layout(VP.0, VP.1);
    let mut c = Clips::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut c,
        scale,
        VP,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    c.0
}

/// Where the panes overlap, the band above all of them is A's second, with B's
/// between A's two: the pair of entries that must not be gathered.
#[test]
fn bands_of_overlapping_panes_keep_their_stacking_order() {
    let mut doc = alternating((13.0, 101.0), (97.0, 101.0));
    assert_eq!(clips(&mut doc, 1.0), 3, "A, B and A again");
    let p = render(&mut doc, 1.0);
    let y = 11 + 17 + 21;
    assert_eq!(px(&p, 40, y), BLUE, "A alone");
    assert_eq!(
        px(&p, 105, y),
        BLUE,
        "A's top band over B's, where both reach"
    );
    assert_eq!(px(&p, 150, y), GREEN, "B alone");
}

/// The same bands under panes clear of each other: the entries are gathered
/// (one clip each) and each pane shows what it did.
#[test]
fn bands_of_panes_apart_are_gathered_and_look_the_same() {
    let mut doc = alternating((13.0, 83.0), (97.0, 101.0));
    assert_eq!(clips(&mut doc, 1.0), 2);
    let p = render(&mut doc, 1.0);
    let y = 11 + 17 + 21;
    assert_eq!(px(&p, 40, y), BLUE);
    assert_eq!(px(&p, 150, y), GREEN);
    assert_eq!(px(&p, 96, y), [0, 0, 0, 0], "nothing between the panes");
}

/// At 1.5x a seam at 67px is at 100.5: both panes antialias device pixel 100,
/// so A's top band and B's band composite there in their true order. At 2x the
/// same seam is a pixel edge.
#[test]
fn panes_meeting_inside_a_device_pixel_are_not_gathered() {
    let mut doc = alternating((13.0, 54.0), (67.0, 101.0));
    assert_eq!(clips(&mut doc, 1.5), 3, "the seam is shared at 1.5x");
    assert_eq!(clips(&mut doc, 2.0), 2, "the seam is not shared at 2x");
}

/// A drag ghost's scene is appended at a fractional offset, so "whole device
/// pixel" means nothing while it is painted: panes meeting on a whole pixel here
/// may share one on screen. Its stacking root is a context of its own so that
/// the panes are a sequence in it. The document's paint after it still groups.
#[test]
fn a_drag_ghost_keeps_the_true_sequence() {
    let (mut doc, body) = page();
    let root = el(
        &mut doc,
        body,
        "position: relative; z-index: 0; width: 300px; height: 200px",
    );
    let pa = pane(&mut doc, root, 13.0, 84.0);
    let pb = pane(&mut doc, root, 97.0, 101.0);
    band(&mut doc, pa, 1, RED);
    band(&mut doc, pb, 2, GREEN);
    band(&mut doc, pa, 3, BLUE);
    assert_eq!(clips(&mut doc, 1.0), 2, "control: the document groups");

    let mut c = Clips::default();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_subtree(
        &doc.tree,
        &mut c,
        root.0,
        1.0,
        &mut doc.font_cx,
        &mut layout_cx,
    );
    assert_eq!(c.0, 3, "A, B and A again");

    assert_eq!(clips(&mut doc, 1.0), 2, "the document groups afterwards");
}

/// A scroller's bars are drawn after its last entry (#1717), and an earlier
/// pane's late entry gathered ahead of that position must not bring the bars
/// forward: `[A] [B] [C, last of C] [A]`, with C's band under its own thumb.
#[test]
fn a_bar_stays_over_the_entries_before_it_when_later_ones_are_gathered() {
    let bar_column = |gather_late_entry: bool| {
        let (mut doc, body) = page();
        let pa = pane(&mut doc, body, 3.0, 61.0);
        let pb = pane(&mut doc, body, 67.0, 61.0);
        let pc = el(
            &mut doc,
            body,
            "position: absolute; left: 131px; top: 11px; width: 101px; height: 170px; \
             overflow-y: scroll",
        );
        el(&mut doc, pc, "height: 900px");
        el(
            &mut doc,
            pc,
            &format!(
                "position: absolute; left: 0; top: 0; width: 101px; height: 170px; z-index: 3; \
                 background-color: {}",
                css(RED)
            ),
        );
        band(&mut doc, pa, 1, GREEN);
        band(&mut doc, pb, 2, BLUE);
        if gather_late_entry {
            band(&mut doc, pa, 4, BLUE);
        }
        let p = render(&mut doc, 1.0);
        // The rightmost pixels of the scroller's box, at its top: the thumb.
        (225..232).map(|x| px(&p, x, 14)).collect::<Vec<_>>()
    };
    let alone = bar_column(false);
    assert!(
        alone.iter().any(|&c| c != RED),
        "control: the thumb shows over the band: {alone:?}"
    );
    assert_eq!(bar_column(true), alone);
}

/// Gathering moves a scroller's last entry ahead of a sibling in the same
/// scroller, under the bars: `[S, I] [S, J] [S, I]`, the bars due after the
/// third. They are drawn at that position, after J's band, not when the entry
/// that fills it in the true sequence is reached.
#[test]
fn a_bar_stays_over_a_sibling_its_last_entry_is_gathered_ahead_of() {
    let bar_column = |gather_late_entry: bool| {
        let (mut doc, body) = page();
        let s = el(
            &mut doc,
            body,
            "position: absolute; left: 13px; top: 11px; width: 201px; height: 170px; \
             overflow-y: scroll",
        );
        el(&mut doc, s, "height: 900px");
        let inner = |doc: &mut RinchDocument, left: u32, width: u32| {
            let style = format!(
                "position: absolute; left: {left}px; top: 0; width: {width}px; height: 170px; \
                 overflow: hidden"
            );
            el(doc, s, &style)
        };
        let i = inner(&mut doc, 0, 90);
        let j = inner(&mut doc, 100, 101);
        el(
            &mut doc,
            i,
            "position: absolute; width: 90px; height: 170px; z-index: 1; \
             background-color: rgb(20, 170, 60)",
        );
        el(
            &mut doc,
            j,
            &format!(
                "position: absolute; width: 101px; height: 170px; z-index: 2; \
                 background-color: {}",
                css(RED)
            ),
        );
        if gather_late_entry {
            el(
                &mut doc,
                i,
                "position: absolute; width: 90px; height: 170px; z-index: 3; \
                 background-color: rgb(30, 50, 210)",
            );
        }
        let p = render(&mut doc, 1.0);
        (206..214).map(|x| px(&p, x, 14)).collect::<Vec<_>>()
    };
    let alone = bar_column(false);
    assert!(
        alone.iter().any(|&c| c != RED),
        "control: the thumb shows over the band: {alone:?}"
    );
    assert_eq!(bar_column(true), alone);
}

/// A two-tree differential, not a check: writes one line per seed to
/// `$RINCH_1645_OUT`, `seed hash`, and to `$RINCH_1645_OUT.clips` the clips each
/// pushed, so a build with grouping off can be compared byte for byte, and the
/// clip counts prove the seeds exercised it.
#[test]
#[ignore = "a harness: writes hashes to $RINCH_1645_OUT for a two-tree differential"]
fn random_alternating_panes_pixel_hashes() {
    let (mut hashes, mut counts) = (String::new(), String::new());
    for seed in 0..600u64 {
        let mut doc = random_doc(seed);
        let scale = [1.0, 1.5, 2.0, 1.25][seed as usize % 4];
        counts.push_str(&format!("{seed} {}\n", clips(&mut doc, scale)));
        let p = render(&mut doc, scale);
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in p.pixels() {
            hash = (hash ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        hashes.push_str(&format!("{seed} {hash:016x}\n"));
    }
    let path = std::env::var("RINCH_1645_OUT").expect("RINCH_1645_OUT");
    std::fs::write(&path, hashes).unwrap();
    std::fs::write(format!("{path}.clips"), counts).unwrap();
}

struct Rng(u64);
impl Rng {
    fn pick(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

/// Panes, some apart, some overlapping, some meeting inside a pixel, with
/// entries whose `z-index`es interleave them, over what makes order visible:
/// translucency, shadows, fixed and absolute boxes that leave a pane, rounded
/// clips, scrolled scrollers and their bars.
fn random_doc(seed: u64) -> RinchDocument {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1645);
    let (mut doc, body) = page();
    let mut left = r.pick(9) as f32;
    let panes: Vec<NodeId> = (0..2 + r.pick(4))
        .map(|_| {
            let width = 30.0 + r.pick(50) as f32;
            let top = r.pick(40);
            let height = 60 + r.pick(120);
            let overflow = ["hidden", "auto", "scroll", "clip"][r.pick(4) as usize];
            let radius = ["", "", "border-radius: 9px;"][r.pick(3) as usize];
            let position = ["absolute", "absolute", "relative"][r.pick(3) as usize];
            let p = el(
                &mut doc,
                body,
                &format!(
                    "position: {position}; left: {left}px; top: {top}px; width: {width}px; \
                     height: {height}px; overflow: {overflow}; {radius}"
                ),
            );
            // Apart, overlapping by a few, or meeting on a fraction.
            left += width + [4.0, 0.0, -6.0, 0.5, -0.5][r.pick(5) as usize];
            p
        })
        .collect();
    for z in 0..6 + r.pick(24) as i32 {
        let parent = panes[r.pick(panes.len() as u64) as usize];
        let position = ["relative", "absolute", "absolute", "fixed"][r.pick(4) as usize];
        let color = format!(
            "rgba({}, {}, {}, 0.{})",
            r.pick(256),
            r.pick(256),
            r.pick(256),
            3 + r.pick(7)
        );
        let opacity = ["", "", "opacity: 0.7;"][r.pick(3) as usize];
        let shadow = ["", "", "box-shadow: 3px 2px 5px rgb(0, 90, 90);"][r.pick(3) as usize];
        el(
            &mut doc,
            parent,
            &format!(
                "position: {position}; z-index: {z}; left: {}px; top: {}px; width: {}px; \
                 height: {}px; background-color: {color}; {opacity} {shadow}",
                r.pick(30) as i64 - 10,
                r.pick(60) as i64 - 10,
                20 + r.pick(90),
                10 + r.pick(70),
            ),
        );
    }
    doc
}
