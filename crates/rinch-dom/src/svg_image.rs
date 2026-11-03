//! SVG files as images (#2111): `<img src="icon.svg">`, `background-image:
//! url(icon.svg)`, `mask-image: url(icon.svg)` and an app scheme's SVG bytes.
//!
//! An inline `<svg>` element is a different thing: it is part of the DOM and
//! `paint/svg.rs` draws it. This module is for an SVG **file** used as an
//! image. It is parsed once, at decode (on the load thread natively), by
//! `usvg` into a tree, and rasterised by `resvg` **at the size it is drawn
//! at**, in device pixels: [`VectorImage::raster`], called by the paint of an
//! `<img>` and by a background or mask layer's tile. So an icon stays sharp
//! under any zoom, DPI or CSS size, where rasterising once at the natural size
//! and scaling that would blur it. Rasters are kept per size, at most
//! [`MAX_RASTERS`] per image (least recently used goes), each at most
//! [`MAX_RASTER_PX`] pixels.
//!
//! **Sizing** follows Chrome 156 (`crate::replaced::default_sized`, shared
//! with the `<svg>` element): the natural width and height are the root's
//! `width` and `height` attributes when they are absolute lengths, the ratio
//! theirs, else the `viewBox`'s. **Drawing** into a box of another shape maps
//! the `viewBox` into it by the root's `preserveAspectRatio` (an `<img>` 40x40
//! of a 16x8 icon shows a 40x20 picture, centred), and stretches a file with
//! no `viewBox`, as Chrome does.
//!
//! **A security boundary.** The file is data, never code and never a fetch:
//! there is no script (usvg has no script engine), and nothing outside the
//! file is read. An `<image>` inside it draws nothing, whatever its `href`
//! (a file path, a URL or a `data:` URL; both of usvg's resolvers answer
//! `None`), and usvg follows no external reference (`<use href="other.svg#a">`,
//! a stylesheet `@import`) of its own. **Text is not drawn**: usvg's `text`
//! feature (a font database, shaping) is off, so a `<text>` element is left
//! out. Everything else usvg renders is drawn: paths, shapes, gradients,
//! patterns, clip paths, masks, filters, opacity.

use std::sync::{Arc, Mutex};

use peniko::kurbo;
use resvg::tiny_skia;
use resvg::usvg;

use crate::image_cache::DecodedImage;

/// The most rasters one image keeps, one per drawn size.
pub const MAX_RASTERS: usize = 4;

/// The most device pixels one raster has (16 MB of RGBA). A larger draw is
/// rasterised at this many pixels, keeping its shape, and scaled up by the
/// painter.
pub const MAX_RASTER_PX: f64 = 4.0 * 1024.0 * 1024.0;

/// What an image says about its own size: CSS's natural width, height and
/// ratio, each of which an SVG may lack. A raster image has all three.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NaturalSize {
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Width over height.
    pub ratio: Option<f32>,
}

impl NaturalSize {
    /// A raster image's: its pixel size, and their ratio when neither is 0.
    pub fn of_pixels(width: u32, height: u32) -> Self {
        let (w, h) = (width as f32, height as f32);
        Self {
            width: Some(w),
            height: Some(h),
            ratio: (w > 0.0 && h > 0.0).then(|| w / h),
        }
    }
}

/// Whether `bytes` are an SVG document: after a byte-order mark, white space,
/// an XML declaration, comments and a doctype, the first element is `svg`
/// (or a prefixed `x:svg`).
pub fn sniff_svg(bytes: &[u8]) -> bool {
    let mut s = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    loop {
        let start = s
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(s.len());
        s = &s[start..];
        let skip_past = |s: &[u8], end: &[u8]| {
            s.windows(end.len())
                .position(|w| w == end)
                .map(|i| i + end.len())
        };
        let next = if s.starts_with(b"<?") {
            skip_past(s, b"?>")
        } else if s.starts_with(b"<!--") {
            skip_past(s, b"-->")
        } else if s.starts_with(b"<!") {
            skip_past(s, b">")
        } else {
            break;
        };
        match next {
            Some(n) => s = &s[n..],
            None => return false,
        }
    }
    let Some(rest) = s.strip_prefix(b"<") else {
        return false;
    };
    let name_end = rest
        .iter()
        .position(|&b| b.is_ascii_whitespace() || b == b'>' || b == b'/')
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    let local = name.rsplit(|&b| b == b':').next().unwrap_or(name);
    local == b"svg" && name_end < rest.len()
}

