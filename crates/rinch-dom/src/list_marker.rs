//! Outside list markers (#1356): where an `outside` marker box goes.
//!
//! `resolve_list_marker` generates an item's marker as a `<span>` at the start
//! of the `<li>`. For `list-style-position: inside` that span is ordinary
//! inline content and is laid out where it stands. For `outside` (the
//! initial value) it is a static block for style, paint, clipping and
//! stacking (the UA sheet, keyed on the `rinch-list-marker` attribute), so
//! it paints in its item's flow and is clipped by whatever clips the item, as
//! a `::marker` is; but its Taffy box is **out of flow** (`position:
//! absolute` in Taffy only, `taffy_style_from_computed`, and
//! `Node::is_out_of_flow` for the inline formatting context), so it takes no
//! line of the item's and no width from the item's lines, and this module
//! places it after every layout, as Chrome places an outside `::marker`:
//!
//! - **vertically**, its baseline on the baseline of the item's **first line
//!   box**, found in tree order through the item's in-flow block descendants
//!   (`<li><div>…</div></li>` and `<li><p>…</p></li>` put it beside the
//!   block's first line, a padded or bordered block's included; a block with
//!   no line, an empty one or a picture, is passed over to the next). A table
//!   is not entered: its marker's baseline is its first row's content bottom.
//!   An item with no line box at all puts the marker's own line at the top
//!   of its content box, and is made at least that line high
//!   (`Node::marker_line_floor`); an atomic inline around the item (an
//!   `inline-block` list) is queued to be sized again with it.
//! - **horizontally**, outside the item's border box on its inline-start
//!   side, in the list's padding, whatever the item's own padding and
//!   border. A `decimal` marker (`"1. "`) ends at that edge. A
//!   `disc`/`circle`/`square` one is drawn as a glyph where Chrome draws a
//!   shape, so the glyph is centred where Chrome centres the shape: Blink's
//!   `ListMarker` puts a symbol's box `ascent * 2 / 3 + 8` px before that edge
//!   and draws a `(ascent * 2 / 3 + 1) / 2` px square in it, one pixel in
//!   (integer arithmetic on the rounded ascent; measured in Chrome 156 at 12,
//!   16, 20, 24 and 32px).
//!
//! The position is written into the marker's `layout` here and nowhere else:
//! the read-back keeps the one this pass wrote (`read_layout_results`), and
//! the out-of-flow passes do not look at a marker (it is not positioned,
//! and `place_static_after_lines` returns early), so a layout
//! that does not move the item's first line does not move or repaint its
//! marker.
//!
//! Known differences from Chrome: a marker whose own line is taller than the
//! first line it sits on (an item's `line-height` above its block's) does
//! not push the item's content down.

use crate::RinchDocument;
use crate::computed_style::{DisplayValue, TablePart};
use crate::node::{NodeContext, NodeTree, RawNodeId};

/// What [`RinchDocument::place_list_markers`] needs to know about an outside
/// marker that its box does not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutsideMarker {
    /// A `disc`/`circle`/`square` glyph (placed by Blink's symbol rule),
    /// rather than text such as `"1. "` (which ends at the content edge).
    pub symbol: bool,
}

/// A first line box: its baseline, from the border-box top of the box it was
/// looked for in, and its font's ascent and descent (`0` for a table's
/// synthesized one).
#[derive(Debug, Clone, Copy)]
struct Line {
    baseline: f32,
    ascent: f32,
    descent: f32,
}

/// The marker's own font, which a flex or grid item's text is placed by (see
/// [`first_line`]): its ascent and descent at `font_size`.
#[derive(Debug, Clone, Copy)]
struct Strut {
    ascent: f32,
    descent: f32,
    font_size: f32,
}

