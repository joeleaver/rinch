//! The scenes and expected pixels of four paint issues, shared by the
//! software painter's `paint_decorations_2396_tests` and the GPU painter's
//! `gpu_paint_decorations_2396` (`rinch/tests`, through `#[path]`):
//!
//! - #2335: an `<img>` draws its outer and inset `box-shadow`, its
//!   `outline` and a `background-image`, as Chrome does.
//! - #2375: a rounded border in four colours covers each pixel exactly as
//!   the same border in one colour, its corners' diagonals included.
//! - #2396: an inline element's dashed, dotted or groove border has its
//!   style, laid out as Chrome lays it out.
//! - #2373: a line-through (and an underline) sits on whole device rows.
//!
//! Every scene is laid out at its own `W` x `H` CSS px and painted at a
//! scale by the caller; each check takes the RGBA pixels painted (over
//! white) and returns what is wrong.

#![allow(dead_code)]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;

/// A pixel's mean channel.
fn lum(p: [u8; 4]) -> u8 {
    ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
}

fn page(css: &str, body: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let b = doc.body();
    doc.set_inner_html(b, body);
    doc
}

// ── #2335: an <img>'s shadows, outline and background image ────────────────

pub mod img {
    use super::*;

    pub const W: u32 = 240;
    pub const H: u32 = 60;

    const CSS: &str = "html, body { margin: 0; background: #fff; }
img { position: absolute; display: block; image-rendering: pixelated;
      width: 20px; height: 20px; }";

    /// The style of each `<img>`.
    pub const BOXES: &[&str] = &[
        "left: 10px; top: 10px; box-shadow: 7px 5px 0 rgb(0, 0, 255)",
        "left: 60px; top: 10px; outline: 3px solid rgb(0, 128, 0); outline-offset: 2px",
        "left: 110px; top: 10px; padding: 5px; \
         background-image: linear-gradient(rgb(255, 0, 255), rgb(255, 0, 255))",
        "left: 160px; top: 10px; padding: 5px; background: rgb(255, 255, 0); \
         box-shadow: inset 0 0 0 3px rgb(0, 0, 255)",
    ];

    /// `(x, y, what, Chrome's colour)` at 1x: Chrome 156,
    /// `--force-device-scale-factor=1`, on the page `BOXES` make.
    const CHROME: &[(u32, u32, &str, [u8; 3])] = &[
        (31, 30, "the outer shadow", [0, 0, 255]),
        (36, 34, "the outer shadow", [0, 0, 255]),
        (10, 10, "the picture", [255, 0, 0]),
        (29, 29, "the picture", [255, 255, 255]),
        (56, 20, "the outline", [0, 128, 0]),
        (57, 20, "the outline", [0, 128, 0]),
        (59, 20, "the outline offset", [255, 255, 255]),
        (60, 20, "the picture", [0, 0, 255]),
        (110, 10, "the background image", [255, 0, 255]),
        (114, 14, "the background image", [255, 0, 255]),
        (115, 15, "the picture", [255, 0, 0]),
        (160, 10, "the inset shadow", [0, 0, 255]),
        (162, 12, "the inset shadow", [0, 0, 255]),
        (164, 20, "the background", [255, 255, 0]),
        (166, 20, "the picture over the inset shadow", [255, 0, 0]),
    ];

    /// `q.png`: 2x2, red green / blue white.
    fn q_png() -> DecodedImage {
        DecodedImage::new(
            [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
            ]
            .concat(),
            2,
            2,
        )
    }

