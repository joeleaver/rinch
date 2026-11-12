//! What an `<img>` with no picture draws in its place (#2309).
//!
//! An `<img>` has no picture when it has no `src` (absent, or empty or white
//! space only) or its load **failed** ([`crate::image_cache::ImageCache::shows_failure`],
//! which stays true while a failed source is being reloaded, so a reload
//! does not blank the placeholder before its answer lands). One loading for
//! the first time has none yet and draws nothing, as in a browser. Mail is
//! the case that matters: Postrinch strips the `src` of every held-back
//! remote picture and keeps its `width`, `height`, `alt` and `style`.
//!
//! Joe's rules ([`placeholder`]), with Chrome 156's thresholds:
//!
//! 1. **Sized** — its box has both dimensions without its picture: computed
//!    `width` and `height` both non-`auto`, or one with a ratio (the
//!    `aspect-ratio: auto w / h` hint of #684, so `max-width: 100%; height:
//!    auto` keeps the attributes' ratio): a [`Placeholder::Box`], a faint
//!    border inside its content box and the `alt` text inside that, wrapped
//!    to the box and clipped to it, in the element's own font and colour.
//!    The ring is what Chrome 156 draws for a sized image with no alt text
//!    (1px `#c0c0c0` on the content box's edge, 1px of padding), and only
//!    when the content box is at least [`MIN_BOX_NO_SRC`] (3px) on both axes
//!    with no `src`, [`MIN_BOX_FAILED`] (18px) after a failed load: a thinner
//!    spacer or divider draws nothing. For one *with* alt text Chrome shows
//!    the text inline and ignores the size (it draws this box only in quirks
//!    mode); rinch draws the box, so held-back pictures keep their place.
//! 2. **`alt`, and not both dimensions**: [`Placeholder::Inline`], the alt
//!    text as Chrome lays it out, an inline-block of it ([`AltText`]): as
//!    wide as the text up to the room it has (it wraps), or its declared
//!    width; as tall as its wrapped lines, or its declared height, and at
//!    least [`MIN_INLINE_HEIGHT`] (Chrome's 16px icon); on its
//!    last line's baseline. `<img width=600 alt=Banner style="display:block;
//!    width:100%; height:auto">` in a 150px column is 150 by one line, as in
//!    Chrome; a long alt text in an 80px column is 80 wide and wraps.
//! 3. **No alt and not both dimensions, or every declared size at most 1px**
//!    (a tracking pixel): nothing drawn, the declared size only (Joe's rule
//!    3). Chrome draws a failed image with no alt and one or no dimension as
//!    a 16x16 box holding its broken-image icon; rinch draws nothing there
//!    and keeps the declared size (0x0).
//!
//! A box below the minimum that has alt text (a failed 16x16
//! `alt="Facebook"` icon) shows the text inline instead, its size ignored
//! ([`ignores_size`]), as Chrome shows it.
//!
//! Rinch draws Chrome's broken-image icon nowhere (Chrome draws it in the
//! box's corner after a failed load at 18px and up, as the whole 16x16 box
//! of a failed image with no alt and no size, and before the alt text of an
//! inline one).
//!
//! **Overriding the look.** The box is drawn from custom properties, read only
//! while the element is a placeholder, inherited like any custom property (so
//! `:root { … }` sets them app-wide), and immune to the `img { border: 0 }`
//! resets mail is full of (which would remove a border set on the element):
//!
//! - `--rinch-img-placeholder: none` is the escape hatch: no placeholder at
//!   all, today's behaviour (nothing drawn, an alt-only image 0x0). Any other
//!   value, or none, is `auto`.
//! - `--rinch-img-placeholder-border-color` (a CSS colour, default `silver`)
//!   and `--rinch-img-placeholder-border-width` (an absolute length or `em`,
//!   default `1px`; `0` draws no border).
//! - `--rinch-img-placeholder-background` (a colour, default `transparent`),
//!   filled inside the border.
//!
//! The alt text takes the element's own `color`, `font-*` and `line-height`,
//! so `img { color: gray }` styles it. It is shaped once, when the element is
//! restyled or its `alt` changes ([`AltText`], on its measure context), and
//! only re-broken to a width by layout and paint. A selector can target a
//! placeholder with no `src` directly (`img:not([src])`); a failed load has
//! no state a selector sees, which is why the look is carried by properties
//! read only in that state.
//!
//! The web backend is the browser's own `<img>`, so there the browser's
//! fallback applies and these properties do nothing.

use crate::computed_style::DimensionValue;
use crate::node::{Node, NodeTree};

