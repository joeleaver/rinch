//! What an `<img>` with no picture draws in its place (#2309).
//!
//! An `<img>` has no picture when it has no `src` (absent, or empty or white
//! space only) or its load **failed** ([`crate::image_cache::ImageCache::is_failed`]).
//! One still loading has none yet and draws nothing, as in a browser. Mail
//! is the case that matters: Postrinch strips the `src` of every held-back
//! remote picture and keeps its `width`, `height`, `alt` and `style`.
//!
//! Joe's rules ([`placeholder`]):
//!
//! 1. **Sized** — its computed `width` or `height` is not `auto` (from the
//!    attributes, which map to both, #684, or from CSS): a [`Placeholder::Box`],
//!    the size layout already gives it (a sized placeholder keeps its
//!    attributes' ratio under `max-width: 100%; height: auto`, the
//!    `aspect-ratio: auto w / h` hint of #684), a faint border inside its
//!    content box and the `alt` text inside that, wrapped to the box and
//!    clipped to it, in the element's own font and colour. The ring is what
//!    Chrome 156 draws for a sized image with no alt text (1px `#c0c0c0` on
//!    the content box's edge, 1px of padding); for one *with* alt text
//!    Chrome shows the text inline and ignores the size (it draws this box
//!    only in quirks mode). Rinch draws the box, so held-back pictures keep
//!    their place in a layout.
//! 2. **`alt` but no size**: [`Placeholder::Inline`], the alt text on one line
//!    in the element's font, laid out as an atomic inline box the size of the
//!    text's line box ([`AltBox`]) whose baseline is the text's, so it sits
//!    in the line as a `<span>` of the same text does in Chrome.
//! 3. **Neither, or a declared size of at most 1x1** (a tracking pixel):
//!    [`Placeholder::None`], nothing drawn and the declared size only.
//!    Chrome draws a 16x16 broken-image icon for a failed one; this is
//!    Joe's deliberate exception. Rinch draws that icon nowhere (Chrome puts
//!    it before the alt text of a failed or missing image too).
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
//!   and `--rinch-img-placeholder-border-width` (a `px` length, default
//!   `1px`; `0` draws no border).
//! - `--rinch-img-placeholder-background` (a colour, default `transparent`),
//!   filled inside the border.
//!
//! The alt text takes the element's own `color`, `font-*` and `line-height`,
//! so `img { color: gray }` styles it. A selector can target a placeholder
//! with no `src` directly (`img:not([src])`); a failed load has no state a
//! selector sees, which is why the look is carried by properties read only
//! in that state.
//!
//! The web backend is the browser's own `<img>`, so there the browser's
//! fallback applies (Chrome's inline alt text for a missing `src`, its
//! 16x16 icon for a failed tracking pixel) and these properties do nothing.

use crate::computed_style::DimensionValue;
use crate::node::{Node, NodeTree};

/// The default border colour, CSS `silver` (Chrome's alt-text container).
pub const BORDER_COLOR: [u8; 3] = [0xc0, 0xc0, 0xc0];
/// The default border width, in CSS px.
pub const BORDER_WIDTH: f32 = 1.0;
/// The padding between the border and the alt text, in CSS px.
pub const PADDING: f32 = 1.0;

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

/// The box of an inline alt placeholder's text, in CSS px: its advance, its
/// line box's height and the baseline from the top of that box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AltBox {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
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
        Some(src) => tree.image_cache.is_failed(src),
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
    let sizes: Vec<Option<f32>> = [&cs.width, &cs.height]
        .into_iter()
        .filter_map(declared)
        .collect();
    if !sizes.is_empty() {
        // A tracking pixel: every declared size a length of at most 1px.
        if sizes.iter().all(|s| s.is_some_and(|px| px <= 1.0)) {
            return Placeholder::None;
        }
        return Placeholder::Box;
    }
    if alt_text(node).is_some() {
        Placeholder::Inline
    } else {
        Placeholder::None
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

/// The [`AltBox`] of `node`'s alt text `text`: its one line's advance and
/// line box, so it adds to its line exactly what a `<span>` of the text
/// does (its `line-height`, not its font's content area, which may be
/// taller).
pub(crate) fn alt_box(
    node: &Node,
    text: &str,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> AltBox {
    let layout = alt_layout(node, text, font_cx, layout_cx, None);
    let Some(m) = layout.lines().next().map(|l| *l.metrics()) else {
        return AltBox {
            width: 0.0,
            height: 0.0,
            baseline: 0.0,
        };
    };
    AltBox {
        width: m.advance,
        height: m.line_height,
        baseline: m.baseline - m.block_min_coord,
    }
}

/// The [`AltBox`] `node` is owed: `Some` only for [`Placeholder::Inline`].
pub(crate) fn wanted_alt_box(
    tree: &NodeTree,
    node: &Node,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> Option<AltBox> {
    if placeholder(tree, node) != Placeholder::Inline {
        return None;
    }
    let text = alt_text(node)?;
    Some(alt_box(node, &text, font_cx, layout_cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_sizes() {
        assert_eq!(declared(&DimensionValue::Auto), None);
        assert_eq!(declared(&DimensionValue::Length(3.0)), Some(Some(3.0)));
        assert_eq!(declared(&DimensionValue::Percent(0.5)), Some(None));
    }
}
