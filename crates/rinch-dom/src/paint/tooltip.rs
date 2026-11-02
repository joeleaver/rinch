//! The `title` tooltip (issue #1646): what an element's advisory `title`
//! looks like when the desktop runtime shows it, and where it goes.
//!
//! It is not a DOM node. A tooltip drawn as part of the document would need a
//! layout pass to appear, would sit in `<body>`'s child list where an app's
//! selectors, `dom_tree` and hit testing see it, and would be laid out again
//! on every pass while hidden. Instead the runtime keeps a [`Tooltip`] —
//! the text shaped once with Parley, the box measured from it — and paints it
//! straight into the frame after the document, like the drag ghost and the
//! inspect highlight. Showing, moving or hiding it runs no style, layout or
//! IFC pass; it costs the paint of its own rect.
//!
//! The colours come from the theme's custom properties on `<body>` (which
//! inherits them from `:root`): `--rinch-color-surface` for the background,
//! `--rinch-color-text` for the text and `--rinch-color-border` for the edge,
//! so a dark theme gets a dark tooltip. Without the theme they fall back to
//! the same light values the built-in text context menu uses. The font is the
//! theme's `--rinch-font-family`, else `sans-serif`, at [`TOOLTIP_FONT_SIZE`].

use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect, RoundedRect, Stroke};

use super::painter::{PaintShape, Painter};
use crate::node::{NodeTree, RawNodeId};

/// The text size, in CSS px. A native tooltip is set smaller than body text.
pub const TOOLTIP_FONT_SIZE: f32 = 12.0;
/// The line height, as a multiple of [`TOOLTIP_FONT_SIZE`].
pub const TOOLTIP_LINE_HEIGHT: f32 = 1.4;
/// Horizontal padding inside the border, CSS px.
pub const TOOLTIP_PAD_X: f32 = 8.0;
/// Vertical padding inside the border, CSS px.
pub const TOOLTIP_PAD_Y: f32 = 4.0;
/// The border width, CSS px.
pub const TOOLTIP_BORDER: f32 = 1.0;
/// The corner radius, CSS px.
pub const TOOLTIP_RADIUS: f32 = 4.0;
/// Text longer than this wraps, CSS px (the content box, padding excluded).
pub const TOOLTIP_MAX_TEXT_WIDTH: f32 = 400.0;
/// How far right of the pointer the tooltip's left edge goes, CSS px.
pub const TOOLTIP_OFFSET_X: f32 = 10.0;
/// How far below the pointer the tooltip's top edge goes, CSS px — clear of
/// the arrow cursor, which hangs about 20px below its hot spot.
pub const TOOLTIP_OFFSET_Y: f32 = 20.0;

const FALLBACK_BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];
const FALLBACK_TEXT: [u8; 3] = [0x21, 0x25, 0x29];
const FALLBACK_BORDER: [u8; 3] = [0xde, 0xe2, 0xe6];

/// The advisory title that applies to `node`: the title of the node itself
/// or of its nearest ancestor that has one, as HTML defines it. A title that
/// is present but empty — or only white space — ends the search with no
/// tooltip: "if this attribute is omitted from an element, then it implies
/// that the title attribute of the nearest ancestor … is relevant", so an
/// empty one is relevant and says nothing (Chrome 155 agrees, for `""` and
/// for white space).
///
/// An element's title is [`own_title`]'s answer. Returns the node it belongs
/// to, and the text.
pub fn advisory_title(tree: &NodeTree, node: RawNodeId) -> Option<(RawNodeId, String)> {
    let mut cur = Some(node);
    while let Some(id) = cur {
        let n = tree.nodes.get(id)?;
        if let Some(title) = own_title(tree, id) {
            if title.trim().is_empty() {
                return None;
            }
            return Some((id, title));
        }
        cur = n.parent;
    }
    None
}