    /// One `<img src="q.png">` per style, laid out.
    pub fn document(styles: &[&str]) -> (RinchDocument, Vec<rinch_core::dom::NodeId>) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        doc.tree
            .image_cache
            .insert_decoded("q.png".to_string(), q_png());
        let body = doc.body();
        let mut ids = Vec::new();
        for style in styles {
            let n = doc.create_element("img");
            doc.set_attribute(n, "src", "q.png");
            doc.set_attribute(n, "style", style);
            doc.append_child(body, n);
            ids.push(n);
        }
        doc.resolve_layout(W as f32, H as f32);
        (doc, ids)
    }

    /// Where `px` (`BOXES` at 1x) is not Chrome's colour.
    pub fn misses(px: &[[u8; 4]]) -> Vec<String> {
        CHROME
            .iter()
            .filter_map(|&(x, y, what, chrome)| {
                let [r, g, b, _] = px[(y * W + x) as usize];
                ([r, g, b] != chrome).then(|| {
                    format!(
                        "({x}, {y}) {what}: rinch {:?}, chrome {chrome:?}",
                        [r, g, b]
                    )
                })
            })
            .collect()
    }
}

// ── #2375: a rounded border's sides in four colours ────────────────────────

pub mod sides {
    use super::*;

    pub const W: u32 = 200;
    pub const H: u32 = 100;

    /// The issue's box, and two more: per-corner radii, and elliptical ones
    /// on a thicker border. Every edge is at an odd CSS px.
    const BOXES: &[&str] = &[
        "left: 11px; top: 45px; width: 34px; height: 24px; border: 3px solid; border-radius: 8px",
        "left: 61px; top: 9px; width: 50px; height: 34px; border: 5px solid; \
         border-radius: 15px 9px 21px 3px",
        "left: 131px; top: 13px; width: 41px; height: 51px; border: 7px solid; \
         border-radius: 25px 5px 13px 31px / 19px 7px 25px 11px",
    ];

