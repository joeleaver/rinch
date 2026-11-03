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
//! and scaling that would blur it.
//!
//! **An SVG file is untrusted input** (review of #2125), and these limits keep
//! one from crashing the app or holding its UI thread:
//! - [`prescan`] reads the bytes before any parser does: element nesting is
//!   at most [`MAX_DEPTH`] (roxmltree and usvg recurse once per level, and a
//!   stack overflow aborts the process, which `catch_unwind` cannot stop); an
//!   internal DTD may declare only flat entities (no `&` or `%` in a value,
//!   no external entity), and all references to them together expand to at
//!   most [`MAX_ENTITY_EXPANSION`] bytes. Illustrator's `&ns_svg;` is fine.
//! - `check_expansion` then walks the parsed XML the way usvg will expand
//!   it — every `<use>`/`href` and `url(#id)` reference counted as a copy of
//!   what it names (a marker once per vertex) — and refuses a file whose
//!   expanded depth passes [`MAX_DEPTH`] or whose element count passes
//!   [`MAX_NODES`], before usvg builds that tree. usvg's own tree is counted
//!   again afterwards.
//! - Each raster has a budget of [`MAX_RASTER_WORK`], an estimate of resvg's
//!   time: the device pixels painted, from the bounding boxes of everything
//!   resvg will fill (each layer and clip/mask/pattern subtree included)
//!   scaled to the drawn size, plus [`WORK_PER_ELEMENT`] per element. A file
//!   over it at that size draws nothing and logs why, once.
//! - Parsing and rasterising run under `catch_unwind`: a panic in usvg or
//!   resvg fails that image (and it is not rasterised again), not the app.
//! - Rasters are kept per drawn size in one cache per thread, at most
//!   [`RASTER_BUDGET_BYTES`] in all; a raster the frame being painted uses is
//!   never evicted (#1156's rule for background tiles), so any number of sizes
//!   on screen are made once, and one no frame has used for
//!   [`RASTER_IDLE_FRAMES`] painted frames is let go. Each raster is at most
//!   [`MAX_RASTER_PX`] pixels (counted after rounding up to whole pixels).
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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use peniko::kurbo;
use resvg::tiny_skia;
use resvg::usvg;

use crate::image_cache::DecodedImage;

/// The most device pixels one raster has (16 MB of RGBA), counted after
/// rounding up to whole pixels. A larger draw is rasterised at this many
/// pixels, keeping its shape, and scaled up by the painter.
pub const MAX_RASTER_PX: f64 = 4.0 * 1024.0 * 1024.0;

/// The deepest an SVG file may nest, as written (elements) and as usvg
/// expands it (each `<use>` or other reference a level). Real icons and
/// illustrations nest a few dozen deep; a debug build survives this many on
/// a 2 MB stack with room to spare (roxmltree, usvg and resvg each recurse
/// once per level, and a debug build overflowed 2 MB at about 1,000).
pub const MAX_DEPTH: usize = 128;

/// The most elements an SVG file may expand to (each reference counted as a
/// copy of what it names): far beyond a detailed illustration (a few
/// thousand paths).
pub const MAX_NODES: u64 = 100_000;

/// The most bytes all references to an internal DTD's entities may expand to.
pub const MAX_ENTITY_EXPANSION: usize = 1 << 20;

/// The most work one raster may take, in device pixels painted, estimated
/// from bounding boxes, plus [`WORK_PER_ELEMENT`] for each element drawn:
/// about a second of resvg on one core (measured on a loaded host, release:
/// 1.46e9 took 1.95 s, 3.65e8 took 0.6 s). A 1200x800 illustration of 2,000
/// small shapes estimates at 2.1e7 (29 ms) and at 2x 2.5e7 (70 ms); 2,000
/// half-transparent shapes each spanning the canvas are drawn at 1200x800
/// and refused at 2x.
pub const MAX_RASTER_WORK: f64 = 1.0e9;

/// What each element drawn costs resvg whatever its size, as pixels painted
/// (about 14 µs a path, measured, against about 1.3 ns a pixel).
pub const WORK_PER_ELEMENT: f64 = 1.0e4;

/// The bytes every SVG raster on one thread may hold together: 64 MB.
/// Charged at 8 bytes a pixel (the raster, and the painter's premultiplied
/// copy or GPU blob of it).
pub const RASTER_BUDGET_BYTES: usize = 64 << 20;