/// Whether a source names an SVG by its `.svg` extension (before any query
/// or fragment, any case) or a `data:image/svg+xml` media type.
pub fn source_says_svg(src: &str) -> bool {
    if let Some(data) = src.strip_prefix("data:") {
        let media = data.split([',', ';']).next().unwrap_or("");
        return media.trim().eq_ignore_ascii_case("image/svg+xml");
    }
    let path = src.split(['?', '#']).next().unwrap_or(src);
    path.len() >= 4 && path[path.len() - 4..].eq_ignore_ascii_case(".svg")
}

/// An SVG file, parsed, with the rasters made of it so far.
pub struct VectorImage {
    tree: usvg::Tree,
    /// The root's `viewBox` and `preserveAspectRatio`, which map the drawing
    /// into whatever box it is drawn into. `None`: stretched.
    view_box: Option<(svgtypes::ViewBox, svgtypes::AspectRatio)>,
    natural: NaturalSize,
    rasters: Mutex<Rasters>,
}

#[derive(Default)]
struct Rasters {
    entries: Vec<RasterEntry>,
    clock: u64,
    made: u64,
}

struct RasterEntry {
    /// The drawn size, in 1/64 device pixels.
    key: (u64, u64),
    raster: Arc<DecodedImage>,
    /// The drawn size the raster's pixels hold (its pixel grid may be a
    /// fraction of a pixel larger).
    drawn: (f64, f64),
    last_used: u64,
}

/// A raster of a [`VectorImage`] at one drawn size: draw `image`'s pixels so
/// that `drawn` device pixels of them cover the image's box. `image` is
/// `ceil(drawn)` pixels; the fraction beyond `drawn` is transparent.
#[derive(Clone)]
pub struct VectorRaster {
    pub image: Arc<DecodedImage>,
    pub drawn: (f64, f64),
}

impl std::fmt::Debug for VectorImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorImage")
            .field("natural", &self.natural)
            .finish_non_exhaustive()
    }
}

/// A length on the root `<svg>` as Chrome 156 reads it for an image's
/// natural size: an absolute length in CSS pixels (`em` against the default
/// 16px font), a negative one 0; a percentage, `ex` or anything unparsable
/// is no natural size.
fn natural_length(value: Option<&str>) -> Option<f32> {
    use svgtypes::LengthUnit as U;
    let l: svgtypes::Length = value?.trim().parse().ok()?;
    let px = match l.unit {
        U::None | U::Px => l.number,
        U::In => l.number * 96.0,
        U::Cm => l.number * 96.0 / 2.54,
        U::Mm => l.number * 96.0 / 25.4,
        U::Pt => l.number * 96.0 / 72.0,
        U::Pc => l.number * 16.0,
        U::Em => l.number * 16.0,
        U::Ex | U::Percent => return None,
    } as f32;
    px.is_finite().then_some(px.max(0.0))
}