/// Whether `text` has anything but collapsible white space (a no-break space
/// is content, as in a browser).
fn is_content(text: &str) -> bool {
    text.chars()
        .any(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
}

/// Whether a line of `layout` holds anything a line box is made of: text
/// other than collapsible white space, or an inline box with a size (an
/// atomic inline or an image; an out-of-flow box's placeholder has none).
/// A no-break space is content, as in a browser.
fn has_line_content(text: &str, layout: &parley::layout::Layout<peniko::Brush>) -> bool {
    if is_content(text) {
        return true;
    }
    layout.lines().any(|line| {
        line.items().any(|item| match item {
            parley::layout::PositionedLayoutItem::InlineBox(b) => b.width > 0.0 || b.height > 0.0,
            _ => false,
        })
    })
}

fn first_line_of(layout: &parley::layout::Layout<peniko::Brush>, top: f32) -> Option<Line> {
    let baseline = layout.lines().next()?.metrics().baseline;
    // The root span's first available font, as Blink sizes a symbol by.
    let (ascent, descent) = layout
        .root_span_metrics()
        .map_or((0.0, 0.0), |m| (m.ascent, m.descent));
    Some(Line {
        baseline: top + baseline,
        ascent,
        descent,
    })
}

/// The first line box in the Taffy subtree at `taffy_id`, skipping `skip`
/// (the marker itself) and every box out of flow. See the module doc for
/// which boxes are entered.
///
/// A flex or grid item's own text (a `NodeContext::Text` leaf) keeps no
/// lines this pass could read in every state (its shaped layout is kept only
/// when the compute measured it at the width it has), so its first line is
/// placed by `strut`, the marker's font scaled to the text's size and
/// half-leaded in the text's line height as CSS lines are: exact when the text
/// is in the item's font, and the same answer whatever was cached.
fn first_line(
    tree: &NodeTree,
    taffy_id: taffy::NodeId,
    skip: Option<taffy::NodeId>,
    strut: Option<Strut>,
    depth: usize,
) -> Option<Line> {
    // A pathological nesting just finds no line.
    if depth > 256 {
        return None;
    }
    let taffy = &tree.taffy;
    let layout = taffy.layout(taffy_id).ok()?;
    let top = layout.padding.top + layout.border.top;
    match taffy.get_node_context(taffy_id) {
        Some(NodeContext::InlineRoot(root)) => {
            let il = tree.nodes.get(*root)?.text_layout.as_deref()?;
            if !has_line_content(&il.text_content, &il.layout) {
                return None;
            }
            return first_line_of(&il.layout, top);
        }
        Some(NodeContext::Text(text)) => {
            let strut = strut?;
            if !is_content(&text.content) {
                return None;
            }
            let style = tree
                .nodes
                .get(text.node_id)
                .and_then(|n| n.parent)
                .and_then(|p| tree.nodes.get(p))
                .map(|p| &p.computed_style)?;
            let scale = style.font_size / strut.font_size.max(1.0);
            let (ascent, descent) = (strut.ascent * scale, strut.descent * scale);
            let leading = style.line_height_px() - (ascent + descent);
            return Some(Line {
                baseline: top + leading / 2.0 + ascent,
                ascent,
                descent,
            });
        }
        Some(NodeContext::Element) | None => {}
        // A replaced element or a form control has no line box to give.
        Some(_) => return None,
    }
    if let Some(&dom) = tree.taffy_map.get(&taffy_id)
        && tree
            .nodes
            .get(dom)
            .is_some_and(|n| n.computed_style.table_part == TablePart::Table)
    {
        // Chrome 156 lines the marker up with the content-box bottom of the
        // table's first row's cells (`border-spacing: 4px` and `padding: 6px`
        // around a 20px line put it at 30), whatever the row holds and
        // however it is aligned: `vertical-align: top` in a 60px cell puts it
        // at 60. Rows generate no box here, so the first cell stands for its
        // row.
        let count = taffy::TraversePartialTree::child_count(taffy, taffy_id);
        for i in 0..count {
            let child = taffy.child_at_index(taffy_id, i).ok()?;
            let is_cell = tree
                .taffy_map
                .get(&child)
                .and_then(|&c| tree.nodes.get(c))
                .is_some_and(|n| n.table_cell.is_some());
            if is_cell {
                let cl = taffy.layout(child).ok()?;
                return Some(Line {
                    baseline: cl.location.y + cl.size.height - cl.padding.bottom - cl.border.bottom,
                    ascent: 0.0,
                    descent: 0.0,
                });
            }
        }
        return None;
    }
    let count = taffy::TraversePartialTree::child_count(taffy, taffy_id);
    for i in 0..count {
        let Ok(child) = taffy.child_at_index(taffy_id, i) else {
            continue;
        };
        if Some(child) == skip {
            continue;
        }
        let in_flow = taffy.style(child).is_ok_and(|s| {
            s.display != taffy::Display::None
                && s.position != taffy::Position::Absolute
                && s.float == taffy::Float::None
        });
        if !in_flow {
            continue;
        }
        if let Some(line) = first_line(tree, child, None, strut, depth + 1) {
            let y = taffy.layout(child).map(|l| l.location.y).unwrap_or(0.0);
            return Some(Line {
                baseline: y + line.baseline,
                ..line
            });
        }
    }
    None
}

impl RinchDocument {
    /// Place every outside list marker (module doc). Answers whether an
    /// item's [`Node::marker_line_floor`](crate::node::Node) changed, which
    /// changes its Taffy style: the caller lays the document out again.
    ///
    /// Only the markers that can have moved are placed (review of #2271):
    /// the ones generated since the last pass, and those of the items above a
    /// box the read-back saw move or resize, or an IFC root that got new
    /// lines (`NodeTree::marker_moves`). A marker's place is relative to its
    /// item and depends only on what is inside it, so a box that only moved
    /// counts from its parent up, and only a resize counts the box itself: a
    /// list pushed down by a box above it places none.
    pub(crate) fn place_list_markers(&mut self) -> bool {
        let tree = &mut self.tree;
        if tree.outside_markers.is_empty() && tree.marker_moves.is_empty() {
            return false;
        }
        let mut markers = std::mem::take(&mut tree.outside_markers);
        let moves = std::mem::take(&mut tree.marker_moves);
        if !moves.is_empty() {
            tree.marker_walk_stamp += 1;
            let stamp = tree.marker_walk_stamp;
            for (id, resized) in moves {
                let mut at = if resized {
                    Some(id)
                } else {
                    tree.nodes.get(id).and_then(|n| n.parent)
                };
                while let Some(cur) = at {
                    let Some(node) = tree.nodes.get_mut(cur) else {
                        break;
                    };
                    // Everything above a node already walked was walked too.
                    if node.marker_walk == stamp {
                        break;
                    }
                    node.marker_walk = stamp;
                    at = node.parent;
                    if let Some(&first) = tree.nodes[cur].children.first()
                        && tree
                            .nodes
                            .get(first)
                            .is_some_and(|c| c.outside_marker.is_some())
                    {
                        markers.push(first);
                    }
                }
            }
        }
        markers.sort_unstable();
        markers.dedup();
        let mut floors_changed = false;
        for m in markers {
            let live = self
                .tree
                .nodes
                .get_live(m)
                .is_some_and(|n| n.outside_marker.is_some() && n.parent.is_some());
            if live {
                self.tree.perf.bump(crate::perf::Counter::ListMarkersPlaced);
                floors_changed |= self.place_list_marker(m);
            }
        }
        floors_changed
    }

    fn place_list_marker(&mut self, marker_id: RawNodeId) -> bool {
        let tree = &self.tree;
        let marker = &tree.nodes[marker_id];
        let Some(info) = marker.outside_marker else {
            return false;
        };
        let Some(li_id) = marker.parent else {
            return false;
        };
        let (Some(marker_taffy), Some(li_taffy)) = (marker.taffy_id, tree.nodes[li_id].taffy_id)
        else {
            return false;
        };
        if marker.computed_style.display == DisplayValue::None
            || tree.nodes[li_id].computed_style.display == DisplayValue::None
        {
            return false;
        }
        let Ok(li_layout) = tree.taffy.layout(li_taffy).copied() else {
            return false;
        };
        let (width, height) = (marker.layout.width, marker.layout.height);
        let own = first_line(tree, marker_taffy, None, None, 0);
        let strut = own.map(|l| Strut {
            ascent: l.ascent,
            descent: l.descent,
            font_size: marker.computed_style.font_size,
        });
        let content = first_line(tree, li_taffy, Some(marker_taffy), strut, 0);
        let content_top = li_layout.padding.top + li_layout.border.top;
        // The item's border-box edge, not its content edge: Chrome 156 hangs
        // the marker in the same place whatever the item's own padding and
        // border (`<li style="padding: 10px; border: 3px solid">`).
        let (start_left, start_right) = (0.0, li_layout.size.width);
        let ltr = crate::ifc::css_direction_is_ltr(&tree.nodes, li_id);

        let own_baseline = own.map_or(height, |l| l.baseline);
        let y = match content {
            Some(line) => line.baseline - own_baseline,
            None => content_top,
        };
        let x = if info.symbol {
            // Blink's symbol box and square, from the rounded ascent.
            let ascent = own.map_or(0.0, |l| l.ascent).round() as i32;
            let offset = ascent * 2 / 3;
            let side = (offset + 1) / 2;
            let reach = (offset + 8) as f32 - 1.0 - side as f32 / 2.0;
            let centre = if ltr {
                start_left - reach
            } else {
                start_right + reach
            };
            centre - width / 2.0
        } else if ltr {
            start_left - width
        } else {
            start_right
        };
        let (x, y) = (x.round(), y.round());
        let floor = content.is_none().then_some(height);

        let node = &mut self.tree.nodes[marker_id];
        if node.layout.x != x || node.layout.y != y {
            node.layout.x = x;
            node.layout.y = y;
            self.tree.paint_dirty_nodes.push(marker_id);
        }
        if self.tree.nodes[li_id].marker_line_floor != floor {
            self.tree.nodes[li_id].marker_line_floor = floor;
            self.rewrite_taffy_style(li_id);
            // An atomic inline around the item is sized by a compute of its
            // own, which the layout this asks for does not run unless the box
            // is queued: an empty `<li>` in an `inline-block` list stayed 0
            // tall on the first layout and took its line only on a later
            // edit, where Chrome gives it one at once.
            self.mark_atomic_inline_dirty(li_id);
            return true;
        }
        false
    }
}
