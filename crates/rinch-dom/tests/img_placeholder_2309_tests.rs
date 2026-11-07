//! #2309 — an `<img>` with no picture (no `src`, or a failed load) draws a
//! placeholder: a sized one a box with a faint border and its alt text, an
//! unsized one with alt text the text inline, a tracking pixel or a bare
//! unsized one nothing (`rinch_dom::img_placeholder`). Software painter;
//! the GPU twin is `crates/rinch/tests/gpu_img_placeholder_2309.rs`.
//!
//! Chrome 156 (headless, `--force-device-scale-factor=1`, standards mode)
//! measured: the box is Chrome's for a sized image without alt, a 1px
//! `#c0c0c0` ring on the content box's edge and 1px of white padding inside
//! it (its broken-image icon, which rinch does not draw, starts 2px in). A
//! sized image *with* alt is inline alt text in Chrome, its size ignored;
//! drawing the box with the alt text inside is Joe's rule (#2309), Chrome's
//! quirks-mode rendering. Chrome draws the box only when its content box is
//! at least 3px on each axis with no `src`, 18px after a failed load. An
//! image with alt and one dimension is an inline-block of its text there,
//! the declared dimension kept. An unsized `<img alt>` is a line box tall in
//! Chrome (20px at `line-height: 20px`, where a `<span>` of the text is its
//! 17px content area), as rinch's is; Chrome puts its 16px icon before the
//! text. `getSelection().toString()` copies no alt text (`a<img alt>b` is
//! `"ab"`), which `rinch/src/app/text_selection_copy_tests.rs` pins.
//!
//! Text geometry is never compared with Chrome's (it depends on the host's
//! fonts): the inline case is compared with the same text laid out as text
//! in the same document.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::Fill;
use peniko::kurbo::{Affine, Rect};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u32 = 300;
const H: u32 = 160;
const VP: (f32, f32) = (W as f32, H as f32);

const RED_3X2: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";
/// A `data:` URL that is not a picture: its load fails, synchronously.
const BROKEN: &str = "data:image/png;base64,AAAA";

const WHITE: [u8; 3] = [255, 255, 255];
const SILVER: [u8; 3] = [0xc0, 0xc0, 0xc0];

fn new_doc(css: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: #fff; font: 16px/20px sans-serif; color: #000; }} {css}"
    ));
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
    let n = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(n, k, v);
    }
    doc.append_child(parent, n);
    n
}

fn img(doc: &mut RinchDocument, attrs: &[(&str, &str)]) -> NodeId {
    let body = doc.body();
    el(doc, body, "img", attrs)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = &doc.tree.nodes[id.0];
    (n.layout.width, n.layout.height)
}

fn paint_at(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    doc.resolve_layout(VP.0, VP.1);
    let (pw, ph) = (
        (W as f64 * scale).round() as u32,
        (H as f64 * scale).round() as u32,
    );
    let mut painter = TinySkiaPainter::new(pw, ph);
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

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    paint_at(doc, 1.0)
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

/// Whether the pixel is text ink: darker than the border colour.
fn ink(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v < 0x90)
}