/// Parse an SVG file. Fails on bytes that are not UTF-8 or not a well-formed
/// SVG document, or whose size cannot be resolved.
pub fn decode_svg(bytes: &[u8]) -> Result<VectorImage, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("SVG is not UTF-8: {e}"))?;
    let doc = usvg::roxmltree::Document::parse_with_options(
        text,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Failed to parse SVG: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err("Failed to parse SVG: the root element is not <svg>".into());
    }
    let view_box = root
        .attribute("viewBox")
        .and_then(|v| v.parse::<svgtypes::ViewBox>().ok())
        .filter(|vb| vb.w > 0.0 && vb.h > 0.0 && (vb.w / vb.h).is_finite())
        .map(|vb| {
            let aspect = root
                .attribute("preserveAspectRatio")
                .and_then(|a| a.parse().ok())
                .unwrap_or_default();
            (vb, aspect)
        });
    let width = natural_length(root.attribute("width"));
    let height = natural_length(root.attribute("height"));
    // Both attributes' ratio first, the viewBox's after, as for an `<svg>`
    // element (`replaced::svg_context`).
    let ratio = match (width, height) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(w / h),
        _ => view_box.map(|(vb, _)| (vb.w / vb.h) as f32),
    };
    drop(doc);

    let options = usvg::Options {
        resources_dir: None,
        // The viewport of a file with neither a size nor a viewBox: CSS's
        // default object size, which Chrome draws it at.
        default_size: usvg::Size::from_wh(300.0, 150.0).expect("a valid size"),
        // Nothing outside the file is read: an `<image>` draws nothing.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree =
        usvg::Tree::from_str(text, &options).map_err(|e| format!("Failed to parse SVG: {e}"))?;
    Ok(VectorImage {
        tree,
        view_box,
        natural: NaturalSize {
            width,
            height,
            ratio,
        },
        rasters: Mutex::new(Rasters::default()),
    })
}

/// The `viewBox` to viewport transform of SVG 1.1 §7.8, for a viewport of
/// `w` x `h` at the origin.
fn view_box_transform(
    vb: &svgtypes::ViewBox,
    aspect: &svgtypes::AspectRatio,
    w: f64,
    h: f64,
) -> tiny_skia::Transform {
    use svgtypes::Align as A;
    let (mut sx, mut sy) = (w / vb.w, h / vb.h);
    if aspect.align != A::None {
        let s = if aspect.slice { sx.max(sy) } else { sx.min(sy) };
        (sx, sy) = (s, s);
    }
    let (fx, fy) = match aspect.align {
        A::None | A::XMinYMin => (0.0, 0.0),
        A::XMidYMin => (0.5, 0.0),
        A::XMaxYMin => (1.0, 0.0),
        A::XMinYMid => (0.0, 0.5),
        A::XMidYMid => (0.5, 0.5),
        A::XMaxYMid => (1.0, 0.5),
        A::XMinYMax => (0.0, 1.0),
        A::XMidYMax => (0.5, 1.0),
        A::XMaxYMax => (1.0, 1.0),
    };
    let tx = -vb.x * sx + (w - vb.w * sx) * fx;
    let ty = -vb.y * sy + (h - vb.h * sy) * fy;
    tiny_skia::Transform::from_row(sx as f32, 0.0, 0.0, sy as f32, tx as f32, ty as f32)
}

impl VectorImage {
    /// The file's natural size and ratio.
    pub fn natural(&self) -> NaturalSize {
        self.natural
    }

    /// The size a draw that knows no other lays the image out at (the
    /// `object-fit` box of an `<img>`): the natural size, a missing dimension
    /// from the ratio or the default object size, and for a ratio alone the
    /// largest box of that ratio inside 300x150 — what Chrome 156 reports as
    /// `naturalWidth`/`naturalHeight`.
    pub fn concrete(&self) -> (f64, f64) {
        let n = self.natural;
        let (w, h) = crate::replaced::default_sized(n.width, n.height, n.ratio);
        if w.is_infinite() {
            let (dw, dh) = crate::replaced::DEFAULT_OBJECT_SIZE;
            let r = n.ratio.unwrap_or(dw / dh);
            let (w, h) = if dw / dh > r {
                (dh * r, dh)
            } else {
                (dw, dw / r)
            };
            return (f64::from(w), f64::from(h));
        }
        (f64::from(w), f64::from(h))
    }

    /// How many rasters this image has made (a test's count).
    #[doc(hidden)]
    pub fn rasters_made(&self) -> u64 {
        self.rasters.lock().unwrap_or_else(|e| e.into_inner()).made
    }

    /// The raster for drawing an `iw` x `ih` box of the image through
    /// `transform` (image units to device pixels): the device size that box
    /// covers is the box's sides scaled by the transform's (a rotation keeps
    /// the length, a skew or a 3D projection is approximated).
    pub fn raster_for(&self, iw: f64, ih: f64, transform: kurbo::Affine) -> Option<VectorRaster> {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        self.raster(iw * a.hypot(b), ih * c.hypot(d))
    }

