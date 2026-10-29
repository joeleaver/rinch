//! The natural size of a `<canvas>`, `<video>` and `<iframe>` (#1173).
//!
//! HTML gives a canvas natural dimensions from its `width` and `height`
//! attributes — 300 and 150 when absent or not a valid non-negative integer,
//! each on its own — and so a natural aspect ratio when neither is zero. A
//! `<video>` with no video data and no poster, and an `<iframe>`, have no
//! natural dimensions, so CSS sizes them with the **default object size**,
//! 300x150, and they have no aspect ratio. Measured in Chrome 153: a bare
//! `display: block` canvas or video is 300x150 in a 400px container, an
//! iframe 304x154 (its UA `border: 2px inset`); `width: 100px` gives a canvas
//! 100x50 and a video 100x150.
//!
//! rinch answers it through a Taffy **measure** ([`NodeContext::Replaced`]),
//! as it does an `<input>`'s line ([`crate::form_control`]), and marks the
//! element's Taffy style `item_is_replaced`, so a block container does not
//! stretch it to its own width (CSS 2.1 §10.3.4: a block-level replaced
//! element's `auto` width is its inline width). A flex container still
//! stretches it, as in Chrome. A **grid** container also still stretches it,
//! where Chrome keeps its natural size: Taffy's grid alignment does not read
//! `item_is_replaced` (#1280).
//!
//! A browser renders none of the three elements' children: a canvas's are
//! fallback content, a video's are `<source>`/`<track>` and fallback, an
//! iframe's are ignored. rinch matches that: every **element** child of one
//! is `display: none !important` by the UA sheet (#1288), and the element is
//! always an inline formatting root that the IFC pass leaves **hollow**, as it
//! does a `<textarea>` (#1159), so its children are detached from Taffy, it
//! keeps this context and no fallback text is shaped or painted. Its own
//! inner `display` is ignored ([`ignores_inner_display`]), so `display: flex`
//! lays it out as `block`.
//!
//! An **`<img>`** is the fourth replaced element here, for sizing only
//! (#788, #1150): its natural size is the decoded image's and it always has a
//! ratio, so both of its measure arms answer through [`measure`], and it is
//! `item_is_replaced` too ([`is_unstretched_replaced`]). A block `<img>` is
//! its natural width, `margin: 0 auto` centres it, and a lone style `width`
//! or `height` (or a `min-*`/`max-*` clamp) gives the other dimension through
//! the ratio. Its context stays `NodeContext::Image`, which the image cache
//! writes; an image not yet loaded has no natural size.
//!
//! An **`<svg>`** is the fifth (#1797, [`svg_context`]): its natural size is
//! its `width`/`height` attributes when they are absolute lengths, its ratio
//! theirs when both are, else its `viewBox`'s, and with no natural size it is
//! the default object size. Its children are its drawing, not CSS content,
//! so it is hollow ([`ignores_inner_display`]); `paint/svg.rs` draws them.
//! Its attributes are presentation attributes, mapped like the dimension
//! attributes below but read as CSS lengths ([`svg_length`]).
//!
//! **The `width` and `height` attributes of an `<img>`, `<video>` and
//! `<iframe>` are presentational hints** (#684), not natural dimensions:
//! HTML maps each to the property of its name, read by the rules for parsing
//! dimension values (`rinch_core::dom::parse_html_dimension`: pixels or a
//! percentage), in a declaration block Stylo cascades below every author rule
//! ([`dimension_hints_css`], `Node::presentational_hints`). On an `<img>` and
//! a `<video>` the pair also maps to `aspect-ratio: auto w / h`
//! ([`attribute_ratio`]) when both are lengths above zero: the ratio of an
//! image with no natural size (not loaded, or failed) and of a video, so
//! `<img width=100 height=50 style="width: 200px; height: auto">` is 200x100
//! before its image arrives. A loaded image's own ratio replaces it. The
//! ratio is a hint like the sizes (#1286), so an author `aspect-ratio`
//! replaces it: `aspect-ratio: auto; width: 200px; height: auto` on an
//! unloaded hinted image is 200x0, as in Chrome. The hints sit below every
//! author rule and above the UA sheet's normal rules.
//!
//! **`aspect-ratio`** (#1286) reaches the four elements' measure, not
//! Taffy ([`measures_aspect_ratio`]): the measure context carries the
//! computed value and [`measure`] weighs it against the natural ratio
//! (`AspectRatioValue::used_ratio`) — a `<ratio>` without `auto` replaces
//! it, `auto <ratio>` is used only when there is none (a video, an iframe,
//! an image not loaded). A `<ratio>` without `auto` relates the
//! `box-sizing` box's sizes; a natural ratio and `auto <ratio>`'s (the
//! attributes' hint included) the content box's. An unloaded or failed image with `alt`
//! text still reserves its attributes' box (Chrome lays out the alt text).
//!
//! [`is_unstretched_replaced`] also covers a line-sized `<input>` /
//! `<textarea>` (#1195): it is not sized here (its measure stays
//! [`NodeContext::FormControl`], in [`crate::form_control`]), only unstretched
//! — a `display: block` text control keeps its `size`/`cols` width in Chrome
//! instead of filling its container.
//!
//! **Not modelled:** a desktop `<video>` never holds video data (rinch's
//! `VideoViewport` is a `div`), so a poster or a loaded video's natural size
//! never applies. A `<video>` with a mapped ratio and both dimensions `auto`
//! is 300 wide and as tall as the ratio says, where Chrome 153 stretches it
//! to its container's width. The other presentational attributes (`hspace`,
//! `vspace`, `border`, `align` on an image; `<hr width>`; a table's) map to
//! nothing (#1419).