/// The bounding box `[x0, y0, x1, y1]` (inclusive) of the pixels in `area`
/// for which `f` holds.
fn bbox(p: &TinySkiaPainter, area: [u32; 4], f: impl Fn([u8; 3]) -> bool) -> Option<[u32; 4]> {
    let mut out: Option<[u32; 4]> = None;
    for y in area[1]..area[3] {
        for x in area[0]..area[2] {
            if f(px(p, x, y)) {
                let b = out.get_or_insert([x, y, x, y]);
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
    }
    out
}

fn count(p: &TinySkiaPainter, f: impl Fn([u8; 3]) -> bool) -> usize {
    p.pixels()
        .chunks(4)
        .filter(|c| f([c[0], c[1], c[2]]))
        .count()
}

fn non_white(p: &TinySkiaPainter) -> usize {
    count(p, |c| c != WHITE)
}

const POS: &str = "position: absolute; left: 10px; top: 10px";

/// The box Chrome 156 draws for a sized `<img>` whose load failed: a 1px
/// silver ring on the content box's edge, white inside it, and the alt text
/// inside the 1px padding, clipped to the box.
fn assert_box(p: &TinySkiaPainter, [x, y, w, h]: [u32; 4], what: &str) {
    let ring = [
        (x, y),
        (x + w - 1, y),
        (x, y + h - 1),
        (x + w - 1, y + h - 1),
        (x + w / 2, y),
        (x, y + h / 2),
        (x + w - 1, y + h / 2),
        (x + w / 2, y + h - 1),
    ];
    for (rx, ry) in ring {
        assert_eq!(px(p, rx, ry), SILVER, "{what}: the border at ({rx}, {ry})");
    }
    // Inside the border, its 1px of padding.
    for (ix, iy) in [(x + 1, y + 1), (x + w - 2, y + h - 2), (x + 1, y + h - 2)] {
        assert_eq!(px(p, ix, iy), WHITE, "{what}: the padding at ({ix}, {iy})");
    }
    // Outside it, nothing.
    for (ox, oy) in [(x - 1, y - 1), (x + w, y + h / 2), (x + w / 2, y + h)] {
        assert_eq!(px(p, ox, oy), WHITE, "{what}: outside at ({ox}, {oy})");
    }
}

/// Text ink inside the box, starting 2px in (border and padding), and none
/// outside the box (the text is clipped to it).
fn assert_alt_in_box(p: &TinySkiaPainter, [x, y, w, h]: [u32; 4], what: &str) {
    let b = bbox(p, [x, y, x + w, y + h], ink).unwrap_or_else(|| panic!("{what}: no alt text"));
    assert!(
        b[0] >= x + 2 && b[1] >= y + 2 && b[2] <= x + w - 2 && b[3] <= y + h - 2,
        "{what}: alt text ink {b:?} not inside the padding box of {:?}",
        [x, y, w, h]
    );
    let all = bbox(p, [0, 0, p.width(), p.height()], ink).unwrap();
    assert_eq!(all, b, "{what}: ink outside the box");
}

#[test]
fn a_sized_img_with_no_src_draws_a_box_and_its_alt_text() {
    let mut doc = new_doc("");
    let i = img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello world"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(size(&doc, i), (100.0, 50.0));
    assert_box(&p, [10, 10, 100, 50], "no src");
    assert_alt_in_box(&p, [10, 10, 100, 50], "no src");
}

/// The alt text in the box starts inside the 1px ring and its 1px of
/// padding: it is drawn exactly as the same text laid out 2px in from the
/// box's corner.
#[test]
fn the_alt_text_sits_inside_the_ring_and_its_padding() {
    let mut doc = new_doc(".t { position: absolute; left: 12px; top: 92px; width: 96px }");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello world"),
        ],
    );
    let body = doc.body();
    let t = el(&mut doc, body, "div", &[("class", "t")]);
    let tt = doc.create_text("Hello world");
    doc.append_child(t, tt);
    let p = paint(&mut doc);
    let in_box = bbox(&p, [10, 10, 110, 60], ink).expect("alt text");
    let text = bbox(&p, [0, 80, W, H], ink).expect("the text");
    assert_eq!(
        in_box,
        [text[0], text[1] - 80, text[2], text[3] - 80],
        "the alt text is the text at (12, 12)"
    );
}

#[test]
fn a_sized_img_whose_load_failed_draws_the_box() {
    let mut doc = new_doc("");
    let i = img(
        &mut doc,
        &[
            ("style", &format!("{POS}; width: 100px; height: 50px")),
            ("src", BROKEN),
            ("alt", "Hello world"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(size(&doc, i), (100.0, 50.0));
    assert_box(&p, [10, 10, 100, 50], "failed");
    assert_alt_in_box(&p, [10, 10, 100, 50], "failed");
}

#[test]
fn a_sized_box_without_alt_is_the_ring_alone() {
    let mut doc = new_doc("");
    img(
        &mut doc,
        &[("style", POS), ("width", "100"), ("height", "50")],
    );
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "no alt");
    assert_eq!(count(&p, ink), 0, "no text");
    // The ring alone: 2 * (100 + 50) - 4 pixels.
    assert_eq!(non_white(&p), 296);
}

/// The box sits in the content box: inside the element's own padding and
/// border, which an author rule may set (and which the ring does not use).
#[test]
fn the_box_is_inside_the_elements_own_padding_and_border() {
    let mut doc = new_doc("img { border: 3px solid #00f; padding: 5px }");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hi"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(px(&p, 11, 11), [0, 0, 255], "its own border");
    assert_box(&p, [18, 18, 100, 50], "inset by 8px");
}

/// A load that fails on the loader's thread reaches the placeholder through
/// the drain, with nothing drawn while it is loading.
struct Fails;
impl ImageLoader for Fails {
    fn load(&self, src: &str) -> ImageLoadResult {
        std::thread::sleep(Duration::from_millis(30));
        ImageLoadResult::Failed(format!("{src}: no"))
    }
}

#[test]
fn a_load_that_fails_later_turns_into_the_box() {
    let mut doc = new_doc("");
    doc.tree.image_loader = Some(Arc::new(Fails));
    let i = img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello"),
            ("src", "remote-2309.png"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(non_white(&p), 0, "nothing while it loads");
    doc.tree.consume_paint_dirty();
    // At the same viewport: nothing but the failure dirties the image.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !doc.tree.image_cache.is_failed("remote-2309.png") {
        assert!(Instant::now() < deadline, "the load never failed");
        std::thread::sleep(Duration::from_millis(5));
        doc.resolve_layout(VP.0, VP.1);
    }
    assert!(
        doc.tree.paint_dirty_nodes.contains(&i.0),
        "the failure queues the image for paint"
    );
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "failed later");
}

/// An alt-only image whose load fails later grows from 0x0 to its text.
#[test]
fn an_inline_placeholder_whose_load_fails_later_takes_its_text_box() {
    let mut doc = new_doc("");
    doc.tree.image_loader = Some(Arc::new(Fails));
    let i = img(&mut doc, &[("alt", "Hello"), ("src", "remote-2309-b.png")]);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (0.0, 0.0), "loading");
    let deadline = Instant::now() + Duration::from_secs(10);
    while size(&doc, i).0 == 0.0 {
        assert!(Instant::now() < deadline, "never re-measured");
        std::thread::sleep(Duration::from_millis(5));
        doc.resolve_layout(VP.0, VP.1);
    }
    assert!(size(&doc, i).1 > 0.0);
}

