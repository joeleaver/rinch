//! The #2719 page and its comparison with Chrome, shared by the software
//! painter's tests (`text_clip_stroke_2719_tests.rs`) and the GPU painter's
//! (`crates/rinch/tests/gpu_text_clip_stroke_2719.rs`), which include it with
//! `#[path]`. The header of the first says what the page and the fixtures
//! are and how the comparison works.
#![allow(dead_code)]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
pub const PAGE: &str = include_str!("../fixtures/text_clip_stroke_2719.html");
pub const CHROME_1X: &[u8] = include_bytes!("../fixtures/text_clip_stroke_2719_chrome_1x.png");
pub const CHROME_2X: &[u8] = include_bytes!("../fixtures/text_clip_stroke_2719_chrome_2x.png");
/// The page's size in CSS px.
pub const W: u32 = 600;
pub const H: u32 = 410;

/// The page's style sheet, as rinch loads it (the `@font-face` is the
/// registered face).
pub const CSS: &str = "html, body { margin: 0; background: #fff; } \
     div, span { font-family: InterT; } \
     .a { position: absolute; font-size: 61px; line-height: 70px; white-space: nowrap; }";

/// The fixture's body, without its script.
pub fn page_body() -> String {
    let body = &PAGE[PAGE.find("<body>").unwrap() + 6..PAGE.find("</body>").unwrap()];
    match body.find("<script>") {
        Some(i) => body[..i].to_string() + &body[body.find("</script>").unwrap() + 9..],
        None => body.to_string(),
    }
}

/// A document of `html` under `css`, with the bundled Inter as `InterT`,
/// laid out at `w` x `h`.
pub fn doc_with(css: &str, html: &str, w: f32, h: f32) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    d.load_css(css);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(w, h);
    d
}

/// [`doc_with`] the page's style sheet.
pub fn doc(html: &str, w: f32, h: f32) -> RinchDocument {
    doc_with(CSS, html, w, h)
}

/// An RGB image: pixels row by row, and its size.
pub struct Image {
    pub px: Vec<[u8; 3]>,
    pub w: u32,
    pub h: u32,
}

impl Image {
    /// From RGBA rows (a painter's pixels), dropping alpha.
    pub fn from_rgba(px: &[[u8; 4]], w: u32, h: u32) -> Self {
        Self {
            px: px.iter().map(|p| [p[0], p[1], p[2]]).collect(),
            w,
            h,
        }
    }

    pub fn at(&self, x: u32, y: u32) -> [u8; 3] {
        self.px[(y * self.w + x) as usize]
    }

    /// The 3x3 box-blurred pixel at `(x, y)`, edges clamped.
    fn blurred(&self, x: u32, y: u32) -> [f32; 3] {
        let mut s = [0.0_f32; 3];
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let xx = (x as i32 + dx).clamp(0, self.w as i32 - 1) as u32;
                let yy = (y as i32 + dy).clamp(0, self.h as i32 - 1) as u32;
                let p = self.at(xx, yy);
                for c in 0..3 {
                    s[c] += p[c] as f32;
                }
            }
        }
        s.map(|v| v / 9.0)
    }

    fn blur(&self) -> Vec<[f32; 3]> {
        (0..self.h)
            .flat_map(|y| (0..self.w).map(move |x| (x, y)))
            .map(|(x, y)| self.blurred(x, y))
            .collect()
    }
}

/// How two images differ over a region: the mean of each blurred pixel's
/// largest channel difference, and how many differ by more than 64.
#[derive(Debug, Clone, Copy)]
pub struct Diff {
    pub mean: f32,
    pub over: usize,
    /// The shift of the second image (device px) the region was compared at.
    pub shift: (i32, i32),
}