use crate::computed_style::{AspectRatioValue, UsedRatio};
use crate::node::{Node, NodeContext, NodeTree};
use taffy::{MaybeResolve, ResolveOrZero};

/// The CSS default object size (css-images-3 §4.3), and a canvas's default
/// bitmap size.
pub const DEFAULT_OBJECT_SIZE: (f32, f32) = (300.0, 150.0);

/// Whether `tag` is sized from a natural or default object size by this
/// module.
pub fn has_default_object_size(tag: &str) -> bool {
    matches!(tag, "canvas" | "video" | "iframe")
}

/// Whether `node` is one of [`has_default_object_size`]'s elements, which
/// render none of their children.
pub fn is_replaced_without_content(node: &Node) -> bool {
    node.tag().is_some_and(has_default_object_size)
}

/// Whether `node`'s `aspect-ratio` is read by [`measure`] (an `<img>` and
/// [`is_replaced_without_content`]'s three) rather than by Taffy (#1286).
pub fn measures_aspect_ratio(node: &Node) -> bool {
    node.tag()
        .is_some_and(|tag| tag == "img" || tag == "svg" || has_default_object_size(tag))
}

/// Whether `node` renders none of its children and so has no inner display
/// of its own — a text control ([`crate::form_control::is_value_control`]) or
/// one of [`has_default_object_size`]'s elements (#1178, #1288). A browser
/// lays such an element out as a replaced box whatever its `display` says
/// inside: `display: flex` on a canvas is a block-level box, `inline-grid` an
/// inline-level one. rinch maps `flex`/`grid` to block and
/// `inline-flex`/`inline-grid` to inline-block in its
/// [`crate::node::DisplayMode`], so it stays a block container: the IFC pass
/// makes it a hollow root, which detaches its children from Taffy and so
/// reaches its measure. Its Taffy `display` may still say `Flex`; Taffy runs
/// the measure of any childless node whatever its display.
pub fn ignores_inner_display(node: &Node) -> bool {
    crate::form_control::is_value_control(node)
        || is_replaced_without_content(node)
        || matches!(node.tag(), Some("select" | "svg"))
}

/// Whether a block container sizes `node` as a replaced element rather than
/// stretching it to its own width (CSS 2.1 §10.3.4): an `<img>` (#788),
/// [`is_replaced_without_content`]'s three, and a line-sized `<input>` /
/// `<textarea>` (#1195) — Chrome treats a text-entry control as "auto width
/// fits content" even at `display: block`.
/// [`crate::form_control::form_control_content_height`] answers `Some` for
/// exactly the controls this applies to: not `checkbox`/`radio`/`range`/
/// `color`/`image`/`hidden`, whose own intrinsic sizes this crate does not
/// model and whose block-stretch behaviour is out of scope for #1195. Read
/// into Taffy's `item_is_replaced`.
pub fn is_unstretched_replaced(node: &Node) -> bool {
    node.tag()
        .is_some_and(|tag| tag == "img" || tag == "svg" || has_default_object_size(tag))
        || crate::form_control::form_control_content_height(node).is_some()
}