/// The inline case is the alt text drawn as text: the same ink, in the same
/// place, as the text itself in a sibling line, and the image is as wide as
/// the text's advance.
fn inline_matches_text(scale: f64) {
    let mut doc = new_doc(".l { position: absolute; left: 10px; width: 280px }");
    let body = doc.body();
    let a = el(
        &mut doc,
        body,
        "div",
        &[("class", "l"), ("style", "top: 10px")],
    );
    let t = doc.create_text("xHelloy");
    let b = el(
        &mut doc,
        body,
        "div",
        &[("class", "l"), ("style", "top: 60px")],
    );
    doc.append_child(b, t);
    let x = doc.create_text("x");
    doc.append_child(a, x);
    let i = el(&mut doc, a, "img", &[("alt", "Hello")]);
    let y = doc.create_text("y");
    doc.append_child(a, y);
    // Its advance: the same text, as an inline-block of its own.
    let c = el(
        &mut doc,
        body,
        "span",
        &[(
            "style",
            "position: absolute; top: 110px; left: 10px; display: inline-block",
        )],
    );
    let ct = doc.create_text("Hello");
    doc.append_child(c, ct);
    let p = paint_at(&mut doc, scale);
    let (iw, ih) = size(&doc, i);
    let (cw, _) = size(&doc, c);
    assert!(
        (iw - cw).abs() <= 1.0 && iw > 0.0,
        "the image is the text's width: {iw} vs {cw}"
    );
    // Its font's content area, which `line-height` does not change (and
    // which may be taller than the 20px line, as a `<span>`'s is).
    assert!(ih > 12.0 && ih < 32.0, "its content area: {ih}");
    let s = |v: u32| (f64::from(v) * scale).round() as u32;
    let row_a = bbox(&p, [0, s(10), p.width(), s(40)], ink).expect("ink in the image's line");
    let row_b = bbox(&p, [0, s(60), p.width(), s(90)], ink).expect("ink in the text's line");
    // The image's box is laid out in whole pixels (rinch rounds layout), so
    // the "y" after it may sit up to a pixel off the text's.
    assert_eq!(
        [row_a[0], row_a[1], row_a[3]],
        [row_b[0], row_b[1] - s(50), row_b[3] - s(50)],
        "at {scale}x the image's line is drawn as the text line is"
    );
    assert!(
        row_a[2].abs_diff(row_b[2]) <= s(1),
        "{row_a:?} vs {row_b:?}"
    );
    // "xHello" pixel for pixel, within the anti-aliasing of a glyph placed
    // at another fractional x.
    let img_right = {
        let n = &doc.tree.nodes[i.0];
        let (ax, _) = rinch_dom::paint::compute_absolute_position(&doc.tree, i.0, scale);
        (ax + f64::from(n.layout.width) * scale).floor() as u32
    };
    let mut differ = 0;
    for yy in row_a[1]..=row_a[3] {
        for xx in row_a[0]..img_right {
            let (pa, pb) = (px(&p, xx, yy), px(&p, xx, yy + s(50)));
            if pa.iter().zip(pb).any(|(u, v)| u.abs_diff(v) > 40) {
                differ += 1;
            }
        }
    }
    assert_eq!(differ, 0, "pixels the image's line draws differently");
}

#[test]
fn alt_without_a_size_is_drawn_inline_as_text() {
    inline_matches_text(1.0);
}

#[test]
fn alt_without_a_size_is_drawn_inline_as_text_at_2x() {
    inline_matches_text(2.0);
}

#[test]
fn a_failed_unsized_img_with_alt_is_inline_text_too() {
    let mut doc = new_doc("");
    let i = img(&mut doc, &[("src", BROKEN), ("alt", "Hello")]);
    let p = paint(&mut doc);
    assert!(size(&doc, i).0 > 0.0);
    assert!(count(&p, ink) > 0, "its text");
    assert_eq!(count(&p, |c| c == SILVER), 0, "no box");
}

/// Joe's exception to Chrome's 16px broken-image icon: a tracking pixel,
/// or an image with neither a size nor alt, draws nothing and takes only
/// its declared size.
#[test]
fn tracking_pixels_and_bare_images_draw_nothing() {
    for (attrs, want) in [
        (&[("width", "1"), ("height", "1")][..], (1.0, 1.0)),
        (
            &[("width", "1"), ("height", "1"), ("src", BROKEN)][..],
            (1.0, 1.0),
        ),
        (
            &[("style", "width: 1px; height: 1px"), ("alt", "pixel")][..],
            (1.0, 1.0),
        ),
        (
            &[("width", "0"), ("height", "0"), ("alt", "pixel")][..],
            (0.0, 0.0),
        ),
        (&[][..], (0.0, 0.0)),
        (&[("src", BROKEN)][..], (0.0, 0.0)),
        (&[("alt", "")][..], (0.0, 0.0)),
        (&[("alt", "   ")][..], (0.0, 0.0)),
    ] {
        let mut doc = new_doc("");
        let i = img(&mut doc, attrs);
        let p = paint(&mut doc);
        assert_eq!(size(&doc, i), want, "{attrs:?}");
        assert_eq!(non_white(&p), 0, "{attrs:?} draws nothing");
    }
}

