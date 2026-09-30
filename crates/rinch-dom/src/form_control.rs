//! The intrinsic content height of a line-sized control (#297): `<input>`,
//! `<textarea>`, and a blockified `<br>`.
//!
//! An `<input>` and a `<textarea>` hold their value in an **attribute**, not in
//! a child, so they are childless however much text they show and nothing in
//! the box tree gives them a height. (A textarea's text *child* is its default
//! value, and does not size it either — [`is_value_control`].) A browser sizes them from their own
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
//! attribute and the font's average character width, which is separate work
//! (#1177).

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
        // HTML's rules for parsing non-negative integers, and 2 unless that
        // gives more than zero (#1153). A float parse took `"1e1"` as 10 rows
        // and `"inf"` / `"1e30"` as infinite / NaN, which made every box laid
        // out after the textarea NaN.
        "textarea" => node
            .attributes
            .get("rows")
            .and_then(|r| rinch_core::dom::parse_html_non_negative_integer(r))
            .filter(|&r| r > 0)
            .map_or(2.0, |r| r as f32),
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
    let have = match tree.taffy.get_node_context(taffy_id) {
        Some(NodeContext::FormControl { content_height }) => Some(*content_height),
        // A context this module did not write is never overwritten: an IFC
        // root's `InlineRoot`, or a text leaf's. A control whose children form
        // an inline formatting context is not one of those — the IFC pass
        // leaves it hollow, carrying the `FormControl` context this function
        // compares against (#1159) — so its `rows`, line height and `type` are
        // tracked here like a childless control's.
        Some(NodeContext::InlineRoot(_)) | Some(NodeContext::Text(_)) => return false,
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

/// Whether `node` is an `<input>` or a `<textarea>`: an element whose children
/// are not content (#1159).
///
/// A browser renders neither element's children. A `<textarea>`'s direct text
/// children are its **default value** ([`control_value`]); an `<input>` has no
/// content model and its children are ignored. Measured in Chrome 153: a
/// textarea holding "hello world foo" as a child is the size of an empty one
/// and draws the text once, as its value; an `<input>` with a text child draws
/// nothing. So when such a control's children form an inline formatting
/// context, the IFC pass leaves it **hollow**: its Taffy node carries
/// [`NodeContext::FormControl`] (or no context) instead of `InlineRoot`, so it
/// is measured exactly as a childless control is, and `build_ifc_layouts`
/// shapes no text for it, so nothing lays its children out or paints them.
/// `paint_input_value` draws the value. Only inline children reach that: a
/// block-level element child is still laid out as content (#1178).
pub fn is_value_control(node: &Node) -> bool {
    matches!(node.tag(), Some("input" | "textarea"))
}

/// The text a field holds: its `value` attribute, else a `<textarea>`'s
/// **child text content** (HTML: the concatenation of its direct `Text`
/// children — its default value), else `None`. Any element with a `value`
/// attribute answers it, so the desktop shell reads every field through this,
/// custom `data-oninput` controls included.
///
/// rinch keeps a control's live value in its `value` attribute (typing writes
/// it, and so does `value_fn`), so an attribute that is present wins even when
/// it is empty — the web's dirty value flag, read from the other end: once a
/// field has been typed into or written, its children no longer show. An
/// `<input>` has no default value in its children, so it answers only its
/// attribute. `None` for any other element.
pub fn control_value(
    nodes: &slab::Slab<Node>,
    node_id: usize,
) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    let node = nodes.get(node_id)?;
    if let Some(v) = node.attributes.get("value") {
        return Some(Cow::Borrowed(v.as_str()));
    }
    if node.tag() != Some("textarea") {
        return None;
    }
    let mut texts = node
        .children
        .iter()
        .filter_map(|&c| nodes.get(c))
        .filter_map(|c| c.text_content());
    let Some(first) = texts.next() else {
        return Some(Cow::Borrowed(""));
    };
    match texts.next() {
        None => Some(Cow::Borrowed(first)),
        Some(second) => {
            let mut all = String::from(first);
            all.push_str(second);
            texts.for_each(|t| all.push_str(t));
            Some(Cow::Owned(all))
        }
    }
}

/// The Taffy context a hollow control root carries in place of `InlineRoot`
/// ([`is_value_control`]): what [`sync_form_control_measure`] gives a
/// childless one.
pub(crate) fn hollow_control_context(node: &Node) -> Option<NodeContext> {
    form_control_content_height(node)
        .map(|content_height| NodeContext::FormControl { content_height })
}