/// Whether HTML maps the `width` and `height` attributes of `tag` to the
/// properties of those names (#684), or SVG does (an `<svg>`'s are
/// presentation attributes, #1797). A canvas's are its bitmap's size
/// instead ([`replaced_context`]).
pub fn maps_dimension_attributes(tag: &str) -> bool {
    matches!(tag, "img" | "video" | "iframe" | "svg")
}

fn dimension_attribute(node: &Node, name: &str) -> Option<rinch_core::dom::HtmlDimension> {
    rinch_core::dom::parse_html_dimension(node.attributes.get(name)?)
}

/// The declarations the `width` and `height` attributes of `node` map to, as
/// CSS text, or `None` when neither parses. The caller has checked
/// [`maps_dimension_attributes`].
pub(crate) fn dimension_hints_css(node: &Node) -> Option<String> {
    use rinch_core::dom::HtmlDimension;
    use std::fmt::Write;
    let mut css = String::new();
    if node.tag() == Some("svg") {
        // A presentation attribute: the property's own grammar, a bare
        // number being pixels (#1797).
        for name in ["width", "height"] {
            // A negative length is invalid as a property, which Stylo
            // drops; it is still a natural size of 0 ([`svg_natural_length`]).
            if let Some((v, unit)) = node.attributes.get(name).and_then(|v| svg_length(v)) {
                let unit = if unit.is_empty() { "px" } else { unit.as_str() };
                write!(css, "{name}: {v}{unit};").unwrap();
            }
        }
        return (!css.is_empty()).then_some(css);
    }
    for name in ["width", "height"] {
        match dimension_attribute(node, name) {
            Some(HtmlDimension::Length(v)) => write!(css, "{name}: {v}px;").unwrap(),
            Some(HtmlDimension::Percentage(v)) => write!(css, "{name}: {v}%;").unwrap(),
            None => {}
        }
    }
    // The pair's ratio, `aspect-ratio: auto w / h` (#684), cascaded like the
    // sizes so that an author `aspect-ratio` replaces it (#1286).
    if let (Some(HtmlDimension::Length(w)), Some(HtmlDimension::Length(h))) = (
        dimension_attribute(node, "width"),
        dimension_attribute(node, "height"),
    ) && attribute_ratio(node).is_some()
    {
        write!(css, "aspect-ratio: auto {w} / {h};").unwrap();
    }
    (!css.is_empty()).then_some(css)
}

/// The aspect ratio (width / height) the `width` and `height` attributes of
/// an `<img>` or `<video>` map to: both must parse as lengths, not
/// percentages, and be above zero. `None` for any other element.
pub fn attribute_ratio(node: &Node) -> Option<f32> {
    use rinch_core::dom::HtmlDimension;
    if !matches!(node.tag(), Some("img" | "video")) {
        return None;
    }
    match (
        dimension_attribute(node, "width")?,
        dimension_attribute(node, "height")?,
    ) {
        (HtmlDimension::Length(w), HtmlDimension::Length(h)) if w > 0.0 && h > 0.0 => {
            let r = (w / h) as f32;
            (r.is_finite() && r > 0.0).then_some(r)
        }
        _ => None,
    }
}

/// An `<svg>` `width`/`height` attribute read as a CSS `<length-percentage>`
/// in which a bare number is pixels (SVG 2 §8.2, a presentation attribute):
/// the number and its unit, lowercased — empty for a bare number, `%`, or a length unit. CSS's
/// number grammar (`1e1`, `+14`, `.5`; not `5.` or `14 px`) after leading and
/// trailing ASCII whitespace. `None` for anything else, which maps nothing.
pub(crate) fn svg_length(value: &str) -> Option<(f64, String)> {
    let v = value.trim_matches(|c: char| c.is_ascii_whitespace());
    let b = v.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let int_start = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i > int_start;
    if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        digits = true;
    }
    if !digits {
        return None;
    }
    // An exponent only when digits follow it; otherwise the `e` starts the
    // unit (`1em`).
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            i = j;
        }
    }
    let number: f64 = v[..i].parse().ok()?;
    if !number.is_finite() {
        return None;
    }
    let unit = v[i..].to_ascii_lowercase();
    let known = matches!(
        unit.as_str(),
        "" | "%"
            | "px"
            | "em"
            | "rem"
            | "ex"
            | "ch"
            | "vw"
            | "vh"
            | "vmin"
            | "vmax"
            | "cm"
            | "mm"
            | "q"
            | "in"
            | "pt"
            | "pc"
    );
    known.then_some((number, unit))
}