/// The escape hatch: `--rinch-img-placeholder: none` keeps the behaviour
/// from before #2309, nothing drawn and an alt-only image 0x0.
#[test]
fn placeholders_can_be_turned_off_from_css() {
    let mut doc = new_doc(":root { --rinch-img-placeholder: none }");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hi"),
        ],
    );
    let i = img(&mut doc, &[("alt", "Hello")]);
    let p = paint(&mut doc);
    assert_eq!(non_white(&p), 0);
    assert_eq!(size(&doc, i), (0.0, 0.0));
}

/// ...and back on for one image, `auto` beating the inherited `none`.
#[test]
fn the_escape_hatch_is_per_element() {
    let mut doc =
        new_doc(":root { --rinch-img-placeholder: none } .on { --rinch-img-placeholder: auto }");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("class", "on"),
            ("width", "100"),
            ("height", "50"),
        ],
    );
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "turned back on");
}

/// The look is the app's: border colour and width, background.
#[test]
fn the_box_takes_its_look_from_custom_properties() {
    let mut doc = new_doc(
        "img { --rinch-img-placeholder-border-color: #f00; \
               --rinch-img-placeholder-border-width: 3px; \
               --rinch-img-placeholder-background: rgb(0, 0, 255); }",
    );
    img(
        &mut doc,
        &[("style", POS), ("width", "100"), ("height", "50")],
    );
    let p = paint(&mut doc);
    for (x, y) in [(10, 10), (12, 12), (109, 59), (107, 57)] {
        assert_eq!(px(&p, x, y), [255, 0, 0], "the 3px border at ({x}, {y})");
    }
    for (x, y) in [(13, 13), (60, 35), (106, 56)] {
        assert_eq!(px(&p, x, y), [0, 0, 255], "the background at ({x}, {y})");
    }
    assert_eq!(px(&p, 9, 9), WHITE);
}

/// A border width of 0 draws no ring, and the alt text takes the element's
/// own colour.
#[test]
fn no_border_and_the_alt_text_in_the_elements_colour() {
    let mut doc = new_doc("img { --rinch-img-placeholder-border-width: 0; color: #f00 }");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(count(&p, |c| c == SILVER), 0, "no ring");
    let red = |c: [u8; 3]| c[0] > 200 && c[1] < 120 && c[2] < 120;
    assert!(count(&p, red) > 10, "red alt text");
    assert_eq!(count(&p, ink), 0, "no black text");
}

/// `max-width: 100%; height: auto` keeps the attributes' ratio, so the
/// layout does not move when the picture arrives (#684's hint).
#[test]
fn a_sized_placeholder_keeps_its_ratio_under_max_width() {
    let mut doc = new_doc(".c { position: absolute; left: 10px; top: 10px; width: 200px }");
    let body = doc.body();
    let c = el(&mut doc, body, "div", &[("class", "c")]);
    let i = el(
        &mut doc,
        c,
        "img",
        &[
            ("width", "400"),
            ("height", "200"),
            ("style", "max-width: 100%; height: auto; display: block"),
            ("alt", "Hello"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(size(&doc, i), (200.0, 100.0));
    assert_box(&p, [10, 10, 200, 100], "scaled");
    // The picture arriving keeps the box.
    doc.set_attribute(i, "src", RED_3X2);
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    assert_eq!(size(&doc, i), (200.0, 133.0), "the picture's own 3:2");
}

#[test]
fn the_box_at_2x() {
    let mut doc = new_doc("");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hi"),
        ],
    );
    let p = paint_at(&mut doc, 2.0);
    for (x, y) in [(20, 20), (21, 21), (219, 119), (218, 118)] {
        assert_eq!(px(&p, x, y), SILVER, "the 2px device border at ({x}, {y})");
    }
    assert_eq!(px(&p, 22, 22), WHITE, "padding");
    assert_eq!(px(&p, 19, 19), WHITE, "outside");
}

/// A `src` written later replaces the placeholder with the picture, and a
/// partial repaint of the damage draws what a full repaint does: no ring or
/// alt text is left behind.
#[test]
fn a_src_that_arrives_later_replaces_the_placeholder() {
    let mut doc = new_doc("");
    let i = img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello world"),
        ],
    );
    let mut p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "before");
    doc.tree.consume_paint_dirty();
    doc.set_attribute(i, "src", RED_3X2);
    doc.resolve_layout(VP.0, VP.1);
    let damage = rinch_dom::paint::compute_damage(&doc.tree, 1.0, VP.0 as f64, VP.1 as f64);
    assert!(!damage.is_full(), "precondition: a partial repaint");
    let rects = damage.rects().to_vec();
    for r in &rects {
        let r = r.intersect(Rect::new(0.0, 0.0, VP.0 as f64, VP.1 as f64));
        if r.width() <= 0.0 || r.height() <= 0.0 {
            continue;
        }
        p.clear_rect_white(
            r.x0 as u32,
            r.y0 as u32,
            r.width().ceil() as u32,
            r.height().ceil() as u32,
        );
    }
    rinch_dom::paint::set_dirty_rects(Some(&rects));
    for r in &rects {
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(*r));
        let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&doc.tree, &mut p, 1.0, VP, &mut doc.font_cx, &mut cx);
        p.pop_layer();
    }
    rinch_dom::paint::set_dirty_region(None);
    let partial = p.pixels().to_vec();
    doc.tree.consume_paint_dirty();
    let full = paint(&mut doc);
    assert_eq!(
        count(&full, |c| c == SILVER),
        0,
        "no ring under the picture"
    );
    assert_eq!(count(&full, ink), 0, "no alt text under the picture");
    assert_eq!(px(&full, 60, 35), [255, 0, 0], "the picture");
    let differing = partial
        .chunks(4)
        .zip(full.pixels().chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differing, 0, "pixels the partial repaint got wrong");
}