/// Painted frames a raster may go unused before it is let go.
pub const RASTER_IDLE_FRAMES: u64 = 60;

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

/// The length of the markup construct at the start of `s` that is not an
/// element — `<?…?>`, `<!--…-->`, `<![CDATA[…]]>` or a `<!DOCTYPE …>` with
/// its quoted strings and internal subset — or `None` if `s` does not start
/// with one or it never ends.
fn skip_non_element(s: &[u8]) -> Option<usize> {
    let past = |from: usize, end: &[u8]| {
        s.get(from..)?
            .windows(end.len())
            .position(|w| w == end)
            .map(|i| from + i + end.len())
    };
    if s.starts_with(b"<?") {
        past(2, b"?>")
    } else if s.starts_with(b"<!--") {
        past(4, b"-->")
    } else if s.starts_with(b"<![CDATA[") {
        past(9, b"]]>")
    } else if s.starts_with(b"<!") {
        // A declaration: `>` ends it outside quotes and outside `[…]`, whose
        // own comments and quoted strings may hold either.
        let mut i = 2;
        let mut subset = false;
        while i < s.len() {
            match s[i] {
                q @ (b'"' | b'\'') => i += s[i + 1..].iter().position(|&b| b == q)? + 2,
                b'<' if subset && s[i..].starts_with(b"<!--") => i = past(i + 4, b"-->")?,
                b'[' if !subset => {
                    subset = true;
                    i += 1;
                }
                b']' if subset => {
                    subset = false;
                    i += 1;
                }
                b'>' if !subset => return Some(i + 1),
                _ => i += 1,
            }
        }
        None
    } else {
        None
    }
}

