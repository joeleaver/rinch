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
//! before its image arrives. A loaded image's own ratio replaces it. rinch
//! has no `aspect-ratio` property; the ratio goes from the attributes to the
//! measure context, and an author `aspect-ratio` declaration still does
//! nothing (#1286) — so it cannot switch the mapped ratio off either, where
//! Chrome 153 gives `aspect-ratio: auto; width: 200px; height: auto` on an
//! unloaded hinted image 200x0. The hints sit below every author rule and
//! above the UA sheet's normal rules. An unloaded or failed image with `alt`
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
    crate::form_control::is_value_control(node) || is_replaced_without_content(node)
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
        .is_some_and(|tag| tag == "img" || has_default_object_size(tag))
        || crate::form_control::form_control_content_height(node).is_some()
}

/// Whether HTML maps the `width` and `height` attributes of `tag` to the
/// properties of those names (#684). A canvas's are its bitmap's size
/// instead ([`replaced_context`]).
pub fn maps_dimension_attributes(tag: &str) -> bool {
    matches!(tag, "img" | "video" | "iframe")
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
    for name in ["width", "height"] {
        match dimension_attribute(node, name) {
            Some(HtmlDimension::Length(v)) => write!(css, "{name}: {v}px;").unwrap(),
            Some(HtmlDimension::Percentage(v)) => write!(css, "{name}: {v}%;").unwrap(),
            None => {}
        }
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

/// An `<img>`'s natural size and the ratio it is measured with: its own once
/// loaded, else `hint_ratio` with no natural size.
pub(crate) fn image_natural_size(
    width: u32,
    height: u32,
    hint_ratio: Option<f32>,
) -> ((f32, f32), Option<f32>) {
    if width == 0 || height == 0 {
        ((0.0, 0.0), hint_ratio)
    } else {
        let (w, h) = (width as f32, height as f32);
        ((w, h), Some(w / h))
    }
}

/// The measure context `node` is owed, or `None` for any other element.
pub fn replaced_context(node: &Node) -> Option<NodeContext> {
    let tag = node.tag()?;
    if !has_default_object_size(tag) {
        return None;
    }
    let (dw, dh) = DEFAULT_OBJECT_SIZE;
    if tag != "canvas" {
        return Some(NodeContext::Replaced {
            width: dw,
            height: dh,
            ratio: attribute_ratio(node),
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
    })
}

/// Make the Taffy measure context of `node_id` say what [`replaced_context`]
/// says, and answer whether it had to change (the caller then owes a layout:
/// `set_node_context` has already marked the Taffy node dirty). Called when
/// the element is created and by every cascade of it, which a `width` or
/// `height` write asks for. For an `<img>` it keeps the context's
/// `hint_ratio` current instead.
pub(crate) fn sync_replaced_measure(tree: &mut NodeTree, node_id: usize) -> bool {
    let Some(node) = tree.nodes.get(node_id) else {
        return false;
    };
    let Some(taffy_id) = node.taffy_id else {
        return false;
    };
    if node.tag() == Some("img") {
        let want = attribute_ratio(node);
        let Some(NodeContext::Image { hint_ratio, .. }) = tree.taffy.get_node_context_mut(taffy_id)
        else {
            return false;
        };
        if *hint_ratio == want {
            return false;
        }
        *hint_ratio = want;
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
            },
            Some(NodeContext::Replaced {
                width: bw,
                height: bh,
                ratio: br,
            }),
        ) => aw == bw && ah == bh && ar == br,
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
/// (width / height; an unloaded image with a mapped ratio has one and a
/// `(0, 0)` natural size), shared by the root compute and the atomic-inline
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
    ratio: Option<f32>,
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
    let (width, height) = if let Some(r) = ratio {
        match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, clamp_h(clamp_w(w) / r)),
            (None, Some(h)) => (clamp_w(clamp_h(h) * r), h),
            (None, None) => {
                let w = clamp_w(nw);
                let h = w / r;
                let hc = clamp_h(h);
                if hc != h {
                    (clamp_w(hc * r), hc)
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