/// Removing the `src` of a loaded picture takes its natural size away and
/// brings the placeholder back.
#[test]
fn removing_a_loaded_src_brings_the_placeholder_back() {
    let mut doc = new_doc("");
    let i = img(&mut doc, &[("alt", "Hello"), ("src", RED_3X2)]);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (3.0, 2.0));
    doc.remove_attribute(i, "src");
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    let (w, h) = size(&doc, i);
    assert!(w > 3.0 && h > 2.0, "its alt text's box: {w}x{h}");
    let p = paint(&mut doc);
    assert!(count(&p, ink) > 0, "and its text");
    // An empty `src` is no picture either.
    doc.set_attribute(i, "src", RED_3X2);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (3.0, 2.0));
    doc.set_attribute(i, "src", "");
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    assert_eq!(size(&doc, i), (w, h));
}

/// The alt text is re-measured when it changes, and dropping it leaves an
/// unsized image with nothing to show.
#[test]
fn an_alt_write_re_measures_the_inline_placeholder() {
    let mut doc = new_doc("");
    let i = img(&mut doc, &[("alt", "Hi")]);
    doc.resolve_layout(VP.0, VP.1);
    let short = size(&doc, i).0;
    doc.set_attribute(i, "alt", "Hi there, a longer text");
    doc.resolve_layout(VP.0, VP.1);
    let long = size(&doc, i).0;
    assert!(long > short * 3.0, "{short} -> {long}");
    doc.remove_attribute(i, "alt");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (0.0, 0.0));
}

/// A size written later turns an inline placeholder into a box.
#[test]
fn a_style_size_turns_the_inline_placeholder_into_a_box() {
    let mut doc = new_doc("");
    let i = img(&mut doc, &[("alt", "Hello"), ("style", POS)]);
    doc.resolve_layout(VP.0, VP.1);
    assert!(size(&doc, i).0 > 0.0);
    doc.set_attribute(i, "style", &format!("{POS}; width: 100px; height: 50px"));
    let p = paint(&mut doc);
    assert_eq!(size(&doc, i), (100.0, 50.0));
    assert_box(&p, [10, 10, 100, 50], "now sized");
}

// ---- Review of PR #2313 -------------------------------------------------

/// Chrome 156 draws a box only when its content box is at least 3px on each
/// axis with no `src` and 18px after a failed load (measured on a grid of
/// `WxH` images: a ring at 3x100 and 300x3, nothing at 2x100 or 300x2; after
/// a failed load nothing at 17x100 or 300x17, the ring at 18). A held-back
/// spacer or divider is not a silver line.
#[test]
fn thin_spacers_and_dividers_draw_nothing() {
    for (w, h, src, drawn) in [
        ("1", "20", None, false),
        ("2", "20", None, false),
        ("2", "100", None, false),
        ("280", "1", None, false),
        ("280", "2", None, false),
        ("3", "100", None, true),
        ("280", "3", None, true),
        ("17", "100", Some(BROKEN), false),
        ("280", "17", Some(BROKEN), false),
        ("40", "17", Some(BROKEN), false),
        ("18", "100", Some(BROKEN), true),
        ("280", "18", Some(BROKEN), true),
    ] {
        let mut doc = new_doc("");
        let mut attrs = vec![("style", POS), ("width", w), ("height", h)];
        if let Some(src) = src {
            attrs.push(("src", src));
        }
        img(&mut doc, &attrs);
        let p = paint(&mut doc);
        let silver = count(&p, |c| c == SILVER);
        assert_eq!(
            silver > 0,
            drawn,
            "{w}x{h} src={src:?}: {silver} silver pixels"
        );
        if !drawn {
            assert_eq!(non_white(&p), 0, "{w}x{h} src={src:?} draws nothing");
        }
    }
}

