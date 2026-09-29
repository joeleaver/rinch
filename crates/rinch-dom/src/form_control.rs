//! The intrinsic content height of a line-sized control (#297): `<input>`,
//! `<textarea>`, and a blockified `<br>`.
//!
//! An `<input>` and a `<textarea>` hold their value in an **attribute**, not in
//! a child, so they are childless however much text they show and nothing in
//! the box tree gives them a height. (A textarea's text *child* is its default
//! value, and does not size it either — [`inline_root_override`].) A browser sizes them from their own
//! metrics: one line box for a single-line `<input>`, `rows` line boxes for a
//! `<textarea>` (2 when `rows` is absent or invalid), plus padding and border.
//! Measured in Chrome 153 with `line-height: 20px; padding: 6px 10px; border:
//! 1px`: an `<input>` is 34px tall whether it is inline, a flex item or
//! `display: block`, and a two-row `<textarea>` 54px.
//!
//! rinch answers that through a Taffy **measure**, not a `min-height`
//! (`NodeContext::FormControl`), because the difference is observable: a
//! column-flex item's `min-height: 0` lets the control shrink below its
//! content (Chrome: 10px in a 10px column), and `min-height: auto` does not
//! (20px). A `min-height` written by rinch cannot tell the two apart. Taffy
//! adds the padding and border to what the measure answers, so the measure
//! answers the content box alone.
//!
//! **Width is not measured** (the measure answers 0, which is what a childless
//! box had before): Chrome's intrinsic width comes from the `size`/`cols`
//! attribute and the font's average character width, which is separate work.

use crate::node::{Node, NodeContext, NodeTree};

/// The content-box height the control at `node` is owed, or `None` when it is
/// not a line-sized control.
///
/// - `<textarea>`: `rows` lines, 2 when `rows` is absent or below 1. (`rows` is
///   read with `str::parse::<f32>`, not HTML's integer rules — #1153.)
/// - `<input>`: one line for **every** type but `checkbox`, `radio`, `range`,
///   `color`, `image` and `hidden`. That is HTML's rule read from the other
///   end: an absent or invalid `type` is the Text state, and the date/time
///   family, the button types and `file` all lay out one line in Chrome 153
///   (20–22px at `line-height: 20px`). The six excluded types have intrinsic
///   sizes of their own (13x13, 16, 27, the image's) that this does not model.
///   `type` is ASCII case-insensitive.
/// - `<br>`: one line. An in-flow `<br>` is IFC content and never reaches a
///   Taffy measure; one that is blockified — a flex item, `display: block` — is
///   a line tall in Chrome 153, which the removed empty-block floor used to
///   give it (#296).
pub(crate) fn form_control_content_height(node: &Node) -> Option<f32> {
    let rows = match node.tag()? {
        "textarea" => node
            .attributes
            .get("rows")
            .and_then(|r| r.trim().parse::<f32>().ok())
            .filter(|r| *r >= 1.0)
            .map_or(2.0, f32::floor),
        "input" => {
            let sized_otherwise = node.attributes.get("type").is_some_and(|t| {
                matches!(
                    t.trim().to_ascii_lowercase().as_str(),
                    "checkbox" | "radio" | "range" | "color" | "image" | "hidden"
                )
            });
            if sized_otherwise {
                return None;
            }
            1.0
        }
        "br" => 1.0,
        _ => return None,
    };
    Some(rows * node.computed_style.line_height_px())
}

/// Make the Taffy measure context of `node_id` say what
/// [`form_control_content_height`] says, and answer whether it had to change
/// (the caller then owes a layout: `set_node_context` has already marked the
/// Taffy node dirty).
///
/// Called wherever a node's computed style is (re)written — the cascade and
/// the two animation ticks — because the line height is part of the answer and
/// is not a Taffy property: a `line-height` or `font-size` change moves no
/// Taffy style, so nothing else would dirty the node.
pub(crate) fn sync_form_control_measure(tree: &mut NodeTree, node_id: usize) -> bool {
    let Some(node) = tree.nodes.get(node_id) else {
        return false;
    };
    let Some(taffy_id) = node.taffy_id else {
        return false;
    };
    let want = form_control_content_height(node);
    // An `<input>` whose `type` just left the line types still owes a
    // re-measure: its InlineRoot measure stops answering the rows height.
    let control_tag = matches!(node.tag(), Some("input" | "textarea"));
    let have = match tree.taffy.get_node_context(taffy_id) {
        Some(NodeContext::FormControl { content_height }) => Some(*content_height),
        // A context this module did not write is never overwritten. The one
        // reachable case is a `<textarea>` whose value arrived as a text
        // **child** (`textarea { "hi" }`, `set_text_content`, parsed HTML): it
        // is an IFC root, the IFC pass owns its context, and the `InlineRoot`
        // measure's *height* is `form_control_content_height` for it (see
        // [`inline_root_override`]). Overwriting it here made the two passes
        // trade the slot — one line after a structural pass, `rows` lines after
        // the next restyle (PR #1152 review, F1). What can still change under
        // it is the `rows`/line height/`type` this restyle read — including a
        // `type` that leaves the line types, where `want` is `None` and the
        // shaped height must come back (review round 2, R2-2) — and Taffy
        // caches that measure, so the node is marked dirty on every restyle of
        // an `input`/`textarea` carrying one. That is a Taffy compute per
        // restyle of such a control, colour-only ones included; the shape is
        // rare, and the cure is to stop its text child forming an IFC (#1159).
        Some(NodeContext::InlineRoot(_)) | Some(NodeContext::Text(_)) => {
            if control_tag {
                let _ = tree.taffy.mark_dirty(taffy_id);
                return true;
            }
            return false;
        }
        _ => None,
    };
    if want == have {
        return false;
    }
    match want {
        Some(content_height) => {
            let _ = tree
                .taffy
                .set_node_context(taffy_id, Some(NodeContext::FormControl { content_height }));
        }
        // A control that stopped being one (its `type` became `checkbox`):
        // take back only the context this module wrote.
        None if have.is_some() => {
            let _ = tree.taffy.set_node_context(taffy_id, None);
        }
        None => return false,
    }
    true
}

/// The measure answer for [`NodeContext::FormControl`], shared by the root
/// compute and the atomic-inline sizer.
pub(crate) fn measure(content_height: f32, known: taffy::Size<Option<f32>>) -> taffy::Size<f32> {
    taffy::Size {
        width: known.width.unwrap_or(0.0),
        height: known.height.unwrap_or(content_height),
    }
}

/// The **height** an `InlineRoot` measure answers for a root that is a
/// line-sized control, in place of its shaped height: a `<textarea>`'s text
/// children are its default **value**, not content it is sized by (Chrome 153:
/// a two-row textarea holding "abc" as a child is 40px, the same as an empty
/// one). `None` for every ordinary IFC root. Only the height: the width stays
/// the shaped text's, so a control whose width is intrinsic (inline, a flex-row
/// item, in an `inline-block`) does not collapse to 0 under its own text
/// (review round 2, R2-1). Both measure sites apply it on the cached path too.
/// Chrome takes that width from `cols`/`size` instead; the text-child shape's
/// remaining problems are #1159.
pub(crate) fn inline_root_override(nodes: &slab::Slab<Node>, root_id: usize) -> Option<f32> {
    form_control_content_height(nodes.get(root_id)?)
}