/// Whether `bytes` are an SVG document: after a byte-order mark, white space,
/// an XML declaration, comments and a doctype (its internal subset
/// included), the first element is `svg` (or a prefixed `x:svg`).
pub fn sniff_svg(bytes: &[u8]) -> bool {
    let mut s = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    loop {
        let start = s
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(s.len());
        s = &s[start..];
        if !s.starts_with(b"<!") && !s.starts_with(b"<?") {
            break;
        }
        match skip_non_element(s) {
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

/// Read an SVG file's bytes before any parser does (see the module doc):
/// element nesting at most [`MAX_DEPTH`], an internal DTD with flat internal
/// entities only, and their references expanding to at most
/// [`MAX_ENTITY_EXPANSION`] bytes. A loop over the bytes, with no recursion.
pub fn prescan(bytes: &[u8]) -> Result<(), String> {
    let mut entities: HashMap<&[u8], usize> = HashMap::new();
    let mut expanded = 0usize;
    let mut depth = 0usize;
    let mut i = 0;
    // Entity references in a run of text or a tag's attribute values.
    let mut count_refs = |run: &[u8], entities: &HashMap<&[u8], usize>| -> Result<(), String> {
        if entities.is_empty() {
            return Ok(());
        }
        let mut at = 0;
        while let Some(amp) = run[at..].iter().position(|&b| b == b'&') {
            let start = at + amp + 1;
            let Some(semi) = run[start..].iter().position(|&b| b == b';') else {
                break;
            };
            if let Some(len) = entities.get(&run[start..start + semi]) {
                expanded += len;
                if expanded > MAX_ENTITY_EXPANSION {
                    return Err(format!(
                        "SVG refused: its entities expand past {MAX_ENTITY_EXPANSION} bytes"
                    ));
                }
            }
            at = start + semi + 1;
        }
        Ok(())
    };
    while i < bytes.len() {
        let Some(lt) = bytes[i..].iter().position(|&b| b == b'<') else {
            count_refs(&bytes[i..], &entities)?;
            break;
        };
        count_refs(&bytes[i..i + lt], &entities)?;
        i += lt;
        let s = &bytes[i..];
        if s.len() >= 9 && s[..9].eq_ignore_ascii_case(b"<!DOCTYPE") {
            let len = skip_non_element(s).ok_or("SVG refused: an unterminated doctype")?;
            read_entities(&s[..len], &mut entities)?;
            i += len;
        } else if s.starts_with(b"<!") || s.starts_with(b"<?") {
            i += skip_non_element(s).ok_or("SVG refused: unterminated markup")?;
        } else {
            // A tag: to its `>`, outside quoted attribute values.
            let mut j = 1;
            let mut quote = None;
            while j < s.len() {
                match (quote, s[j]) {
                    (None, q @ (b'"' | b'\'')) => quote = Some(q),
                    (Some(q), b) if b == q => quote = None,
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            if j >= s.len() {
                break; // Unterminated: the parser reports it.
            }
            count_refs(&s[..j], &entities)?;
            if s.starts_with(b"</") {
                depth = depth.saturating_sub(1);
            } else if s[j - 1] != b'/' {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(format!(
                        "SVG refused: elements nested more than {MAX_DEPTH} deep"
                    ));
                }
            }
            i += j + 1;
        }
    }
    Ok(())
}

/// The `<!ENTITY>` declarations of a doctype into `entities` (name → value
/// length). Refuses a parameter entity, an external one, and a value that
/// refers to another entity: nesting is what makes expansion exponential.
fn read_entities<'a>(
    doctype: &'a [u8],
    entities: &mut HashMap<&'a [u8], usize>,
) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("SVG refused: a DTD entity {why}"));
    let mut rest = doctype;
    while let Some(at) = rest.windows(8).position(|w| w == b"<!ENTITY") {
        rest = rest[at + 8..].trim_ascii_start();
        let name_len = rest
            .iter()
            .position(|b| b.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        if name.is_empty() {
            return refuse("has no name");
        }
        if name == b"%" {
            return refuse("is a parameter entity");
        }
        let value = rest[name_len..].trim_ascii_start();
        let Some(&q @ (b'"' | b'\'')) = value.first() else {
            return refuse("is external (SYSTEM or PUBLIC)");
        };
        let Some(end) = value[1..].iter().position(|&b| b == q) else {
            return refuse("value is unterminated");
        };
        let text = &value[1..1 + end];
        if text.contains(&b'&') || text.contains(&b'%') {
            return refuse("refers to another entity");
        }
        entities.insert(name, text.len());
        rest = &value[1 + end..];
    }
    Ok(())
}

/// The `#id`s an element refers to, and how many copies of each usvg makes:
/// an `href`/`xlink:href` (a `<use>`, a gradient or pattern's template) and
/// every `url(#id)` in an attribute or its `style`, once each, except a
/// marker, drawn at each vertex of the path (estimated as one per two numbers
/// in its `d` or `points`).
fn references<'a, 'i>(node: usvg::roxmltree::Node<'a, 'i>) -> Vec<(&'a str, u64)> {
    let vertices = || {
        let data = node
            .attribute("d")
            .or(node.attribute("points"))
            .unwrap_or("");
        let numbers = data
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .filter(|n| !n.is_empty())
            .count() as u64;
        numbers / 2 + 1
    };
    let mut out = Vec::new();
    for attr in node.attributes() {
        let v = attr.value();
        if attr.name() == "href" {
            if let Some(id) = v.trim().strip_prefix('#') {
                out.push((id, 1));
            }
            continue;
        }
        let marker =
            attr.name().starts_with("marker") || (attr.name() == "style" && v.contains("marker"));
        let mut rest = v;
        while let Some(at) = rest.find("url(") {
            rest = &rest[at + 4..];
            let inner = rest.split(')').next().unwrap_or("");
            let id = inner
                .trim()
                .trim_matches(['"', '\''])
                .trim_start_matches('#');
            if !id.is_empty() {
                out.push((id, if marker { vertices() } else { 1 }));
            }
        }
    }
    out
}

/// Walk the parsed XML as usvg will expand it (see the module doc): refuse a
/// file whose expanded depth passes [`MAX_DEPTH`] or whose element count
/// passes [`MAX_NODES`]. Memoised per element, and its own recursion stops at
/// [`MAX_DEPTH`].
fn check_expansion(doc: &usvg::roxmltree::Document) -> Result<(), String> {
    use usvg::roxmltree::{Node, NodeId};
    type Memo = HashMap<NodeId, Option<(u64, usize)>>;
    fn too_deep<T>() -> Result<T, String> {
        Err(format!(
            "SVG refused: nested more than {MAX_DEPTH} deep once its references are expanded"
        ))
    }
    fn expand<'a, 'i>(
        ids: &HashMap<&'a str, Node<'a, 'i>>,
        memo: &mut Memo,
        node: Node<'a, 'i>,
        depth: usize,
    ) -> Result<(u64, usize), String> {
        if depth > MAX_DEPTH {
            return too_deep();
        }
        match memo.get(&node.id()) {
            Some(Some((count, height))) => {
                return if depth + height > MAX_DEPTH {
                    too_deep()
                } else {
                    Ok((*count, *height))
                };
            }
            // A reference cycle: usvg drops it.
            Some(None) => return Ok((0, 0)),
            None => {}
        }
        memo.insert(node.id(), None);
        let (mut count, mut height) = (1u64, 0usize);
        let refs: Vec<(Node<'a, 'i>, u64)> = references(node)
            .into_iter()
            .filter_map(|(id, n)| ids.get(id).map(|&t| (t, n)))
            .collect();
        let children = node.children().filter(|c| c.is_element()).map(|c| (c, 1));
        for (target, copies) in refs.into_iter().chain(children) {
            let (c, h) = expand(ids, memo, target, depth + 1)?;
            count = count.saturating_add(c.saturating_mul(copies));
            height = height.max(h + 1);
            if count > MAX_NODES {
                return Err(format!(
                    "SVG refused: it expands to more than {MAX_NODES} elements"
                ));
            }
        }
        memo.insert(node.id(), Some((count, height)));
        Ok((count, height))
    }
    let ids = doc
        .descendants()
        .filter_map(|n| n.attribute("id").map(|id| (id, n)))
        .collect();
    expand(&ids, &mut Memo::new(), doc.root_element(), 0).map(|_| ())
}

/// Run `f`, turning a panic into an error naming `what`: usvg and resvg are
/// large, and one of their panics must fail an image, not the app.
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Err(format!("{what} panicked: {message}"))
    })
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