/// The title `id` carries itself, if any: an HTML element's `title`
/// attribute, or an SVG element's first `<title>` child's text. As in Chrome
/// 155, an SVG element's `title` *attribute* is no title (SVG has none).
pub fn own_title(tree: &NodeTree, id: RawNodeId) -> Option<String> {
    let n = tree.nodes.get(id)?;
    let tag = n.tag()?;
    if tag == "svg" || crate::attr_name::is_svg_content_tag(tag) {
        let title = n
            .children
            .iter()
            .filter_map(|&c| tree.nodes.get_live(c))
            .find(|c| c.tag() == Some("title"))?;
        let mut text = String::new();
        for &c in &title.children {
            if let Some(t) = tree.nodes.get_live(c).and_then(|c| c.text_content()) {
                text.push_str(t);
            }
        }
        return Some(text);
    }
    n.attributes.get("title").cloned()
}

/// Where a tooltip of `size` goes for a pointer at `pointer`, in a
/// `viewport`-sized window (all logical px): below and to the right of the
/// pointer; left of it when that would cross the right edge, above it when it
/// would cross the bottom edge; and never past the top or left edge.
pub fn place_tooltip(pointer: (f32, f32), size: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    let (px, py) = pointer;
    let (w, h) = size;
    let (vw, vh) = viewport;
    let mut x = px + TOOLTIP_OFFSET_X;
    if x + w > vw {
        // Flip to the left of the pointer, then slide inside the window if
        // even that does not fit.
        x = (px - w).min(vw - w);
    }
    let mut y = py + TOOLTIP_OFFSET_Y;
    if y + h > vh {
        y = py - h - 4.0;
    }
    (x.max(0.0), y.max(0.0))
}

/// A shaped tooltip: its text, colours and the size of its box.
pub struct Tooltip {
    text: String,
    layout: parley::Layout<Brush>,
    background: AlphaColor<Srgb>,
    border: AlphaColor<Srgb>,
    size: (f32, f32),
}

impl std::fmt::Debug for Tooltip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tooltip")
            .field("text", &self.text)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Tooltip {
    /// Shape `text` in `tree`'s body font and theme colours.
    ///
    /// White space is kept as written, and a line feed breaks the line, as a
    /// browser's title tooltip does.
    pub fn new(
        tree: &NodeTree,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        text: &str,
    ) -> Self {
        // The theme's UI font, else the platform's sans-serif — not the
        // page's own `font-family`, which a tooltip is no part of (a browser
        // sets its tooltips in the system UI font).
        let family = custom_property(tree, tree.body_id, "rinch-font-family")
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "sans-serif".to_string());
        let color = |name: &str, fallback: [u8; 3]| {
            custom_property(tree, tree.body_id, name)
                .and_then(|v| crate::layout::parse_color(&v))
                .map(|c| c.to_rgba8())
                .map(|c| AlphaColor::from_rgba8(c.r, c.g, c.b, c.a))
                .unwrap_or_else(|| AlphaColor::from_rgb8(fallback[0], fallback[1], fallback[2]))
        };
        let background = color("rinch-color-surface", FALLBACK_BACKGROUND);
        let foreground = color("rinch-color-text", FALLBACK_TEXT);
        let border = color("rinch-color-border", FALLBACK_BORDER);

        let font_family = crate::fonts::parley_text_family(font_cx, &family, text);
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(parley::style::StyleProperty::FontSize(TOOLTIP_FONT_SIZE));
        builder.push_default(parley::style::StyleProperty::LineHeight(
            parley::style::LineHeight::FontSizeRelative(TOOLTIP_LINE_HEIGHT),
        ));
        builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(
            foreground,
        )));
        font_family.push_to(&mut builder);
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(TOOLTIP_MAX_TEXT_WIDTH));
        layout.align(
            parley::layout::Alignment::Start,
            parley::layout::AlignmentOptions::default(),
        );
        let text_w = layout.width().min(TOOLTIP_MAX_TEXT_WIDTH).ceil();
        let text_h = layout.height().ceil();
        let edge = TOOLTIP_BORDER * 2.0;
        Tooltip {
            text: text.to_string(),
            layout,
            background,
            border,
            size: (
                text_w + TOOLTIP_PAD_X * 2.0 + edge,
                text_h + TOOLTIP_PAD_Y * 2.0 + edge,
            ),
        }
    }

    /// The text the tooltip shows.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The box's size, logical px, border included.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// The framebuffer rect [`Tooltip::paint`] touches with its box at
    /// `origin` (logical px), in device px and clamped to a `w` x `h`
    /// surface, a pixel of anti-aliasing included. `None` off the surface.
    pub fn device_rect(&self, scale: f64, origin: (f32, f32), w: u32, h: u32) -> Option<Rect> {
        let (x, y) = (origin.0 as f64 * scale, origin.1 as f64 * scale);
        let r = Rect::new(
            (x - 1.0).floor(),
            (y - 1.0).floor(),
            (x + self.size.0 as f64 * scale + 1.0).ceil(),
            (y + self.size.1 as f64 * scale + 1.0).ceil(),
        )
        .intersect(Rect::new(0.0, 0.0, w as f64, h as f64));
        (r.width() > 0.0 && r.height() > 0.0).then_some(r)
    }

    /// Draw the tooltip with its box's top-left corner at `origin` (logical
    /// px), at `scale` device px per logical px.
    pub fn paint(&self, painter: &mut dyn Painter, scale: f64, origin: (f32, f32)) {
        let s = scale;
        let (x, y) = (origin.0 as f64 * s, origin.1 as f64 * s);
        let (w, h) = (self.size.0 as f64 * s, self.size.1 as f64 * s);
        let r = TOOLTIP_RADIUS as f64 * s;
        let outer = RoundedRect::new(x, y, x + w, y + h, r);
        painter.fill_color(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            self.background,
            &PaintShape::RoundedRect(outer),
        );
        // The stroke is centred on its path: inset by half its width so it
        // lies inside the box.
        let bw = TOOLTIP_BORDER as f64 * s;
        let half = bw / 2.0;
        let edge = RoundedRect::new(x + half, y + half, x + w - half, y + h - half, r - half);
        painter.stroke_color(
            &Stroke::new(bw),
            Affine::IDENTITY,
            self.border,
            &PaintShape::RoundedRect(edge),
        );
        let tx = x + ((TOOLTIP_BORDER + TOOLTIP_PAD_X) as f64 * s);
        let ty = y + ((TOOLTIP_BORDER + TOOLTIP_PAD_Y) as f64 * s);
        super::text::draw_text(
            painter,
            &super::text::PaintText::new(&self.layout),
            Affine::translate((tx.round(), ty.round())),
            s,
            None,
            None,
            None,
            None,
            None,
        );
    }
}