/// The default border colour, CSS `silver` (Chrome's alt-text container).
pub const BORDER_COLOR: [u8; 3] = [0xc0, 0xc0, 0xc0];
/// The default border width, in CSS px.
pub const BORDER_WIDTH: f32 = 1.0;
/// The padding between the border and the alt text, in CSS px.
pub const PADDING: f32 = 1.0;

/// The smallest content box, on each axis, that an image with no `src` draws
/// its box in (Chrome 156: a ring at 3x100 and 300x3, nothing at 2x100).
pub const MIN_BOX_NO_SRC: f32 = 3.0;
/// The same after a failed load (Chrome 156: nothing at 17x100 or 300x17,
/// the ring and its icon at 18).
pub const MIN_BOX_FAILED: f32 = 18.0;

/// The least height of an inline alt placeholder's content box: Chrome
/// 156's alt container holds its 16x16 broken-image icon, so a cell's
/// `line-height: 0` or `font-size: 0` leaves it 16px tall (rinch draws no
/// icon, but keeps the box: the layout is Chrome's).
pub const MIN_INLINE_HEIGHT: f32 = 16.0;

/// The escape hatch: `none` turns placeholders off.
pub const PROPERTY: &str = "rinch-img-placeholder";
/// The border colour property.
pub const BORDER_COLOR_PROPERTY: &str = "rinch-img-placeholder-border-color";
/// The border width property.
pub const BORDER_WIDTH_PROPERTY: &str = "rinch-img-placeholder-border-width";
/// The background property.
pub const BACKGROUND_PROPERTY: &str = "rinch-img-placeholder-background";

/// What an `<img>` draws in place of a picture it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    /// Nothing: it has a picture, is loading one, is a tracking pixel, has
    /// neither a size nor alt text, or the app turned placeholders off.
    None,
    /// The sized box with a border and the alt text inside.
    Box,
    /// The alt text alone, as an inline box ([`AltBox`]).
    Inline,
}

/// An image's alt text, shaped once in its typography (at scale 1, on one
/// line) and re-broken to a width when it is measured or painted.
#[derive(Clone)]
pub struct AltText {
    /// What it was shaped from (the text, its typography, the font set), so
    /// a restyle that changes none of it keeps it.
    pub key: u64,
    /// The shaped text, broken onto one line.
    pub layout: std::sync::Arc<parley::Layout<peniko::Brush>>,
    /// Its one line's advance (max-content width).
    pub max_content: f32,
    /// Its widest unbreakable run (min-content width).
    pub min_content: f32,
    /// One line box's height.
    pub line_height: f32,
    /// The baseline from the top of a line box.
    pub baseline: f32,
    /// Whether the image is an inline alt placeholder, which the measure
    /// sizes from the text ([`Placeholder::Inline`]); a box's is only drawn.
    pub inline: bool,
}

impl PartialEq for AltText {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.inline == other.inline
    }
}

impl std::fmt::Debug for AltText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AltText")
            .field("max_content", &self.max_content)
            .field("min_content", &self.min_content)
            .field("line_height", &self.line_height)
            .field("inline", &self.inline)
            .finish()
    }
}

impl AltText {
    /// The text broken at `width` (CSS px): its layout.
    pub fn broken(&self, width: f32) -> parley::Layout<peniko::Brush> {
        let mut layout = (*self.layout).clone();
        layout.break_all_lines(Some(width.max(0.0)));
        layout
    }

    /// Its size broken at `width`: the widest line and the lines' height.
    pub fn wrapped(&self, width: f32) -> (f32, f32) {
        if width >= self.max_content {
            return (self.max_content, self.layout.height());
        }
        let layout = self.broken(width);
        (layout.width(), layout.height())
    }

    /// The last line's baseline from the top of its box, its lines broken at
    /// `width` and laid out from the box's top (every line one line box).
    pub fn last_baseline(&self, width: f32) -> f32 {
        self.wrapped(width).1 - (self.line_height - self.baseline)
    }
}

/// One unregistered custom property's computed token string on `node`, `name`
/// without its leading `--`.
pub(crate) fn custom_property(node: &Node, name: &str) -> Option<String> {
    let data = node.stylo_element_data.borrow();
    let primary = data.as_ref().and_then(|d| d.styles.get_primary())?;
    let atom = style::Atom::from(name);
    primary
        .custom_properties()
        .get(
            style::properties_and_values::rule::Descriptors::unregistered(),
            &atom,
        )
        .map(|v| v.to_variable_value().css.trim().to_string())
}

/// Whether the app turned placeholders off for `node`
/// (`--rinch-img-placeholder: none`).
pub fn turned_off(node: &Node) -> bool {
    custom_property(node, PROPERTY).is_some_and(|v| v.eq_ignore_ascii_case("none"))
}