/// An SVG file, parsed.
pub struct VectorImage {
    tree: usvg::Tree,
    /// The root's `viewBox` and `preserveAspectRatio`, which map the drawing
    /// into whatever box it is drawn into. `None`: stretched.
    view_box: Option<(svgtypes::ViewBox, svgtypes::AspectRatio)>,
    natural: NaturalSize,
    /// The device pixels a raster paints per pixel of the raster, estimated
    /// from bounding boxes (see [`MAX_RASTER_WORK`]).
    work_per_pixel: f64,
    /// The elements usvg built (each copy of a reference counted).
    elements: u64,
    /// Process-unique: the raster cache's key.
    id: u64,
    made: AtomicU64,
    /// A refused or failed raster has been logged.
    warned: AtomicBool,
    /// resvg panicked on this file: it is not rasterised again.
    broken: AtomicBool,
}

/// The SVG rasters of one thread (the painting one): see the module doc.
struct RasterCache {
    entries: HashMap<(u64, u64, u64), RasterEntry>,
    bytes: usize,
    frame: u64,
}

struct RasterEntry {
    raster: Arc<DecodedImage>,
    /// The drawn size the raster's pixels hold (its pixel grid may be a
    /// fraction of a pixel larger).
    drawn: (f64, f64),
    bytes: usize,
    last_frame: u64,
}

thread_local! {
    static RASTERS: RefCell<RasterCache> = RefCell::new(RasterCache {
        entries: HashMap::new(),
        bytes: 0,
        frame: 0,
    });
    static BUDGET: Cell<usize> = const { Cell::new(RASTER_BUDGET_BYTES) };
}

/// A new painted frame on this thread (`paint::paint_document` calls it):
/// the rasters the last one used may now be evicted, and those no frame has
/// used for [`RASTER_IDLE_FRAMES`] frames are let go.
pub fn begin_frame() {
    RASTERS.with_borrow_mut(|c| {
        c.frame += 1;
        let frame = c.frame;
        let mut freed = 0;
        c.entries.retain(|_, e| {
            let keep = frame - e.last_frame <= RASTER_IDLE_FRAMES;
            if !keep {
                freed += e.bytes;
            }
            keep
        });
        c.bytes -= freed;
    });
}

/// The bytes this thread's SVG rasters hold, as charged (a test's count).
#[doc(hidden)]
pub fn raster_bytes() -> usize {
    RASTERS.with_borrow(|c| c.bytes)
}

/// The guard [`test_budget`] returns.
#[doc(hidden)]
pub struct BudgetGuard(usize);

impl Drop for BudgetGuard {
    fn drop(&mut self) {
        BUDGET.with(|b| b.set(self.0));
    }
}

