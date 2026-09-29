//! The intrinsic content height of a text-entry form control (#297).
//!
//! An `<input>` and a `<textarea>` hold their value in an **attribute**, not in
//! a child, so they are childless however much text they show and nothing in
//! the box tree gives them a height. A browser sizes them from their own
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
/// not a text-entry control.
///
/// The `<input>` types are the ones the text engine edits (#812's list, the
/// same one the text context menu uses). `checkbox`, `radio`, `range`, `color`
/// and the button types have intrinsic sizes of their own that this does not
/// model; they keep the zero content box they always had.
pub(crate) fn form_control_content_height(node: &Node) -> Option<f32> {
    let rows = match node.tag()? {
        "textarea" => node
            .attributes
            .get("rows")
            .and_then(|r| r.trim().parse::<f32>().ok())
            .filter(|r| *r >= 1.0)
            .map_or(2.0, f32::floor),
        "input" => {
            let text_like = node.attributes.get("type").is_none_or(|t| {
                matches!(
                    t.trim().to_ascii_lowercase().as_str(),
                    "" | "text" | "search" | "url" | "tel" | "email" | "password" | "number"
                )
            });
            if !text_like {
                return None;
            }
            1.0
        }
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
    let have = match tree.taffy.get_node_context(taffy_id) {
        Some(NodeContext::FormControl { content_height }) => Some(*content_height),
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
pub(crate) fn measure(
    content_height: f32,
    known: taffy::Size<Option<f32>>,
) -> taffy::Size<f32> {
    taffy::Size {
        width: known.width.unwrap_or(0.0),
        height: known.height.unwrap_or(content_height),
    }
}