    /// How many rasters this image keeps now (a test's count).
    #[doc(hidden)]
    pub fn rasters_held(&self) -> usize {
        self.rasters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }

    /// The drawing rasterised to cover `dw` x `dh` device pixels, made now or
    /// kept from an earlier draw at that size. `None` for an empty or
    /// non-finite size.
    pub fn raster(&self, dw: f64, dh: f64) -> Option<VectorRaster> {
        if !(dw > 0.0 && dh > 0.0 && dw.is_finite() && dh.is_finite()) {
            return None;
        }
        // A huge draw is rasterised smaller, keeping its shape.
        let shrink = (MAX_RASTER_PX / (dw * dh)).sqrt().min(1.0);
        let (dw, dh) = (dw * shrink, dh * shrink);
        let key = ((dw * 64.0).round() as u64, (dh * 64.0).round() as u64);
        let mut rasters = self.rasters.lock().unwrap_or_else(|e| e.into_inner());
        rasters.clock += 1;
        let clock = rasters.clock;
        if let Some(entry) = rasters.entries.iter_mut().find(|e| e.key == key) {
            entry.last_used = clock;
            return Some(VectorRaster {
                image: entry.raster.clone(),
                drawn: entry.drawn,
            });
        }
        let w = (dw - 1e-6).ceil().max(1.0) as u32;
        let h = (dh - 1e-6).ceil().max(1.0) as u32;
        let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
        let size = self.tree.size();
        let (sw, sh) = (f64::from(size.width()), f64::from(size.height()));
        // usvg has already mapped the viewBox into the tree's own size;
        // undo that and map it into the drawn size instead, by the root's
        // `preserveAspectRatio`. With no viewBox, Chrome stretches.
        let transform = match &self.view_box {
            Some((vb, aspect)) => {
                let to_tree = view_box_transform(vb, aspect, sw, sh);
                let to_drawn = view_box_transform(vb, aspect, dw, dh);
                to_drawn.pre_concat(to_tree.invert()?)
            }
            None => tiny_skia::Transform::from_scale((dw / sw) as f32, (dh / sh) as f32),
        };
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        let raster = Arc::new(DecodedImage::new(pixmap.take_demultiplied(), w, h));
        rasters.made += 1;
        if rasters.entries.len() >= MAX_RASTERS
            && let Some(oldest) = rasters
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(i, _)| i)
        {
            rasters.entries.swap_remove(oldest);
        }
        rasters.entries.push(RasterEntry {
            key,
            raster: raster.clone(),
            drawn: (dw, dh),
            last_used: clock,
        });
        Some(VectorRaster {
            image: raster,
            drawn: (dw, dh),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing_finds_the_svg_root_after_a_prolog() {
        assert!(sniff_svg(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(sniff_svg(
            b"\xEF\xBB\xBF  <?xml version='1.0'?>\n<!-- c -->\n<!DOCTYPE svg>\n<svg>"
        ));
        assert!(sniff_svg(b"<svg:svg xmlns:svg='x'>"));
        assert!(!sniff_svg(b"<svgx>"));
        assert!(!sniff_svg(b"<html><svg>"));
        assert!(!sniff_svg(b"\x89PNG\r\n"));
        assert!(!sniff_svg(b"<!-- never closed"));
    }

    #[test]
    fn a_source_names_svg_by_extension_or_media_type() {
        assert!(source_says_svg("icons/a.svg"));
        assert!(source_says_svg("https://x/a.SVG?v=2#frag"));
        assert!(source_says_svg("data:image/svg+xml;base64,AAAA"));
        assert!(source_says_svg("data:IMAGE/SVG+XML,%3Csvg"));
        assert!(!source_says_svg("a.svgz"));
        assert!(!source_says_svg("data:image/png;base64,AAAA"));
        assert!(!source_says_svg("svg"));
    }
}