/// Set this thread's raster budget until the guard drops (a test's knob).
#[doc(hidden)]
pub fn test_budget(bytes: usize) -> BudgetGuard {
    BudgetGuard(BUDGET.with(|b| b.replace(bytes)))
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
/// SVG document, whose size cannot be resolved, or that pass a limit of the
/// module doc ([`prescan`], `check_expansion`); and if usvg panics.
pub fn decode_svg(bytes: &[u8]) -> Result<VectorImage, String> {
    guarded("SVG parsing", || decode_svg_unguarded(bytes))
}

fn decode_svg_unguarded(bytes: &[u8]) -> Result<VectorImage, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("SVG is not UTF-8: {e}"))?;
    prescan(bytes)?;
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
    check_expansion(&doc)?;
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
    // What usvg built, measured: its element count (it expands a marker per
    // vertex, which the estimate above only approximates) and the area its
    // fills and layers cover.
    let size = tree.size();
    let canvas = (f64::from(size.width()), f64::from(size.height()));
    let mut tally = Tally {
        nodes: 0,
        area: 0.0,
        canvas,
    };
    tally.group(tree.root(), 0)?;
    Ok(VectorImage {
        tree,
        view_box,
        natural: NaturalSize {
            width,
            height,
            ratio,
        },
        work_per_pixel: tally.area / (canvas.0 * canvas.1),
        elements: tally.nodes,
        id: {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            NEXT.fetch_add(1, Ordering::Relaxed)
        },
        made: AtomicU64::new(0),
        warned: AtomicBool::new(false),
        broken: AtomicBool::new(false),
    })
}

/// The usvg tree's element count and the area (in its own units, clipped to
/// its canvas) of every path and every isolated layer resvg paints, clip,
/// mask and pattern contents included as often as they are used.
struct Tally {
    nodes: u64,
    area: f64,
    canvas: (f64, f64),
}

impl Tally {
    fn rect(&mut self, r: usvg::Rect) {
        let w = (f64::from(r.right()).min(self.canvas.0) - f64::from(r.left()).max(0.0)).max(0.0);
        let h = (f64::from(r.bottom()).min(self.canvas.1) - f64::from(r.top()).max(0.0)).max(0.0);
        self.area += w * h;
    }