/// An `<svg>`'s natural size along one axis: its attribute when that is an
/// absolute length (or `em`, against its own font size), else none — a
/// percentage, a viewport or font-relative unit other than `em`, or no
/// attribute.
fn svg_natural_length(node: &Node, name: &str) -> Option<f32> {
    let (v, unit) = svg_length(node.attributes.get(name)?)?;
    let px = match unit.as_str() {
        "" | "px" => v,
        "in" => v * 96.0,
        "cm" => v * 96.0 / 2.54,
        "mm" => v * 96.0 / 25.4,
        "q" => v * 96.0 / 101.6,
        "pt" => v * 96.0 / 72.0,
        "pc" => v * 16.0,
        "em" => v * f64::from(node.computed_style.font_size),
        _ => return None,
    } as f32;
    // A negative attribute maps no property, but Chrome 155 reads it as a
    // natural size of 0: `height="-3"` alone is 300x0, beside a `viewBox`
    // the ratio still answers (`width="30" height="-3"` is 30x15).
    px.is_finite().then_some(px.max(0.0))
}

/// The measure context of an `<svg>` (#1797): CSS's default sizing of a
/// replaced element whose natural dimensions are its `width` and `height`
/// attributes ([`svg_natural_length`]) and whose natural ratio is theirs
/// when both are, else its `viewBox`'s. A missing dimension comes through the
/// ratio from the other, else from the default object size (300x150); with
/// neither dimension but a ratio the width is the containing block's
/// ([`SVG_FILL_WIDTH`]). Measured in Chrome 155: `width=14` alone is 14x150,
/// `viewBox="0 0 16 8"` alone 400x200 in a 400px block, nothing 300x150.
fn svg_context(node: &Node) -> NodeContext {
    let (dw, dh) = DEFAULT_OBJECT_SIZE;
    let view_box = node.attributes.get("viewBox").and_then(|v| {
        let n: Vec<f64> = v
            .split(|c: char| c.is_ascii_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<f64>())
            .collect::<Result<_, _>>()
            .ok()?;
        match n[..] {
            [_, _, w, h] if w > 0.0 && h > 0.0 && (w / h).is_finite() => Some((w / h) as f32),
            _ => None,
        }
    });
    let nw = svg_natural_length(node, "width");
    let nh = svg_natural_length(node, "height");
    // Both attributes' ratio first, the viewBox's after: Chrome 155 lays
    // `width="30" height="10" viewBox="0 0 16 16"` at `width: 60px; height:
    // auto` out 60x20.
    let ratio = match (nw, nh) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(w / h),
        _ => view_box,
    };
    let (width, height) = match (nw, nh, ratio) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some(r)) => (w, w / r),
        (Some(w), None, None) => (w, dh),
        (None, Some(h), Some(r)) => (h * r, h),
        (None, Some(h), None) => (dw, h),
        (None, None, Some(r)) => (SVG_FILL_WIDTH, dw / r),
        (None, None, None) => (dw, dh),
    };
    NodeContext::Replaced {
        width,
        height,
        ratio,
        aspect: node.computed_style.aspect_ratio,
    }
}

/// The natural width [`svg_context`] gives an `<svg>` with a ratio and no
/// natural dimension: [`measure`] reads it as the parent's width (less the
/// element's padding and border), or the default object size's when that is
/// not definite — which an inline `<svg>`, sized as an atomic inline, always
/// finds, so it is 300 wide where Chrome 155 fills its containing block. Infinity rather than `NaN`, so that two
/// contexts compare equal and a cascade leaves Taffy's cache alone.
pub const SVG_FILL_WIDTH: f32 = f32::INFINITY;