    /// The boxes, with `border-color: colours`.
    pub fn document(colours: &str) -> RinchDocument {
        let body: String = BOXES
            .iter()
            .map(|s| format!(r#"<div style="{s}; border-color: {colours}"></div>"#))
            .collect();
        let mut doc = page(
            "html, body { margin: 0; background: #fff; }
             div { position: absolute; }",
            &body,
        );
        doc.resolve_layout(W as f32, H as f32);
        doc
    }

    /// The one-colour border, and the four-colour one: none of the colours
    /// has any red, so the red channel is the coverage.
    pub const ONE: &str = "#000";
    pub const FOUR: &str = "#000 #00f #0f0 #0ff";

    /// The most a pixel may differ in tiny-skia: three of its 16 coverage
    /// samples. A corner's piece is cut where the one-colour ring is not, and
    /// tiny-skia flattens each cut curve on its own (measured: up to 48
    /// levels). The four-colour border before #2375 was 60 to 255 off.
    pub const SOFTWARE_TOL: u8 = 48;
    /// The same on the GPU painter, which flattens a whole circular arc as
    /// a circle and a cut one as a cubic, both to 0.03 px (measured: up to
    /// 11 levels, at 2x). The strokes before #2375 were up to 48 off.
    pub const GPU_TOL: u8 = 16;

    /// Where the red channels of `one` and `four` (each `pw` wide) differ
    /// by more than `tol`.
    pub fn misses(one: &[[u8; 4]], four: &[[u8; 4]], pw: u32, tol: u8) -> Vec<String> {
        let mut out: Vec<String> = one
            .iter()
            .zip(four)
            .enumerate()
            .filter(|(_, (a, b))| a[0].abs_diff(b[0]) > tol)
            .map(|(i, (a, b))| {
                format!(
                    "({}, {}): one colour {}, four {}",
                    i as u32 % pw,
                    i as u32 / pw,
                    a[0],
                    b[0]
                )
            })
            .collect();
        // A positive control: the ring is there.
        let solid = one.iter().filter(|p| p[0] == 0).count();
        if solid < 300 {
            out.push(format!("only {solid} solid ring pixels"));
        }
        out
    }
}

// ── #2396: an inline element's border style ─────────────────────────────────

pub mod inline {
    use super::*;

    pub const W: u32 = 240;
    pub const H: u32 = 160;

    /// Chrome 156's page. Each inline element holds only a fixed-size
    /// inline-block (#2597; it held a zero-width space as well before that
    /// drew), so its width is the page's, not the font's; its height is the
    /// font's content area, so each border's top row is found by scanning
    /// rather than assumed.
    pub fn document() -> RinchDocument {
        let mut doc = page(
            "html, body { margin: 0; background: #fff; }
             div { position: absolute; font: 16px/40px sans-serif; }
             i { display: inline-block; width: 101px; height: 10px; }",
            r#"
<div style="left: 10px; top: 0"><span style="border: 2px dashed #000; padding: 0 3px"><i></i></span></div>
<div style="left: 10px; top: 50px"><span style="border: 3px dotted #000"><i></i></span></div>
<div style="left: 10px; top: 100px"><span style="border: 4px groove rgb(200, 100, 50)"><i></i></span></div>
"#,
        );
        doc.resolve_layout(W as f32, H as f32);
        doc
    }

    /// Chrome's first row of the dashed and dotted borders' top sides, from
    /// x = 5 (1x), as mean channels.
    const DASHED_TOP: [u8; 125] = [
        255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 255, 255, 255, 138, 0, 0, 0, 0, 0, 116, 255,
        255, 255, 22, 0, 0, 0, 0, 0, 232, 255, 255, 161, 0, 0, 0, 0, 0, 93, 255, 255, 255, 45, 0,
        0, 0, 0, 0, 209, 255, 255, 185, 0, 0, 0, 0, 0, 69, 255, 255, 255, 68, 0, 0, 0, 0, 0, 186,
        255, 255, 208, 0, 0, 0, 0, 0, 46, 255, 255, 255, 92, 0, 0, 0, 0, 0, 162, 255, 255, 231, 0,
        0, 0, 0, 0, 23, 255, 255, 255, 115, 0, 0, 0, 0, 0, 139, 255, 255, 255, 0, 0, 0, 0, 0, 0,
        255, 255, 255, 255, 255, 255, 255, 255, 255,
    ];
    const DOTTED_TOP: [u8; 125] = [
        255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255,
        255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0,
        255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0,
        0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255,
        255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
    ];
    /// The most a dash end may differ from Chrome's: the software painter's
    /// coverage comes in quarters of a pixel (64 levels).
    const DASH_TOL: u8 = 70;
    /// Chrome's groove, down through its top side at x = 60 (its outer half
    /// darkened, its inner half lightened), then the content.
    const GROOVE_DOWN: [[u8; 3]; 5] = [
        [116, 58, 29],
        [116, 58, 29],
        [255, 127, 63],
        [255, 127, 63],
        [255, 255, 255],
    ];

    fn at(px: &[[u8; 4]], x: u32, y: u32) -> u8 {
        lum(px[(y * W + x) as usize])
    }

    /// The first row in column `x` from `y0` that is not white: a border's
    /// top side.
    fn top_row(px: &[[u8; 4]], x: u32, y0: u32) -> Option<u32> {
        (y0..y0 + 50).find(|&y| at(px, x, y) < 200)
    }

    /// Where the row from `(5, y)` is off `chrome` by more than `DASH_TOL`.
    fn compare(px: &[[u8; 4]], what: &str, chrome: &[u8; 125], y: u32) -> Option<String> {
        let bad: Vec<(u32, u8, u8)> = (0..125)
            .map(|i| (i + 5, chrome[i as usize], at(px, i + 5, y)))
            .filter(|&(_, c, l)| c.abs_diff(l) > DASH_TOL)
            .collect();
        (!bad.is_empty()).then(|| format!("{what}, row {y}: (x, chrome, rinch) {bad:?}"))
    }

    /// Where `px` (the page at 1x) is not Chrome's.
    pub fn misses(px: &[[u8; 4]]) -> Vec<String> {
        let mut out = Vec::new();
        let (Some(dashed), Some(dotted), Some(groove)) = (
            top_row(px, 10, 0),
            top_row(px, 10, 50),
            top_row(px, 60, 100),
        ) else {
            return vec!["a border is missing".into()];
        };
        out.extend(compare(px, "dashed top", &DASHED_TOP, dashed));
        out.extend(compare(px, "dotted top", &DOTTED_TOP, dotted));
        for (i, chrome) in GROOVE_DOWN.iter().enumerate() {
            let y = groove + i as u32;
            let [r, g, b, _] = px[(y * W + 60) as usize];
            if [r, g, b]
                .iter()
                .zip(chrome)
                .any(|(a, c)| a.abs_diff(*c) > 2)
            {
                out.push(format!(
                    "groove at (60, {y}): rinch {:?}, chrome {chrome:?}",
                    [r, g, b]
                ));
            }
        }
        // The dashed left side is dashed too: a gap somewhere down it.
        let bottom = (dashed + 2..dashed + 60)
            .rev()
            .find(|&y| at(px, 10, y) < 100)
            .unwrap_or(dashed);
        if !(dashed..bottom).any(|y| at(px, 10, y) > 200) {
            out.push(format!(
                "the dashed left side is solid from {dashed} to {bottom}"
            ));
        }
        out
    }
}

// ── #2396: a wrapped inline element's open ends ───────────────────────────

pub mod wrapped {
    use super::*;

    pub const W: u32 = 200;
    pub const H: u32 = 100;

    const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

    /// An inline element whose border wraps onto two lines: the first
    /// fragment holds its left edge, the second its right
    /// (`box-decoration-break: slice`), and neither draws a side at the
    /// break. Its text is transparent, so the border is the only ink. Set in
    /// this crate's Inter (Chrome loads the same file through `@font-face`),
    /// so where the text ends is the font's, not the host's.
    pub fn document() -> RinchDocument {
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
        doc.load_css(
            "html, body { margin: 0; background: #fff; }
             div { position: absolute; left: 10px; top: 0; width: 100px;
                   font: 16px/40px ProbeFace; }",
        );
        let b = doc.body();
        doc.set_inner_html(
            b,
            r#"<div><span style="border: 2px solid #000; color: transparent">MMMMM MMMMM</span></div>"#,
        );
        doc.resolve_layout(W as f32, H as f32);
        doc
    }

    fn at(px: &[[u8; 4]], x: u32, y: u32) -> u8 {
        lum(px[(y * W + x) as usize])
    }

    /// The dark runs of row `y`, as `(first x, last x)`.
    fn runs(px: &[[u8; 4]], y: u32) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for x in 0..W {
            if at(px, x, y) < 100 {
                match out.last_mut() {
                    Some(r) if r.1 + 1 == x => r.1 = x,
                    _ => out.push((x, x)),
                }
            }
        }
        out
    }

    /// Chrome 157's dark runs, row by row (1x): each fragment is
    /// `getClientRects`' `[10, 8, 74.27, 24]` and `[10, 48, 74.27, 24]`,
    /// its edges on the nearest pixel (x 10 to 84), the first ending at its
    /// last glyph and not at the space its line ends with (#2607); its top
    /// and bottom sides no longer than it, and its left side on the first
    /// line and its right on the second.
    const CHROME: &[(u32, &[(u32, u32)])] = &[
        (7, &[]),
        (8, &[(10, 83)]),
        (9, &[(10, 83)]),
        (18, &[(10, 11)]),
        (30, &[(10, 83)]),
        (31, &[(10, 83)]),
        (32, &[]),
        (47, &[]),
        (48, &[(10, 83)]),
        (49, &[(10, 83)]),
        (58, &[(82, 83)]),
        (70, &[(10, 83)]),
        (71, &[(10, 83)]),
        (72, &[]),
    ];

    /// Where `px` (the page at 1x) is not Chrome's.
    pub fn misses(px: &[[u8; 4]]) -> Vec<String> {
        CHROME
            .iter()
            .filter_map(|&(y, chrome)| {
                let rinch = runs(px, y);
                (rinch != chrome)
                    .then(|| format!("row {y}: dark runs {rinch:?}, Chrome {chrome:?}"))
            })
            .collect()
    }
}

// ── #2373: decorations on whole device rows ─────────────────────────────────

pub mod decorations {
    use super::*;