/// The reviewer's mail case: spacers laid out in a table.
#[test]
fn spacers_in_a_table_draw_nothing() {
    let mut doc = new_doc(
        "table { border-collapse: collapse; position: absolute; left: 10px; top: 10px } \
         td { padding: 0 }",
    );
    let body = doc.body();
    let t = el(&mut doc, body, "table", &[]);
    let tb = el(&mut doc, t, "tbody", &[]);
    let tr = el(&mut doc, tb, "tr", &[]);
    let td = el(&mut doc, tr, "td", &[]);
    let a = el(
        &mut doc,
        td,
        "img",
        &[("width", "1"), ("height", "20"), ("style", "display:block")],
    );
    let td2 = el(&mut doc, tr, "td", &[]);
    let b = el(&mut doc, td2, "img", &[("width", "200"), ("height", "1")]);
    let p = paint(&mut doc);
    assert_eq!((size(&doc, a), size(&doc, b)), ((1.0, 20.0), (200.0, 1.0)));
    assert_eq!(non_white(&p), 0);
}

/// The width an inline-block `<span>` of `text` is given in `doc` (the
/// reference for the alt text's box), and its height.
fn span_box(doc: &mut RinchDocument, text: &str, style: &str) -> (f32, f32) {
    let body = doc.body();
    let s = el(
        doc,
        body,
        "span",
        &[(
            "style",
            &format!("position: absolute; top: 120px; left: 0; display: inline-block; {style}"),
        )],
    );
    let t = doc.create_text(text);
    doc.append_child(s, t);
    doc.resolve_layout(VP.0 + 3.0, VP.1);
    size(doc, s)
}

/// An image with alt text and only one dimension is laid out as Chrome lays
/// it out: an inline-block of its text, its declared dimension kept, the
/// other its text's (Chrome 156: `width=600 alt=Banner style="display:block;
/// width:100%; height:auto"` in a 150px column is 150x20).
#[test]
fn alt_with_one_dimension_takes_the_other_from_its_text() {
    let mut doc = new_doc(".c { position: absolute; left: 10px; top: 10px; width: 150px }");
    let body = doc.body();
    let c = el(&mut doc, body, "div", &[("class", "c")]);
    let banner = el(
        &mut doc,
        c,
        "img",
        &[
            ("width", "600"),
            ("alt", "Banner"),
            ("style", "display:block;width:100%;height:auto"),
        ],
    );
    let w = img(
        &mut doc,
        &[
            ("style", "position:absolute;left:10px;top:40px"),
            ("width", "100"),
            ("alt", "Hello world"),
        ],
    );
    let h = img(
        &mut doc,
        &[
            ("style", "position:absolute;left:10px;top:70px"),
            ("height", "40"),
            ("alt", "Hello world"),
        ],
    );
    let p = paint(&mut doc);
    assert_eq!(size(&doc, banner), (150.0, 20.0), "Chrome's 150x20");
    assert_eq!(size(&doc, w), (100.0, 20.0));
    let (tw, _) = span_box(&mut doc, "Hello world", "");
    assert_eq!(size(&doc, h), (tw, 40.0), "its text's width");
    let p_ink = |area| bbox(&p, area, ink).is_some();
    assert!(p_ink([10, 10, 160, 30]), "the banner's alt text");
    assert!(p_ink([10, 40, 110, 60]), "the width-only one's");
    assert!(
        p_ink([10, 70, 10 + tw as u32, 110]),
        "the height-only one's"
    );
}

/// A long alt text wraps like text: in an 80px column it is an 80px-wide
/// inline-block of its lines, on a line of its own after text before it
/// (Chrome 156: 80x160 for this text, its parent 20 + 160 with "xx " first).
#[test]
fn a_long_inline_alt_text_wraps_in_its_column() {
    let alt = "A very long alt text that will not fit in this narrow column at all";
    let mut doc = new_doc(".c { position: absolute; left: 10px; top: 0; width: 80px }");
    let body = doc.body();
    let c = el(&mut doc, body, "div", &[("class", "c")]);
    let i = el(&mut doc, c, "img", &[("alt", alt)]);
    let d = el(
        &mut doc,
        body,
        "div",
        &[(
            "style",
            "position: absolute; left: 150px; top: 0; width: 80px",
        )],
    );
    let xx = doc.create_text("xx ");
    doc.append_child(d, xx);
    let j = el(&mut doc, d, "img", &[("alt", alt)]);
    let p = paint(&mut doc);
    let (w, h) = size(&doc, i);
    assert_eq!(w, 80.0, "as wide as its column");
    assert!(h >= 100.0 && h % 20.0 == 0.0, "lines of 20px: {h}");
    let ink_box = bbox(&p, [0, 0, 140, H], ink).expect("its text");
    assert!(
        ink_box[0] >= 10 && ink_box[2] < 90,
        "inside its column: {ink_box:?}"
    );
    assert!(ink_box[3] > 60, "on several lines: {ink_box:?}");
    assert_eq!(size(&doc, j), (w, h));
    let dn = &doc.tree.nodes[d.0];
    assert_eq!(dn.layout.height, 20.0 + h, "after the line of \"xx \"");
}