    fn node(&mut self, node: &usvg::Node, depth: usize) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!(
                "SVG refused: it expands to more than {MAX_NODES} elements"
            ));
        }
        if depth > 2 * MAX_DEPTH {
            return Err(format!(
                "SVG refused: nested more than {MAX_DEPTH} deep once its references are expanded"
            ));
        }
        let mut result = Ok(());
        node.subroots(|g| {
            if result.is_ok() {
                result = self.group(g, depth + 1);
            }
        });
        result?;
        match node {
            usvg::Node::Group(g) => {
                if g.should_isolate() {
                    self.rect(g.abs_layer_bounding_box().to_rect());
                }
                self.group(g, depth + 1)
            }
            usvg::Node::Path(p) => {
                self.rect(p.abs_stroke_bounding_box());
                Ok(())
            }
            usvg::Node::Image(i) => {
                self.rect(i.abs_bounding_box());
                Ok(())
            }
            usvg::Node::Text(_) => Ok(()),
        }
    }

    fn group(&mut self, g: &usvg::Group, depth: usize) -> Result<(), String> {
        for child in g.children() {
            self.node(child, depth)?;
        }
        Ok(())
    }
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
        self.made.load(Ordering::Relaxed)
    }

    /// The raster for drawing an `iw` x `ih` box of the image through
    /// `transform` (image units to device pixels): the device size that box
    /// covers is the box's sides scaled by the transform's (a rotation keeps
    /// the length, a skew or a 3D projection is approximated).
    pub fn raster_for(&self, iw: f64, ih: f64, transform: kurbo::Affine) -> Option<VectorRaster> {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        self.raster(iw * a.hypot(b), ih * c.hypot(d))
    }

    /// How many rasters of this image this thread keeps now (a test's count).
    #[doc(hidden)]
    pub fn rasters_held(&self) -> usize {
        RASTERS.with_borrow(|c| c.entries.keys().filter(|k| k.0 == self.id).count())
    }

    /// The device pixels a `w` x `h` raster is estimated to paint, which
    /// [`MAX_RASTER_WORK`] bounds.
    pub fn estimated_work(&self, w: u32, h: u32) -> f64 {
        self.work_per_pixel * f64::from(w) * f64::from(h) + self.elements as f64 * WORK_PER_ELEMENT
    }

    fn warn_once(&self, why: &str) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!("SVG image not drawn: {why}");
        }
    }

    /// The drawing rasterised to cover `dw` x `dh` device pixels, made now or
    /// kept from an earlier draw at that size. `None` for an empty or
    /// non-finite size, a draw over [`MAX_RASTER_WORK`], or a file resvg
    /// panicked on.
    pub fn raster(&self, dw: f64, dh: f64) -> Option<VectorRaster> {
        if !(dw > 0.0 && dh > 0.0 && dw.is_finite() && dh.is_finite())
            || self.broken.load(Ordering::Relaxed)
        {
            return None;
        }
        // A huge draw is rasterised smaller, keeping its shape, so that the
        // whole pixels allocated (rounded up) stay within the cap.
        let whole = |v: f64| (v - 1e-6).ceil().max(1.0);
        let (mut dw, mut dh) = (dw, dh);
        while whole(dw) * whole(dh) > MAX_RASTER_PX {
            let shrink = (MAX_RASTER_PX / (whole(dw) * whole(dh))).sqrt().min(0.999);
            (dw, dh) = (dw * shrink, dh * shrink);
        }
        let key = (
            self.id,
            (dw * 64.0).round() as u64,
            (dh * 64.0).round() as u64,
        );
        let hit = RASTERS.with_borrow_mut(|c| {
            let frame = c.frame;
            c.entries.get_mut(&key).map(|e| {
                e.last_frame = frame;
                VectorRaster {
                    image: e.raster.clone(),
                    drawn: e.drawn,
                }
            })
        });
        if hit.is_some() {
            return hit;
        }
        let (w, h) = (whole(dw) as u32, whole(dh) as u32);
        let work = self.estimated_work(w, h);
        if work > MAX_RASTER_WORK {
            self.warn_once(&format!(
                "rasterising it at {w}x{h} would paint about {work:.1e} pixels, past the \
                 limit of {MAX_RASTER_WORK:.1e}"
            ));
            return None;
        }
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
        let pixels = guarded("SVG rasterising", || {
            let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("no pixmap")?;
            resvg::render(&self.tree, transform, &mut pixmap.as_mut());
            Ok(pixmap.take_demultiplied())
        });
        let pixels = match pixels {
            Ok(p) => p,
            Err(e) => {
                self.broken.store(true, Ordering::Relaxed);
                self.warn_once(&e);
                return None;
            }
        };
        let raster = Arc::new(DecodedImage::new(pixels, w, h));
        self.made.fetch_add(1, Ordering::Relaxed);
        let bytes = w as usize * h as usize * 8;
        RASTERS.with_borrow_mut(|c| {
            // Make room: least recently used first, never one this frame has
            // drawn (if only those are left, the frame's own rasters stay
            // over the budget until a later frame stops using them).
            let budget = BUDGET.with(Cell::get);
            let frame = c.frame;
            while c.bytes + bytes > budget {
                let Some(oldest) = c
                    .entries
                    .iter()
                    .filter(|(_, e)| e.last_frame < frame)
                    .min_by_key(|(_, e)| e.last_frame)
                    .map(|(k, _)| *k)
                else {
                    break;
                };
                let gone = c.entries.remove(&oldest).expect("listed");
                c.bytes -= gone.bytes;
            }
            c.bytes += bytes;
            c.entries.insert(
                key,
                RasterEntry {
                    raster: raster.clone(),
                    drawn: (dw, dh),
                    bytes,
                    last_frame: frame,
                },
            );
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
    fn a_panic_fails_the_image_not_the_app() {
        let r: Result<(), String> = guarded("SVG parsing", || panic!("boom"));
        assert_eq!(r, Err("SVG parsing panicked: boom".to_string()));
    }

    #[test]
    fn the_prescan_counts_depth_across_comments_and_quotes() {
        let nest = |n: usize| format!("{}{}", "<g a='>'><!-- <g> -->".repeat(n), "</g>".repeat(n));
        assert!(prescan(nest(MAX_DEPTH).as_bytes()).is_ok());
        assert!(prescan(nest(MAX_DEPTH + 1).as_bytes()).is_err());
        assert!(prescan(b"<g/><g/><g></g>").is_ok());
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