/// Whether `node` (an `<img>`) has no picture and is not waiting for one: no
/// `src`, or one whose load failed.
pub fn has_no_picture(tree: &NodeTree, node: &Node) -> bool {
    match node.attributes.get("src") {
        None => true,
        Some(src) if src.trim_ascii().is_empty() => true,
        Some(src) => tree.image_cache.shows_failure(src),
    }
}

/// `node`'s `alt` text as it is drawn: white space collapsed and trimmed,
/// `None` when that leaves nothing.
pub fn alt_text(node: &Node) -> Option<String> {
    let alt = node.attributes.get("alt")?;
    let text = alt.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

/// A declared `width`/`height`: `None` for `auto`, else its length in px
/// when it is one (`Some(None)` for a percentage, `calc()` or keyword).
fn declared(v: &DimensionValue) -> Option<Option<f32>> {
    match v {
        DimensionValue::Auto => None,
        DimensionValue::Length(px) => Some(Some(*px)),
        _ => Some(None),
    }
}

/// What `node` draws in place of a picture ([`Placeholder`], the module's
/// three rules). Anything but an `<img>` is [`Placeholder::None`].
pub fn placeholder(tree: &NodeTree, node: &Node) -> Placeholder {
    if node.tag() != Some("img") || !has_no_picture(tree, node) || turned_off(node) {
        return Placeholder::None;
    }
    let cs = &node.computed_style;
    let (w, h) = (declared(&cs.width), declared(&cs.height));
    // A tracking pixel: every declared size a length of at most 1px.
    let sizes: Vec<Option<f32>> = [w, h].into_iter().flatten().collect();
    if !sizes.is_empty() && sizes.iter().all(|s| s.is_some_and(|px| px <= 1.0)) {
        return Placeholder::None;
    }
    // Both dimensions without the picture: both declared, or one and a ratio.
    let ratio = cs.aspect_ratio.used_ratio(None).map(|r| r.value);
    if (w.is_some() && h.is_some()) || ((w.is_some() || h.is_some()) && ratio.is_some()) {
        // Too small a box for Chrome to draw one in: with alt text, the text
        // inline instead, its size ignored, as Chrome shows it (review of
        // #2313: a failed 16x16 `alt="Facebook"` icon); without, nothing.
        return if alt_text(node).is_some() && below_min_box(node, w, h, ratio) {
            Placeholder::Inline
        } else {
            Placeholder::Box
        };
    }
    if alt_text(node).is_some() {
        Placeholder::Inline
    } else {
        Placeholder::None
    }
}

/// Whether the box `node`'s declared lengths give it (`w`, `h`, the other
/// through `ratio` when one is missing) is under [`min_box`] on an axis. A
/// percentage or keyword size is not known here and counts as large enough
/// (paint still draws no box under the minimum).
fn below_min_box(
    node: &Node,
    w: Option<Option<f32>>,
    h: Option<Option<f32>>,
    ratio: Option<f32>,
) -> bool {
    let (w, h) = (w.flatten(), h.flatten());
    let (w, h) = match (w, h, ratio) {
        (Some(w), None, Some(r)) => (Some(w), Some(w / r)),
        (None, Some(h), Some(r)) => (Some(h * r), Some(h)),
        _ => (w, h),
    };
    let min = min_box(node);
    w.is_some_and(|v| v < min) || h.is_some_and(|v| v < min)
}

/// Whether `node` is an alt placeholder whose declared size is ignored: a
/// box too small to draw ([`placeholder`]) shows its alt text inline, at the
/// text's size, as Chrome does. Its Taffy style drops its `width` and
/// `height` (`taffy_style_from_computed`).
pub fn ignores_size(tree: &NodeTree, node: &Node) -> bool {
    let cs = &node.computed_style;
    let (w, h) = (
        declared(&cs.width).is_some(),
        declared(&cs.height).is_some(),
    );
    let ratio = cs.aspect_ratio.used_ratio(None).is_some();
    placeholder(tree, node) == Placeholder::Inline && ((w && h) || ((w || h) && ratio))
}

/// The smallest content box, on each axis, that `node`'s box placeholder is
/// drawn in: [`MIN_BOX_NO_SRC`] with no `src`, [`MIN_BOX_FAILED`] after a
/// failed load.
pub fn min_box(node: &Node) -> f32 {
    match node.attributes.get("src") {
        Some(src) if !src.trim_ascii().is_empty() => MIN_BOX_FAILED,
        _ => MIN_BOX_NO_SRC,
    }
}

/// The alt text's layout in `node`'s typography, at scale 1, broken at
/// `max_width` (`None`: one line).
pub(crate) fn alt_layout(
    node: &Node,
    text: &str,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    max_width: Option<f32>,
) -> parley::Layout<peniko::Brush> {
    node.computed_style
        .build_parley_layout(text, 1.0, font_cx, layout_cx, max_width)
}

/// What [`AltText::key`] is made of: the text, every typography input
/// [`ComputedStyle::build_parley_layout`] reads, and the font set.
///
/// [`ComputedStyle::build_parley_layout`]: crate::computed_style::ComputedStyle::build_parley_layout
fn alt_key(node: &Node, text: &str, font_generation: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let cs = &node.computed_style;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    cs.hash_text_layout_inputs(&mut h);
    font_generation.hash(&mut h);
    h.finish()
}

/// `node`'s alt text `text` shaped ([`AltText`]).
pub(crate) fn shape_alt(
    node: &Node,
    text: &str,
    key: u64,
    inline: bool,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> AltText {
    let layout = alt_layout(node, text, font_cx, layout_cx, None);
    let (line_height, baseline, max_content) = layout
        .lines()
        .next()
        .map(|l| {
            let m = l.metrics();
            (m.line_height, m.baseline - m.block_min_coord, m.advance)
        })
        .unwrap_or_default();
    let mut narrow = layout.clone();
    narrow.break_all_lines(Some(0.0));
    AltText {
        key,
        min_content: narrow.width().min(max_content),
        layout: std::sync::Arc::new(layout),
        max_content,
        line_height,
        baseline,
        inline,
    }
}

/// What [`AltText`] `node` is owed, given the one it has (`have`): `None`
/// when it shows no alt text, `have` again when nothing it was made from
/// changed, else the text shaped anew.
pub(crate) fn wanted_alt(
    tree: &NodeTree,
    node: &Node,
    have: Option<&AltText>,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> Option<AltText> {
    let kind = placeholder(tree, node);
    if kind == Placeholder::None {
        return None;
    }
    let text = alt_text(node)?;
    let key = alt_key(node, &text, tree.font_generation);
    let inline = kind == Placeholder::Inline;
    if let Some(have) = have
        && have.key == key
        && have.inline == inline
    {
        return Some(have.clone());
    }
    Some(shape_alt(node, &text, key, inline, font_cx, layout_cx))
}

/// The measure answer of an inline alt placeholder (#2309): the content box
/// of an inline-block holding its text. Its width is its declared one when it
/// has one, else shrink-to-fit (the text's advance, down to the room it has,
/// never under its widest word); its height its declared one, else its
/// lines' at that width, and never under [`MIN_INLINE_HEIGHT`]. `min-*`/`max-*` clamp both, as
/// [`crate::replaced::measure`] does.
pub(crate) fn measure_inline_alt(
    alt: &AltText,
    known: taffy::Size<Option<f32>>,
    style: &taffy::Style,
    parent: taffy::Size<Option<f32>>,
    inherent: bool,
    available: taffy::Size<taffy::AvailableSpace>,
) -> taffy::Size<f32> {
    use taffy::ResolveOrZero;
    let m =
        |nw: f32, nh: f32| crate::replaced::measure((nw, nh), None, known, style, parent, inherent);
    // A width the style or Taffy decides answers the same whatever the
    // natural width is.
    let (lo, hi) = (m(0.0, 0.0).width, m(1.0e6, 0.0).width);
    let width = if lo == hi {
        lo
    } else {
        let calc = |_: *const (), _: f32| 0.0;
        let pb = (style.padding.resolve_or_zero(parent.width, calc)
            + style.border.resolve_or_zero(parent.width, calc))
        .sum_axes()
        .width;
        let room = match available.width {
            taffy::AvailableSpace::Definite(a) => (a - pb).max(0.0),
            taffy::AvailableSpace::MinContent => alt.min_content,
            taffy::AvailableSpace::MaxContent => alt.max_content,
        };
        m(alt.max_content.min(room.max(alt.min_content)), 0.0).width
    };
    let (_, lines) = alt.wrapped(width);
    m(width, lines.max(MIN_INLINE_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thresholds_are_chromes() {
        assert_eq!((MIN_BOX_NO_SRC, MIN_BOX_FAILED), (3.0, 18.0));
    }

    #[test]
    fn declared_sizes() {
        assert_eq!(declared(&DimensionValue::Auto), None);
        assert_eq!(declared(&DimensionValue::Length(3.0)), Some(Some(3.0)));
        assert_eq!(declared(&DimensionValue::Percent(0.5)), Some(None));
    }
}