/// Switching a loaded `src` to one that already failed, or one still
/// loading, drops the picture's size; and back to the picture restores it.
#[test]
fn switching_from_a_loaded_src_to_a_failed_or_loading_one_drops_its_size() {
    let mut doc = new_doc("");
    let _failed = img(&mut doc, &[("src", BROKEN)]);
    let i = img(&mut doc, &[("alt", "Hello"), ("src", RED_3X2)]);
    let j = img(&mut doc, &[("src", RED_3X2), ("width", "30")]);
    doc.resolve_layout(VP.0, VP.1);
    assert!(doc.tree.image_cache.is_failed(BROKEN));
    assert_eq!((size(&doc, i), size(&doc, j)), ((3.0, 2.0), (30.0, 20.0)));
    doc.set_attribute(i, "src", BROKEN);
    doc.set_attribute(j, "src", BROKEN);
    doc.resolve_layout(VP.0, VP.1);
    let (tw, th) = span_box(&mut doc, "Hello", "");
    assert_eq!(
        size(&doc, i),
        (tw, th),
        "its alt text's box, as a fresh failed one"
    );
    assert_eq!(size(&doc, j), (30.0, 0.0), "as a fresh failed one");
    // Back to the picture.
    doc.set_attribute(i, "src", RED_3X2);
    doc.set_attribute(j, "src", RED_3X2);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!((size(&doc, i), size(&doc, j)), ((3.0, 2.0), (30.0, 20.0)));
    // And to a source still loading (another element asked for it first).
    struct Slow;
    impl ImageLoader for Slow {
        fn load(&self, _: &str) -> ImageLoadResult {
            std::thread::sleep(Duration::from_secs(2));
            ImageLoadResult::Failed("slow".into())
        }
    }
    doc.tree.image_loader = Some(Arc::new(Slow));
    let _first = img(&mut doc, &[("src", "slow-2313.png")]);
    doc.set_attribute(j, "src", "slow-2313.png");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        size(&doc, j),
        (30.0, 0.0),
        "a loading source has no size yet"
    );
}

/// Reloading a failed source keeps its placeholder up until the answer
/// lands, so no stale or blank frame is drawn; a second failure keeps it.
#[test]
fn reloading_a_failed_source_keeps_its_placeholder() {
    // The alt text in the platform's own `serif`/`sans-serif`, as before
    // #2255: with Chrome's Times New Roman and Arial heading them (a host
    // that has them) its ink reaches the padding pixel probed. CI has neither.
    rinch_dom::fonts::set_font_defaults(
        rinch_dom::fonts::FontDefaults::default()
            .with_serif(None)
            .with_sans_serif(None),
    );
    struct AlwaysFails;
    impl ImageLoader for AlwaysFails {
        fn load(&self, src: &str) -> ImageLoadResult {
            std::thread::sleep(Duration::from_millis(30));
            ImageLoadResult::Failed(format!("{src}: no"))
        }
    }
    let mut doc = new_doc("");
    doc.tree.image_loader = Some(Arc::new(AlwaysFails));
    let i = img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello"),
            ("src", "r-2313.png"),
        ],
    );
    let j = img(&mut doc, &[("alt", "Inline"), ("src", "r-2313.png")]);
    let wait = |doc: &mut RinchDocument| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !doc.tree.image_cache.is_failed("r-2313.png") {
            assert!(Instant::now() < deadline, "never failed");
            std::thread::sleep(Duration::from_millis(5));
            doc.resolve_layout(VP.0, VP.1);
        }
    };
    wait(&mut doc);
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "failed");
    let inline = size(&doc, j);
    assert!(inline.0 > 0.0);
    doc.reload_image("r-2313.png");
    doc.resolve_layout(VP.0, VP.1);
    assert!(!doc.tree.image_cache.is_failed("r-2313.png"), "reloading");
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "while it reloads");
    assert_eq!(size(&doc, j), inline, "the inline one keeps its box");
    doc.tree.consume_paint_dirty();
    wait(&mut doc);
    assert!(
        doc.tree.paint_dirty_nodes.contains(&i.0),
        "the answer repaints it"
    );
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "failed again");
}

/// Turning placeholders off from CSS repaints a box (its size is unchanged)
/// and collapses an inline one; turning them on brings both back.
#[test]
fn toggling_the_escape_hatch_repaints() {
    let mut doc = new_doc(".off { --rinch-img-placeholder: none }");
    let i = img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hi"),
        ],
    );
    let j = img(
        &mut doc,
        &[
            ("alt", "Hello"),
            ("style", "position:absolute;left:10px;top:100px"),
        ],
    );
    let p = paint(&mut doc);
    assert_box(&p, [10, 10, 100, 50], "on");
    let inline = size(&doc, j);
    doc.tree.consume_paint_dirty();
    let body = doc.body();
    doc.set_attribute(body, "class", "off");
    doc.resolve_layout(VP.0, VP.1);
    assert!(
        doc.tree.paint_dirty_nodes.contains(&i.0),
        "the box repaints"
    );
    assert_eq!(size(&doc, j), (0.0, 0.0));
    let p = paint(&mut doc);
    assert_eq!(non_white(&p), 0);
    doc.set_attribute(body, "class", "");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, j), inline);
}