/// `rect` is in CSS px `(x0, y0, x1, y1)`, scaled by `scale`. The two
/// engines place a line of text up to a pixel or two apart (a rounding of the
/// line's half-leading, not anything this page is about), so `b` is compared
/// at each shift up to two CSS px and the best is kept.
fn diff(
    a: &[[f32; 3]],
    b: &[[f32; 3]],
    w: u32,
    h: u32,
    rect: (u32, u32, u32, u32),
    scale: u32,
) -> Diff {
    let shift = 2 * scale as i32;
    let (x0, y0, x1, y1) = (
        rect.0 * scale,
        rect.1 * scale,
        rect.2 * scale,
        rect.3 * scale,
    );
    let mut best: Option<Diff> = None;
    for sy in -shift..=shift {
        for sx in -shift..=shift {
            let (mut sum, mut over, mut n) = (0.0_f32, 0, 0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (bx, by) = (x as i32 + sx, y as i32 + sy);
                    if bx < 0 || by < 0 || bx >= w as i32 || by >= h as i32 {
                        continue;
                    }
                    let pa = a[(y * w + x) as usize];
                    let pb = b[(by as u32 * w + bx as u32) as usize];
                    let d = (0..3).map(|c| (pa[c] - pb[c]).abs()).fold(0.0, f32::max);
                    sum += d;
                    n += 1;
                    if d > 64.0 {
                        over += 1;
                    }
                }
            }
            let d = Diff {
                mean: sum / n.max(1) as f32,
                over,
                shift: (sx, sy),
            };
            if best.is_none_or(|b| d.mean < b.mean) {
                best = Some(d);
            }
        }
    }
    best.unwrap()
}

/// The tolerances every region is held to: the mean blurred difference,
/// and the share of its pixels whose blurred difference passes 64. Text
/// rasterised by the two engines differs this much where nothing is wrong
/// (the most, about 4.8 mean, in small text, whose strokes Chrome draws
/// heavier); a broken feature costs far more (the PR's mutants: 13 to 150).
pub const MEAN: f32 = 6.0;
pub const OVER: f32 = 0.01;

pub type Region = (&'static str, (u32, u32, u32, u32));

/// The regions of the page, in CSS px, by what each shows.
pub const LINEAR: Region = ("linear gradient", (0, 0, 190, 80));
pub const RADIAL: Region = ("radial gradient", (190, 0, 375, 80));
pub const PADDING: Region = ("gradient over padding", (375, 0, 600, 80));
pub const WIDTHS: Region = ("stroke widths", (0, 80, 300, 160));
pub const STROKE_FIRST: Region = ("paint-order: stroke", (300, 80, 600, 160));
pub const HUD: Region = ("gradient, stroke under it, hard shadow", (0, 160, 235, 250));
pub const FILL_COLOR_SHADOW: Region = ("fill colour, stroke, blurred shadow", (235, 160, 400, 250));
pub const TRANSPARENT_SHADOW: Region = ("transparent fill, stroke, shadow", (400, 160, 600, 250));
pub const PARAGRAPH: Region = ("wrapped paragraph", (0, 250, 250, 345));
pub const INLINE: Region = ("inline spans", (250, 250, 600, 300));
pub const MIXED: Region = ("spans in a clipped block", (250, 300, 600, 350));
pub const ALL: [Region; 11] = [
    LINEAR,
    RADIAL,
    PADDING,
    WIDTHS,
    STROKE_FIRST,
    HUD,
    FILL_COLOR_SHADOW,
    TRANSPARENT_SHADOW,
    PARAGRAPH,
    INLINE,
    MIXED,
];

/// The regions of `ours` (the page painted at `scale`) unlike `chrome`'s:
/// each with its [`Diff`]. `RINCH_2719_PRINT` prints every region's.
pub fn unlike_chrome(
    ours: &Image,
    chrome: &Image,
    scale: u32,
    regions: &[Region],
) -> Vec<(&'static str, Diff)> {
    assert_eq!((ours.w, ours.h), (chrome.w, chrome.h));
    let (a, b) = (ours.blur(), chrome.blur());
    let mut bad = Vec::new();
    for &(name, rect) in regions {
        let d = diff(&a, &b, ours.w, ours.h, rect, scale);
        let area = ((rect.2 - rect.0) * (rect.3 - rect.1) * scale * scale) as f32;
        if std::env::var_os("RINCH_2719_PRINT").is_some() {
            eprintln!(
                "{name} at {scale}x: {d:?} ({:.4} of the area)",
                d.over as f32 / area
            );
        }
        if d.mean > MEAN || d.over as f32 > OVER * area {
            bad.push((name, d));
        }
    }
    bad
}