/// An `<img>`'s natural size and the ratio it is measured with: its own once
/// loaded, with no natural size before, and the ratio its computed
/// `aspect-ratio` makes of that ([`AspectRatioValue::used_ratio`]).
pub(crate) fn image_natural_size(
    width: u32,
    height: u32,
    aspect: AspectRatioValue,
) -> ((f32, f32), Option<UsedRatio>) {
    if width == 0 || height == 0 {
        ((0.0, 0.0), aspect.used_ratio(None))
    } else {
        let (w, h) = (width as f32, height as f32);
        ((w, h), aspect.used_ratio(Some(w / h)))
    }
}

/// The measure context `node` is owed, or `None` for any other element.
pub fn replaced_context(node: &Node) -> Option<NodeContext> {
    let tag = node.tag()?;
    if tag == "svg" {
        return Some(svg_context(node));
    }
    if !has_default_object_size(tag) {
        return None;
    }
    let (dw, dh) = DEFAULT_OBJECT_SIZE;
    let aspect = node.computed_style.aspect_ratio;
    if tag != "canvas" {
        // No natural ratio: a video's `width`/`height` attributes reach its
        // computed `aspect-ratio` as a presentational hint (#684, #1286).
        return Some(NodeContext::Replaced {
            width: dw,
            height: dh,
            ratio: None,
            aspect,
        });
    }
    // HTML: a canvas's `width`/`height` content attributes are parsed with
    // the rules for non-negative integers; a missing or invalid one takes
    // its default (300, 150), independently of the other.
    let dim = |name: &str, default: f32| {
        node.attributes
            .get(name)
            .and_then(|v| rinch_core::dom::parse_html_non_negative_integer(v))
            .map_or(default, |v| v as f32)
    };
    let (width, height) = (dim("width", dw), dim("height", dh));
    Some(NodeContext::Replaced {
        width,
        height,
        ratio: (width > 0.0 && height > 0.0).then(|| width / height),
        aspect,
    })
}

/// Make the Taffy measure context of `node_id` say what [`replaced_context`]
/// says, and answer whether it had to change (the caller then owes a layout:
/// `set_node_context` has already marked the Taffy node dirty). Called when
/// the element is created and by every cascade of it, which a `width` or
/// `height` write asks for, and whatever moves the element's computed
/// `aspect-ratio` (#1286). For an `<img>` it keeps the context's `aspect`
/// current instead.
pub(crate) fn sync_replaced_measure(tree: &mut NodeTree, node_id: crate::node::RawNodeId) -> bool {
    let Some(node) = tree.nodes.get(node_id) else {
        return false;
    };
    let Some(taffy_id) = node.taffy_id else {
        return false;
    };
    if node.tag() == Some("img") {
        let want = node.computed_style.aspect_ratio;
        let Some(NodeContext::Image { aspect, .. }) = tree.taffy.get_node_context_mut(taffy_id)
        else {
            return false;
        };
        if *aspect == want {
            return false;
        }
        *aspect = want;
        let _ = tree.taffy.mark_dirty(taffy_id);
        return true;
    }
    let Some(want) = replaced_context(node) else {
        return false;
    };
    let same = match (&want, tree.taffy.get_node_context(taffy_id)) {
        (
            NodeContext::Replaced {
                width: aw,
                height: ah,
                ratio: ar,
                aspect: aa,
            },
            Some(NodeContext::Replaced {
                width: bw,
                height: bh,
                ratio: br,
                aspect: ba,
            }),
        ) => aw == bw && ah == bh && ar == br && aa == ba,
        _ => false,
    };
    if same {
        return false;
    }
    let _ = tree.taffy.set_node_context(taffy_id, Some(want));
    true
}