/// One unregistered custom property's computed token string on `node`,
/// `name` without its leading `--`.
fn custom_property(tree: &NodeTree, node: RawNodeId, name: &str) -> Option<String> {
    let n = tree.nodes.get(node)?;
    let data = n.stylo_element_data.borrow();
    let primary = data.as_ref().and_then(|d| d.styles.get_primary())?;
    let atom = style::Atom::from(name);
    primary
        .custom_properties()
        .get(
            style::properties_and_values::registry::PropertyRegistrationData::unregistered(),
            &atom,
        )
        .map(|v| v.to_variable_value().css.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_right_of_the_pointer_when_it_fits() {
        assert_eq!(
            place_tooltip((100.0, 50.0), (80.0, 24.0), (800.0, 600.0)),
            (110.0, 70.0)
        );
    }

    #[test]
    fn left_of_the_pointer_at_the_right_edge() {
        // 700 + 10 + 120 = 830 > 800: flipped to end at the pointer.
        assert_eq!(
            place_tooltip((700.0, 50.0), (120.0, 24.0), (800.0, 600.0)),
            (580.0, 70.0)
        );
    }

    #[test]
    fn above_the_pointer_at_the_bottom_edge() {
        // 590 + 20 + 24 > 600: above, 4px clear of the hot spot.
        assert_eq!(
            place_tooltip((100.0, 590.0), (80.0, 24.0), (800.0, 600.0)),
            (110.0, 562.0)
        );
    }

    #[test]
    fn never_past_the_left_or_top_edge() {
        // Wider than the window: pinned at its left edge.
        assert_eq!(
            place_tooltip((30.0, 5.0), (900.0, 700.0), (800.0, 600.0)),
            (0.0, 0.0)
        );
    }
}