/// The border width takes any absolute length (and `em`).
#[test]
fn the_border_width_takes_absolute_lengths() {
    for (value, px_w) in [("0.125in", 12u32), ("6pt", 8), ("0.5em", 8), ("3px", 3)] {
        let mut doc = new_doc(&format!(
            "img {{ --rinch-img-placeholder-border-width: {value} }}"
        ));
        img(
            &mut doc,
            &[("style", POS), ("width", "100"), ("height", "50")],
        );
        let p = paint(&mut doc);
        assert_eq!(
            px(&p, 10 + px_w - 1, 35),
            SILVER,
            "{value}: its last column"
        );
        assert_eq!(px(&p, 10 + px_w, 35), WHITE, "{value}: inside it");
    }
}

/// The alt text is shaped when the element is synced, not on every paint.
#[test]
fn the_alt_text_is_not_shaped_per_paint() {
    let mut doc = new_doc("");
    img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "100"),
            ("height", "50"),
            ("alt", "Hello world"),
        ],
    );
    let body = doc.body();
    let d = el(
        &mut doc,
        body,
        "div",
        &[("style", "position:absolute;top:80px")],
    );
    el(&mut doc, d, "img", &[("alt", "Hello")]);
    let _ = paint(&mut doc);
    let before = doc.tree.perf.get(rinch_dom::perf::Counter::ShapePaint);
    for _ in 0..3 {
        let p = paint(&mut doc);
        assert!(count(&p, ink) > 0);
    }
    assert_eq!(
        doc.tree.perf.get(rinch_dom::perf::Counter::ShapePaint),
        before,
        "three paints shaped nothing"
    );
}

/// An alt text that wraps in its declared width sits on its **last** line's
/// baseline, as an inline-block does: the text beside it lines up with its
/// bottom line, not its top one.
#[test]
fn a_wrapped_alt_box_sits_on_its_last_line() {
    let mut doc = new_doc(".c { position: absolute; left: 10px; top: 10px; width: 280px }");
    let body = doc.body();
    let c = el(&mut doc, body, "div", &[("class", "c")]);
    let x = doc.create_text("x");
    doc.append_child(c, x);
    let i = el(
        &mut doc,
        c,
        "img",
        &[("width", "40"), ("alt", "Hi Hi Hi Hi")],
    );
    let p = paint(&mut doc);
    let (_, h) = size(&doc, i);
    assert!(h >= 40.0, "it wraps: {h}");
    // The "x" before it, left of the image.
    let x_ink = bbox(&p, [10, 0, 17, H], ink).expect("the x");
    let line_top = 10.0 + h - 20.0;
    assert!(
        x_ink[1] as f32 >= line_top,
        "the x {x_ink:?} is on the image's last line (from {line_top})"
    );
}

/// A box too small for its placeholder that has alt text shows the text
/// inline, its size ignored, as Chrome 156 shows a failed 10x10
/// `alt="Facebook"` icon; at the threshold it is the box again. A picture
/// arriving gives it its declared size back.
#[test]
fn a_box_below_the_threshold_with_alt_shows_the_alt_inline() {
    let mut doc = new_doc("");
    let (tw, th) = span_box(&mut doc, "Facebook", "");
    for (w, h, src, inline) in [
        ("10", "10", Some(BROKEN), true),
        ("17", "17", Some(BROKEN), true),
        ("40", "17", Some(BROKEN), true),
        ("18", "18", Some(BROKEN), false),
        ("2", "20", None, true),
        ("3", "20", None, false),
    ] {
        let mut doc = new_doc("");
        let mut attrs = vec![
            ("style", POS),
            ("width", w),
            ("height", h),
            ("alt", "Facebook"),
        ];
        if let Some(src) = src {
            attrs.push(("src", src));
        }
        let i = img(&mut doc, &attrs);
        let p = paint(&mut doc);
        let what = format!("{w}x{h} src={src:?}");
        if inline {
            assert_eq!(size(&doc, i), (tw, th), "{what}: the text's box");
            assert!(count(&p, ink) > 0, "{what}: its text");
            // No ring (even the smallest, 10x10, is 36 pixels); an
            // anti-aliased glyph edge can land on silver's value.
            assert!(count(&p, |c| c == SILVER) < 8, "{what}: no box");
        } else {
            let (ww, hh) = (w.parse::<f32>().unwrap(), h.parse::<f32>().unwrap());
            assert_eq!(size(&doc, i), (ww, hh), "{what}: its declared box");
            assert!(count(&p, |c| c == SILVER) > 0, "{what}: the box");
        }
    }
    // The picture arrives: its declared size again.
    let mut doc = new_doc("");
    let i = img(
        &mut doc,
        &[
            ("style", POS),
            ("width", "10"),
            ("height", "10"),
            ("alt", "Facebook"),
            ("src", BROKEN),
        ],
    );
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (tw, th));
    doc.set_attribute(i, "src", RED_3X2);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (10.0, 10.0));
    doc.set_attribute(i, "src", BROKEN);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(size(&doc, i), (tw, th), "and failed again");
}
