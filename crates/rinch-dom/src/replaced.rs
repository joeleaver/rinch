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
//! iframe's are ignored. rinch matches that only for children that are all
//! text or non-atomic inlines: their inline formatting context leaves the
//! element **hollow**, as it does a `<textarea>` (#1159), so it keeps this
//! context and no text is shaped or painted. A **block-level or atomic
//! inline** child is still laid out and painted (#1288), and a block child
//! also takes the element's size away: a Taffy node with children never
//! calls its measure, so the element is sized by its children.
//!
//! **Not modelled:** a desktop `<video>` never holds video data (rinch's
//! `VideoViewport` is a `div`), so a poster or a loaded video's natural size
//! never applies; the `width`/`height` attributes of a `<video>` or an
//! `<iframe>` are presentational hints (#684), not natural dimensions, and
//! are not read.

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

/// Whether a block container sizes `node` as a replaced element rather than
/// stretching it to its own width (CSS 2.1 §10.3.4): an `<img>` (#788) and
/// [`is_replaced_without_content`]'s three. Read into Taffy's
/// `item_is_replaced`.
pub fn is_unstretched_replaced(node: &Node) -> bool {
    node.tag()
        .is_some_and(|tag| tag == "img" || has_default_object_size(tag))
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
            ratio: false,
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
        ratio: width > 0.0 && height > 0.0,
    })
}

/// Make the Taffy measure context of `node_id` say what [`replaced_context`]
/// says, and answer whether it had to change (the caller then owes a layout:
/// `set_node_context` has already marked the Taffy node dirty). Called when
/// the element is created and when a canvas's `width`/`height` changes.
pub(crate) fn sync_replaced_measure(tree: &mut NodeTree, node_id: usize) -> bool {
    let Some(node) = tree.nodes.get(node_id) else {
        return false;
    };
    let Some(taffy_id) = node.taffy_id else {
        return false;
    };
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
/// of a replaced element with natural size `natural` (and a natural aspect
/// ratio when `ratio`), shared by the root compute and the atomic-inline
/// sizer. Taffy adds the padding and border, and keeps any dimension it
/// already knows.
///
/// A dimension known from `known` (Taffy's, border-box) or, when `inherent`,
/// from the style's `width`/`height` gives the other through the ratio; with
/// neither known the natural size is constrained by `min-*`/`max-*` as CSS
/// 2.1 §10.4's table does, the ratio carrying one axis's clamp to the other
/// (`max-width: 100px` gives a canvas 100x50, where Taffy's own clamp would
/// leave it 100x150). Percentages resolve against `parent`, a percentage
/// against an indefinite size being `auto`.
pub(crate) fn measure(
    natural: (f32, f32),
    ratio: bool,
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
    let (width, height) = if ratio {
        let r = nw / nh;
        match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, clamp_h(w / r)),
            (None, Some(h)) => (clamp_w(h * r), h),
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