    pub const W: u32 = 200;
    pub const H: u32 = 220;

    /// `(font size, top)` per line: fractional tops, so a decoration placed
    /// at its exact position straddles two rows.
    const LINES: &[(u32, f64)] = &[
        (13, 5.3),
        (16, 31.5),
        (17, 59.7),
        (20, 88.25),
        (23, 121.6),
        (29, 160.4),
    ];

    /// Each line's text transparent and its decorations black, so the
    /// decorations are the only ink: a line-through on the left, an
    /// underline on the right.
    pub fn document() -> RinchDocument {
        let body: String = LINES
            .iter()
            .map(|(size, top)| {
                format!(
                    r#"<div style="top: {top}px; font-size: {size}px"><s>MMM</s></div><div style="left: 110px; top: {top}px; font-size: {size}px"><u>MMM</u></div>"#
                )
            })
            .collect();
        let mut doc = page(
            "html, body { margin: 0; background: #fff; }
             div { position: absolute; left: 10px; line-height: 1.5; color: transparent; }
             s, u { text-decoration-color: #000; }",
            &body,
        );
        doc.resolve_layout(W as f32, H as f32);
        doc
    }

    /// Every pixel inked at all should be fully black: a decoration on
    /// whole device rows has no partly covered row. Only the columns well
    /// inside a decoration are read (its ends may be anti-aliased across a
    /// column).
    pub fn misses(px: &[[u8; 4]], pw: u32, scale: f64) -> Vec<String> {
        let ph = px.len() as u32 / pw;
        let mut out = Vec::new();
        let mut rows = 0;
        // Columns four CSS px into each decoration's text.
        let cols = [(14.0 * scale) as u32, (114.0 * scale) as u32];
        for x in cols {
            for y in 0..ph {
                let l = lum(px[(y * pw + x) as usize]);
                if l < 255 {
                    rows += 1;
                    if l > 0 {
                        out.push(format!("({x}, {y}) is {l}, part covered"));
                    }
                }
            }
        }
        if rows < 2 * LINES.len() {
            out.push(format!("only {rows} inked rows"));
        }
        out
    }
}

// ── #2375's cost: 200 cards with a multi-colour rounded border ─────────────

pub mod cards {
    use super::*;

    pub const W: u32 = 800;
    pub const H: u32 = 600;

    /// One coloured side on a grey border (review of #2606).
    pub const ONE_SIDE: &str =
        "border: 2px solid #ccc; border-left-color: #36f; border-radius: 8px";
    /// Four colours, 3px, radius 12.
    pub const FOUR: &str =
        "border: 3px solid; border-color: #c00 #0a0 #00c #888; border-radius: 12px";

    /// 200 cards (20 by 10, 36x56 each with its border) with `border`.
    pub fn document(border: &str) -> RinchDocument {
        let body: String = (0..200)
            .map(|i| {
                let (x, y) = ((i % 20) * 40, (i / 20) * 60);
                format!(r#"<div style="left: {x}px; top: {y}px"></div>"#)
            })
            .collect();
        let mut doc = page(
            &format!(
                "html, body {{ margin: 0; background: #fff; }}
                 div {{ position: absolute; box-sizing: border-box; width: 36px;
                        height: 56px; {border} }}"
            ),
            &body,
        );
        doc.resolve_layout(W as f32, H as f32);
        doc
    }
}