/// The measure answer for [`NodeContext::Replaced`]: the **content-box** size
/// of a replaced element with natural size `natural` and aspect ratio `ratio`
/// (width / height, [`AspectRatioValue::used_ratio`]: the natural one unless
/// `aspect-ratio` declares one without `auto`, #1286; an unloaded image with a
/// mapped or declared ratio has one and a `(0, 0)` natural size; a `<ratio>`
/// without `auto` relates the `box-sizing` box's sizes, any other the content
/// box's), shared by the root compute and the atomic-inline
/// sizer. Taffy adds the padding and border, and keeps any dimension it
/// already knows.
///
/// A dimension known from `known` (Taffy's, border-box) or, when `inherent`,
/// from the style's `width`/`height` gives the other through the ratio — its
/// value **after** its own axis's `min-*`/`max-*` clamp, so `width: 100%;
/// max-width: 100px` in 300px is 100x75 for a 4:3 image, not 100x225; with
/// neither known the natural size is constrained by `min-*`/`max-*` as CSS
/// 2.1 §10.4's table does, the ratio carrying one axis's clamp to the other
/// (`max-width: 100px` gives a canvas 100x50, where Taffy's own clamp would
/// leave it 100x150). Percentages resolve against `parent`, a percentage
/// against an indefinite size being `auto`.
pub(crate) fn measure(
    natural: (f32, f32),
    ratio: Option<UsedRatio>,
    known: taffy::Size<Option<f32>>,
    style: &taffy::Style,
    parent: taffy::Size<Option<f32>>,
    inherent: bool,
) -> taffy::Size<f32> {
    let calc = |_: *const (), _: f32| 0.0;
    let pb = (style.padding.resolve_or_zero(parent.width, calc)
        + style.border.resolve_or_zero(parent.width, calc))
    .sum_axes();
    let border_box = style.box_sizing == taffy::BoxSizing::BorderBox;
    // A style length to a content-box length.
    let content = |v: f32, pb: f32| if border_box { (v - pb).max(0.0) } else { v };

    let (mut w, mut h) = (
        known.width.map(|v| (v - pb.width).max(0.0)),
        known.height.map(|v| (v - pb.height).max(0.0)),
    );
    let none = taffy::Size {
        width: None,
        height: None,
    };
    let (min, max) = if inherent {
        let size = style.size.maybe_resolve(parent, calc);
        w = w.or(size.width.map(|v| content(v, pb.width)));
        h = h.or(size.height.map(|v| content(v, pb.height)));
        let min = style.min_size.maybe_resolve(parent, calc);
        let max = style.max_size.maybe_resolve(parent, calc);
        (
            taffy::Size {
                width: min.width.map(|v| content(v, pb.width)),
                height: min.height.map(|v| content(v, pb.height)),
            },
            taffy::Size {
                width: max.width.map(|v| content(v, pb.width)),
                height: max.height.map(|v| content(v, pb.height)),
            },
        )
    } else {
        (none, none)
    };
    let clamp = |v: f32, lo: Option<f32>, hi: Option<f32>| {
        let v = hi.map_or(v, |hi| v.min(hi));
        lo.map_or(v, |lo| v.max(lo))
    };
    let clamp_w = |v: f32| clamp(v, min.width, max.width);
    let clamp_h = |v: f32| clamp(v, min.height, max.height);

    let (nw, nh) = natural;
    // An `<svg>` with a ratio and no natural size fills its containing
    // block's width ([`SVG_FILL_WIDTH`]).
    let nw = if nw.is_infinite() {
        parent
            .width
            .map_or(DEFAULT_OBJECT_SIZE.0, |p| (p - pb.width).max(0.0))
    } else {
        nw
    };
    let (width, height) = if let Some(UsedRatio {
        value: r,
        box_sizing_box,
    }) = ratio
    {
        // A `<ratio>` without `auto` relates sizes of the `box-sizing` box
        // (css-sizing-4 §2), which with `border-box` is the content box grown
        // by `pb`; a natural ratio, or `auto <ratio>`'s, the content box's.
        let (ox, oy) = if box_sizing_box && border_box {
            (pb.width, pb.height)
        } else {
            (0.0, 0.0)
        };
        let h_of = |w: f32| ((w + ox) / r - oy).max(0.0);
        let w_of = |h: f32| ((h + oy) * r - ox).max(0.0);
        match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, clamp_h(h_of(clamp_w(w)))),
            (None, Some(h)) => (clamp_w(w_of(clamp_h(h))), h),
            (None, None) => {
                let w = clamp_w(nw);
                let h = h_of(w);
                let hc = clamp_h(h);
                if hc != h {
                    (clamp_w(w_of(hc)), hc)
                } else {
                    (w, h)
                }
            }
        }
    } else {
        (
            w.unwrap_or_else(|| clamp_w(nw)),
            h.unwrap_or_else(|| clamp_h(nh)),
        )
    };
    taffy::Size { width, height }
}
